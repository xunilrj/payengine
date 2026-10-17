use csv::{ByteRecord, Position, ReaderBuilder, Terminator};
use std::env::args;
use std::fs::File;
use std::io;
use std::ops::Range;
use tracing::trace;

use crate::error::RowError;
use anyhow::{Context, bail};
use payengine::raw::{RawAmount, RawTransaction, TransactionType};

/// Read size when a block's last line continues past the block's end.
const EXTEND_CHUNK: u64 = 64 * 1024;

/// Column positions, found once from the header row.
struct ColPositions {
    ty: usize,
    client: usize,
    tx: usize,
    amount: Option<usize>,
}

/// A slice of the input after the header, owning every line that starts in
/// `bytes`. Its last line may continue past `bytes.end`.
pub struct LineBlock {
    bytes: Range<u64>,
}

/// The rows of one block. Line numbers (in `rows` and `errors`) are relative
/// to the block, starting at 1: only the consumer, which sees blocks in order,
/// knows how many lines came before.
pub struct ParsedBlock {
    pub rows: Vec<RawTransaction>,
    pub errors: Vec<RowError>,
    pub lines: u64,
}

/// The lines a block owns, as read from the file.
struct OwnedLines {
    buf: Vec<u8>,
    skip: usize,
    offset: u64,
}

impl OwnedLines {
    fn bytes(&self) -> &[u8] {
        &self.buf[self.skip..]
    }
}

pub struct Input {
    path: String,
    file: File,
    len: u64,
    header_end: u64,
    first_line: u64,
    header_len: usize,
    columns: ColPositions,
}

impl Input {
    /// Opens the CSV given as the first CLI argument and reads its header row.
    pub fn from_args() -> anyhow::Result<Self> {
        let mut args = args();
        let path = args
            .nth(1)
            .context("missing input file argument — usage: payengine <input.csv>")?;

        // The spec says the input file is the first and only argument, so
        // anything more is a usage error rather than something to ignore.
        if let Some(extra) = args.next() {
            bail!("unexpected extra argument {extra:?} — usage: payengine <input.csv>");
        }
        trace!(path = &path, "input file opened");

        let file =
            File::open(&path).with_context(|| format!("failed to open input file {path:?}"))?;
        let mut reader = reader_builder().has_headers(true).from_reader(file);
        let headers = reader
            .byte_headers()
            .context("failed to read CSV header row")?
            .clone();
        let header_end = reader.position().byte();
        let file = reader.into_inner();

        let column = |name: &str| {
            headers
                .iter()
                .position(|h| h.trim_ascii() == name.as_bytes())
        };
        let required =
            |name: &str| column(name).with_context(|| format!("header row has no `{name}` column"));

        // An empty file has no header row, and no rows to read either.
        let columns = if headers.is_empty() {
            ColPositions {
                ty: 0,
                client: 1,
                tx: 2,
                amount: Some(3),
            }
        } else {
            ColPositions {
                ty: required("type")?,
                client: required("client")?,
                tx: required("tx")?,
                amount: column("amount"),
            }
        };

        let len = file
            .metadata()
            .context("failed to read input file size")?
            .len();
        let mut header = vec![0; header_end as usize];
        read_exact_at(&file, &mut header, 0).context("failed to read CSV header row")?;
        let first_line = 1 + count_newlines(&header);

        Ok(Input {
            path,
            file,
            len,
            header_end,
            first_line,
            header_len: headers.len(),
            columns,
        })
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    /// The line number of the first line after the header.
    pub fn first_line(&self) -> u64 {
        self.first_line
    }

    /// Splits everything after the header into blocks of `size` bytes.
    pub fn blocks(&self, size: u64) -> impl Iterator<Item = LineBlock> + use<> {
        let (header_end, len) = (self.header_end, self.len);
        (header_end..len)
            .step_by(size as usize)
            .map(move |start| LineBlock {
                bytes: start..(start + size).min(len),
            })
    }

    pub fn parse_block(&self, block: LineBlock) -> io::Result<ParsedBlock> {
        let lines = self.read_lines(&block)?;
        let (bytes, offset) = (lines.bytes(), lines.offset);

        let mut parsed = ParsedBlock {
            rows: Vec::new(),
            errors: Vec::new(),
            lines: count_lines(bytes),
        };
        // Positions from this reader are relative to `bytes`.
        let at = |position: &Position| {
            let mut absolute = position.clone();
            absolute.set_byte(offset + position.byte());
            absolute
        };

        let mut reader = reader_builder().has_headers(false).from_reader(bytes);
        let mut record = ByteRecord::new();
        loop {
            match reader.read_byte_record(&mut record) {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) => {
                    parsed.errors.push(RowError {
                        position: e.position().map(at),
                        field: None,
                        message: e.to_string(),
                    });
                    continue;
                }
            }
            let position = record.position().map(at).unwrap_or_else(Position::new);

            // Deal with more columns than expected
            if record.len() > self.header_len {
                parsed.errors.push(RowError {
                    position: Some(position),
                    field: None,
                    message: format!(
                        "expected at most {} fields, found {}",
                        self.header_len,
                        record.len()
                    ),
                });
                continue;
            }

            match parse_record(&record, &self.columns, position) {
                Ok(raw) => parsed.rows.push(raw),
                Err(error) => parsed.errors.push(error),
            }
        }
        Ok(parsed)
    }

