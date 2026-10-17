use csv::{ByteRecord, ErrorKind, ReaderBuilder, Terminator, Trim};
use rust_decimal::Decimal;
use serde::Deserialize;
use std::env::args;
use tracing::trace;

use crate::error::{CsvErrors, culprit_field};
use anyhow::Context;

#[derive(Debug, Deserialize)]
pub enum TransactionType {
    #[serde(rename = "deposit")]
    Deposit,
    #[serde(rename = "withdrawal")]
    Withdrawal,
    #[serde(rename = "dispute")]
    Dispute,
    #[serde(rename = "resolve")]
    Resolve,
    #[serde(rename = "chargeback")]
    Chargeback,
}

#[derive(Debug, Deserialize)]
pub struct InputCsv {
    #[serde(rename = "type")]
    pub ty: TransactionType,
    pub client: u16,
    pub tx: u32,
    pub amount: Option<Decimal>,
}

struct FieldError {
    field: usize,
    msg: String,
}

impl InputCsv {
    /// Extra validation for the deserialized transaction. Done here
    /// to enable more user friendly error on the terminal
    fn validate(&self) -> Result<(), FieldError> {
        let name = match self.ty {
            TransactionType::Deposit => "deposit",
            TransactionType::Withdrawal => "withdrawal",
            _ => return Ok(()),
        };
        match self.amount {
            None => Err(FieldError {
                field: 3,
                msg: format!("{name} requires an `amount`"),
            }),
            Some(a) if a.is_sign_negative() => Err(FieldError {
                field: 3,
                msg: format!("{name} `amount` must not be negative"),
            }),
            Some(_) => Ok(()),
        }
    }
}

pub struct Input {
    rdr: csv::Reader<std::fs::File>,
    headers: ByteRecord,
    errs: CsvErrors,
}

impl Input {
    /// Opens the CSV given as the first CLI argument and reads its header row.
    pub fn from_args() -> anyhow::Result<Self> {
        let path = args()
            .nth(1)
            .context("missing input file argument — usage: payengine <input.csv>")?;
        trace!(target: "payengine", input = &path, "Input file");

        let mut reader = ReaderBuilder::new()
            .delimiter(b',')
            .has_headers(true)
            .trim(Trim::All)
            .terminator(Terminator::CRLF)
            .from_path(&path)
            .with_context(|| format!("failed to open input file {path:?}"))?;
        let headers = reader
            .byte_headers()
            .context("failed to read CSV header row")?
            .clone();
        Ok(Input {
            rdr: reader,
            headers,
            errs: CsvErrors::new(&path),
        })
    }
}

impl Iterator for Input {
    type Item = InputCsv;

    fn next(&mut self) -> Option<InputCsv> {
        loop {
            let record = match self.rdr.byte_records().next() {
                Some(Ok(r)) => r,
                Some(Err(e)) => {
                    self.errs.report(e.position(), None, e.to_string());
                    continue;
                }
                None => return None,
            };
            let pos = record.position().cloned();
            let rec = match record.deserialize::<InputCsv>(Some(&self.headers)) {
                Ok(r) => r,
                Err(e) => {
                    let msg = match e.kind() {
                        ErrorKind::Deserialize { err, .. } => err.to_string(),
                        _ => e.to_string(),
                    };
                    let field = culprit_field(&record);
                    self.errs.report(pos.as_ref(), field, msg);
                    continue;
                }
            };
            trace!(target: "payengine", record = ?rec, "Record");

            if let Err(err) = rec.validate() {
                self.errs.report(pos.as_ref(), Some(err.field), err.msg);
                continue;
            }
            return Some(rec);
        }
    }
}
