use csv::{ByteRecord, Position, ReaderBuilder, Terminator};
use std::env::args;
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

pub struct Input {
    path: String,
    rdr: csv::Reader<std::fs::File>,
    header_len: usize,
    columns: ColPositions,
    record: ByteRecord,
    errors: Vec<RowError>,
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

        let mut reader = ReaderBuilder::new()
            .delimiter(b',')
            .has_headers(true)
            .terminator(Terminator::CRLF)
            .flexible(true)
            .from_path(&path)
            .with_context(|| format!("failed to open input file {path:?}"))?;
        let headers = reader
            .byte_headers()
            .context("failed to read CSV header row")?;
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
        let header_len = headers.len();

        Ok(Input {
            path,
            rdr: reader,
            header_len,
            columns,
            record: ByteRecord::new(),
            errors: Vec::new(),
        })
    }

    /// The input path and every row rejected by the parser thread.
    pub fn into_parts(self) -> (String, Vec<RowError>) {
        (self.path, self.errors)
    }

    fn parse_record(&self, position: Position) -> Result<RawTransaction, RowError> {
        let error = |field: usize, message: String| RowError {
            position: Some(position.clone()),
            field: Some(field),
            message,
        };
        let field = |column: usize, name: &str| match self.record.get(column) {
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

        let ty_bytes = field(self.columns.ty, "type")?;
        let ty = TransactionType::parse(ty_bytes).ok_or_else(|| {
            let text = String::from_utf8_lossy(ty_bytes);
            error(
                self.columns.ty,
                format!(
                    "unknown transaction type `{text}`, expected deposit, withdrawal, dispute, resolve or chargeback"
                ),
            )
        })?;
        let client = integer(self.columns.client, "client", u32::from(u16::MAX))? as u16;
        let tx = integer(self.columns.tx, "tx", u32::MAX)?;
        let amount = match self.columns.amount.and_then(|c| self.record.get(c)) {
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
            amount_column: self.columns.amount.unwrap_or(3),
        })
    }
}

impl Iterator for Input {
    type Item = RawTransaction;

    fn next(&mut self) -> Option<RawTransaction> {
        loop {
            match self.rdr.read_byte_record(&mut self.record) {
                Ok(true) => {}
                Ok(false) => return None,
                Err(e) => {
                    self.errors.push(RowError {
                        position: e.position().cloned(),
                        field: None,
                        message: e.to_string(),
                    });
                    continue;
                }
            }
            let position = self
                .record
                .position()
                .cloned()
                .unwrap_or_else(Position::new);

            // Deal with more columns than expected
            if self.record.len() > self.header_len {
                self.errors.push(RowError {
                    position: Some(position),
                    field: None,
                    message: format!(
                        "expected at most {} fields, found {}",
                        self.header_len,
                        self.record.len()
                    ),
                });
                continue;
            }

            match self.parse_record(position) {
                Ok(record) => return Some(record),
                Err(error) => self.errors.push(error),
            }
        }
    }
}
