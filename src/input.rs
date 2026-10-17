use csv::{ByteRecord, Position, ReaderBuilder, Terminator};
use std::env::args;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use tracing::trace;

use crate::error::RowError;
use anyhow::{Context, bail};
use payengine::raw::{RawAmount, RawTransaction, TransactionType};

/// Column positions, found once from the header row.
struct ColPositions {
    ty: usize,
    client: usize,
    tx: usize,
    amount: Option<usize>,
}

/// Records of the input after the header. It starts and ends on a record
/// boundary, so it can be parsed on its own.
pub struct LineBlock {
    bytes: Vec<u8>,
    /// File offset of `bytes[0]`.
    offset: u64,
}

/// Reads the input after the header sequentially and cuts it into blocks of at
/// least `size` bytes, each ending at a newline outside quotes. Tracking quote
/// parity here keeps a quoted field containing a newline inside one block.
pub struct LineBlocks {
    file: File,
    size: usize,
    /// File offset of `pending[0]`.
    offset: u64,
    /// Read but not yet handed out.
    pending: Vec<u8>,
    /// `pending[..scanned]` is accounted for in `in_quotes`.
    scanned: usize,
    in_quotes: bool,
    eof: bool,
}

impl Iterator for LineBlocks {
    type Item = io::Result<LineBlock>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_block().transpose()
    }
}

impl LineBlocks {
    fn next_block(&mut self) -> io::Result<Option<LineBlock>> {
        let cut = loop {
            if let Some(cut) = self.find_cut() {
                break cut;
            }
            if self.eof {
                break self.pending.len();
            }
            self.fill()?;
        };

        debug_assert!(
            !self.in_quotes || self.eof,
            "a block can end inside quotes only at EOF"
        );

        if cut == 0 {
            return Ok(None);
        }

        let rest = self.pending.split_off(cut);
        let block = LineBlock {
            bytes: std::mem::replace(&mut self.pending, rest),
            offset: self.offset,
        };
        // Blocks are only cut outside quotes.
        self.offset += cut as u64;
        self.scanned = 0;
        self.in_quotes = false;
        Ok(Some(block))
    }

    /// Index just past the first newline outside quotes that makes the block
    /// at least `size` bytes long, scanning only bytes not scanned before.
    fn find_cut(&mut self) -> Option<usize> {
        // Quotes before the earliest possible cut only flip the parity.
        let earliest = (self.size - 1).min(self.pending.len());
        if self.scanned < earliest {
            let quotes = memchr::memchr_iter(b'"', &self.pending[self.scanned..earliest]).count();
            self.in_quotes ^= quotes % 2 == 1;
            self.scanned = earliest;
        }
        while let Some(i) = memchr::memchr2(b'"', b'\n', &self.pending[self.scanned..]) {
            let i = self.scanned + i;
            self.scanned = i + 1;
            if self.pending[i] == b'"' {
                self.in_quotes = !self.in_quotes;
            } else if !self.in_quotes {
                return Some(i + 1);
            }
        }
        None
    }

    fn fill(&mut self) -> io::Result<()> {
        let read = (&mut self.file)
            .take(self.size as u64)
            .read_to_end(&mut self.pending)?;
        self.eof = read == 0;
        Ok(())
    }
}

/// The rows of one block. Line numbers (in `rows` and `errors`) are relative
/// to the block, starting at 1: only the consumer, which sees blocks in order,
/// knows how many lines came before.
pub struct ParsedBlock {
    pub rows: Vec<RawTransaction>,
    pub errors: Vec<RowError>,
    pub lines: u64,
}

pub struct Input {
    path: String,
    file: File,
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
        let mut file = reader.into_inner();

        let column = |name: &str| {
            headers
                .iter()
                .position(|h| h.trim_ascii().eq_ignore_ascii_case(name.as_bytes()))
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

        let mut header = vec![0; header_end as usize];
        file.seek(SeekFrom::Start(0))
            .and_then(|_| file.read_exact(&mut header))
            .context("failed to read CSV header row")?;
        let first_line = 1 + count_newlines(&header);

        Ok(Input {
            path,
            file,
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

    /// Everything after the header, in blocks of at least `size` bytes.
    pub fn blocks(&self, size: u64) -> io::Result<LineBlocks> {
        let mut file = self.file.try_clone()?;
        file.seek(SeekFrom::Start(self.header_end))?;
        Ok(LineBlocks {
            file,
            size: size as usize,
            offset: self.header_end,
            pending: Vec::new(),
            scanned: 0,
            in_quotes: false,
            eof: false,
        })
    }

    pub fn parse_block(&self, block: LineBlock) -> ParsedBlock {
        let (bytes, offset) = (block.bytes.as_slice(), block.offset);

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
            // A line of only whitespace is skipped, like an empty line.
            if record.len() == 1 && record[0].trim_ascii().is_empty() {
                continue;
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
        parsed
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