    /// Reads the lines `block` owns: those starting inside it, the last one
    /// finished even past `bytes.end`.
    fn read_lines(&self, block: &LineBlock) -> io::Result<OwnedLines> {
        let mut buf = vec![0; (block.bytes.end - block.bytes.start) as usize];
        read_exact_at(&self.file, &mut buf, block.bytes.start)?;

        // A line that starts before `start` belongs to the previous block.
        let skip = if block.bytes.start == self.header_end {
            0
        } else {
            let mut previous = [0];
            read_exact_at(&self.file, &mut previous, block.bytes.start - 1)?;
            if previous[0] == b'\n' {
                0
            } else {
                match buf.iter().position(|&b| b == b'\n') {
                    Some(newline) => newline + 1,
                    None => buf.len(),
                }
            }
        };

        if skip == buf.len() {
            return Ok(OwnedLines {
                buf: Vec::new(),
                skip: 0,
                offset: block.bytes.end,
            });
        }

        // Finish the last line, which may continue past `bytes.end`.
        let mut position = block.bytes.end;
        while buf.last() != Some(&b'\n') && position < self.len {
            let mut chunk = vec![0; EXTEND_CHUNK.min(self.len - position) as usize];
            read_exact_at(&self.file, &mut chunk, position)?;
            match chunk.iter().position(|&b| b == b'\n') {
                Some(newline) => buf.extend_from_slice(&chunk[..=newline]),
                None => buf.extend_from_slice(&chunk),
            }
            position += chunk.len() as u64;
        }

        Ok(OwnedLines {
            buf,
            skip,
            offset: block.bytes.start + skip as u64,
        })
    }
}

fn reader_builder() -> ReaderBuilder {
    let mut builder = ReaderBuilder::new();
    builder
        .delimiter(b',')
        .terminator(Terminator::CRLF)
        .flexible(true);
    builder
}

fn parse_record(
    record: &ByteRecord,
    columns: &ColPositions,
    position: Position,
) -> Result<RawTransaction, RowError> {
    let error = |field: usize, message: String| RowError {
        position: Some(position.clone()),
        field: Some(field),
        message,
    };
    let field = |column: usize, name: &str| match record.get(column) {
        Some(bytes) => Ok(bytes.trim_ascii()),
        None => Err(error(column, format!("missing `{name}`"))),
    };
    let integer = |column: usize, name: &str, max: u32| {
        let bytes = field(column, name)?;
        if bytes.is_empty() {
            return Err(error(column, format!("missing `{name}`")));
        }
        std::str::from_utf8(bytes)
            .ok()
            .and_then(|text| text.parse::<u32>().ok())
            .filter(|value| *value <= max)
            .ok_or_else(|| {
                let text = String::from_utf8_lossy(bytes);
                error(
                    column,
                    format!("`{name}` must be an integer from 0 to {max}, got `{text}`"),
                )
            })
    };

    let ty_bytes = field(columns.ty, "type")?;
    let ty = TransactionType::parse(ty_bytes).ok_or_else(|| {
        let text = String::from_utf8_lossy(ty_bytes);
        error(
            columns.ty,
            format!(
                "unknown transaction type `{text}`, expected deposit, withdrawal, dispute, resolve or chargeback"
            ),
        )
    })?;
    let client = integer(columns.client, "client", u32::from(u16::MAX))? as u16;
    let tx = integer(columns.tx, "tx", u32::MAX)?;
    let amount = match columns.amount.and_then(|c| record.get(c)) {
        Some(bytes) => RawAmount::new(bytes.trim_ascii()),
        None => RawAmount::Missing,
    };

    Ok(RawTransaction {
        ty,
        client,
        tx,
        amount,
        line: position.line(),
        byte: position.byte(),
        amount_column: columns.amount.unwrap_or(3),
    })
}

fn count_newlines(bytes: &[u8]) -> u64 {
    bytes.iter().filter(|&&b| b == b'\n').count() as u64
}

/// Lines started in `bytes`; the last one may have no trailing newline.
fn count_lines(bytes: &[u8]) -> u64 {
    count_newlines(bytes) + u64::from(bytes.last().is_some_and(|&b| b != b'\n'))
}

#[cfg(unix)]
fn read_exact_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(file, buf, offset)
}

#[cfg(windows)]
fn read_exact_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut done = 0;
    while done < buf.len() {
        match file.seek_read(&mut buf[done..], offset + done as u64)? {
            0 => return Err(io::ErrorKind::UnexpectedEof.into()),
            n => done += n,
        }
    }
    Ok(())
}
