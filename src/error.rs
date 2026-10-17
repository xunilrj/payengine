use ariadne::{Color, Label, Report, ReportKind, Source};
use csv::{ByteRecord, Position};
use rust_decimal::Decimal;
use std::fs::File;
use std::ops::Range;

const TYPE_NAMES: [&str; 5] = ["deposit", "withdrawal", "dispute", "resolve", "chargeback"];

/// Renders malformed-CSV diagnostics with [`ariadne`], pointing at the exact
/// offending field of the real file line. The input file is never slurped: only
/// the single offending line is read back on error (lazily, via a second file
/// handle opened on first use).
pub struct CsvErrors {
    path: String,
    seek_file: Option<File>,
    skipped: usize,
}

impl CsvErrors {
    pub fn new(path: &str) -> Self {
        CsvErrors {
            path: path.to_string(),
            seek_file: None,
            skipped: 0,
        }
    }

    pub fn skipped(&self) -> usize {
        self.skipped
    }

    /// Report and skip one bad record. `field` is the 0-based culprit column
    /// index when known, used to underline the exact token; otherwise the whole
    /// record is underlined.
    pub fn report(&mut self, pos: Option<&Position>, field: Option<usize>, msg: String) {
        self.skipped += 1;
        let (text, pad, mut span) = match pos {
            Some(p) => {
                if self.seek_file.is_none() {
                    self.seek_file = File::open(&self.path).ok();
                }
                let text = match self.seek_file.as_mut() {
                    Some(f) => read_line_at(f, p.byte()).unwrap_or_default(),
                    None => String::new(),
                };
                let char_len = text.chars().count();
                let span = field
                    .and_then(|f| field_token_span(&text, f))
                    .unwrap_or(0..char_len);
                (text, p.line().saturating_sub(1) as usize, span)
            }
            None => (String::new(), 0, 0..0),
        };
        // Pad the source with `pad` empty lines so the error line sits at Source
        // index `pad`. ariadne's gutter is `idx + 1` (it ignores
        // display_line_offset), so this makes the gutter show the real file line
        // `pad + 1`. The empty lines carry no label, so ariadne collapses them
        // into a single blank margin row — only the error line renders. No file
        // I/O: just `pad` generated newlines, never the whole file.
        let src_text = if pad == 0 {
            text
        } else {
            let mut s = String::with_capacity(pad + text.len());
            s.extend(std::iter::repeat('\n').take(pad));
            s.push_str(&text);
            s
        };
        span.start += pad;
        span.end += pad;
        let src = Source::from(src_text);
        let path = self.path.clone();
        let mut builder =
            Report::build(ReportKind::Error, (path.clone(), span.clone())).with_message(msg);
        if pos.is_some() {
            let label_msg = match field {
                Some(f) => field_name(f).to_string(),
                None => "this record".to_string(),
            };
            builder = builder.with_label(
                Label::new((path.clone(), span))
                    .with_color(Color::Red)
                    .with_message(label_msg),
            );
        }
        builder.finish().eprint((path, &src)).ok();
    }
}

/// Index of the first field of `record` that would fail to deserialize, i.e.
/// where serde stopped. Needed because serde's `invalid_value`/`unknown_variant`
/// errors report `field: None`, so the csv error's own field index is missing
/// for the type and amount cases.
pub fn culprit_field(record: &ByteRecord) -> Option<usize> {
    let field = |i| record.get(i).and_then(|b| std::str::from_utf8(b).ok());

    let f0 = field(0)?;
    if !TYPE_NAMES.contains(&f0) {
        return Some(0);
    }
    if field(1).map(|s| s.parse::<u16>().is_err()).unwrap_or(true) {
        return Some(1);
    }
    if field(2).map(|s| s.parse::<u32>().is_err()).unwrap_or(true) {
        return Some(2);
    }
    if let Some(f3) = field(3) {
        if !f3.is_empty()
            && f3
                .parse::<Decimal>()
                .or_else(|_| Decimal::from_scientific(f3))
                .is_err()
        {
            return Some(3);
        }
    }
    None
}

fn read_line_at(file: &mut File, byte_start: u64) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    file.seek(SeekFrom::Start(byte_start)).ok()?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 256];
    loop {
        let n = file.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        if let Some(end) = chunk[..n].iter().position(|&b| b == b'\n') {
            buf.extend_from_slice(&chunk[..end]);
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }
    String::from_utf8(buf).ok()
}

fn field_name(i: usize) -> &'static str {
    match i {
        0 => "type",
        1 => "client",
        2 => "tx",
        _ => "amount",
    }
}

/// Byte-to-char-offset span of the trimmed token in column `field` of `line`.
/// Splits on `,` (the challenge data is unquoted), then trims ASCII whitespace
/// around the cell.
fn field_token_span(line: &str, field: usize) -> Option<Range<usize>> {
    let bytes = line.as_bytes();
    let mut start = 0;
    let mut idx = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b',' {
            if idx == field {
                return Some(token_char_span(line, start, i));
            }
            idx += 1;
            start = i + 1;
        }
        i += 1;
    }
    (idx == field).then(|| token_char_span(line, start, bytes.len()))
}

fn token_char_span(line: &str, byte_start: usize, byte_end: usize) -> Range<usize> {
    let cell = &line.as_bytes()[byte_start..byte_end];
    let mut lo = 0;
    let mut hi = cell.len();
    while lo < hi && cell[lo].is_ascii_whitespace() {
        lo += 1;
    }
    while hi > lo && cell[hi - 1].is_ascii_whitespace() {
        hi -= 1;
    }
    let tb0 = byte_start + lo;
    let tb1 = byte_start + hi;
    line[..tb0].chars().count()..line[..tb1].chars().count()
}
