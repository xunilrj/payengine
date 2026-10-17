use csv::{ByteRecord, ErrorKind, ReaderBuilder, Terminator, Trim};
use rust_decimal::Decimal;
use serde::Deserialize;
use std::env::args;
use tracing::trace;

use crate::error::{CsvErrors, culprit_field};
use anyhow::{Context, bail};
use payengine::engine::Transaction;

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
    fn into_transaction(self) -> Result<Transaction, FieldError> {
        let (client, tx) = (self.client, self.tx);
        Ok(match self.ty {
            TransactionType::Deposit => Transaction::Deposit {
                client,
                tx,
                amount: required_amount(self.amount, "deposit")?,
            },
            TransactionType::Withdrawal => Transaction::Withdrawal {
                client,
                tx,
                amount: required_amount(self.amount, "withdrawal")?,
            },
            TransactionType::Dispute => Transaction::Dispute { client, tx },
            TransactionType::Resolve => Transaction::Resolve { client, tx },
            TransactionType::Chargeback => Transaction::Chargeback { client, tx },
        })
    }
}

fn required_amount(amount: Option<Decimal>, name: &str) -> Result<Decimal, FieldError> {
    match amount {
        None => Err(FieldError {
            field: 3,
            msg: format!("{name} requires an `amount`"),
        }),
        Some(a) if a.is_sign_negative() => Err(FieldError {
            field: 3,
            msg: format!("{name} `amount` must not be negative"),
        }),
        Some(a) => Ok(a),
    }
}

pub struct Input {
    rdr: csv::Reader<std::fs::File>,
    headers: ByteRecord,
    record: ByteRecord,
    errs: CsvErrors,
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
            .trim(Trim::All)
            .terminator(Terminator::CRLF)
            .flexible(true)
            .from_path(&path)
            .with_context(|| format!("failed to open input file {path:?}"))?;
        let headers = reader
            .byte_headers()
            .context("failed to read CSV header row")?
            .clone();
        Ok(Input {
            rdr: reader,
            headers,
            record: ByteRecord::new(),
            errs: CsvErrors::new(&path),
        })
    }

    pub fn skipped(&self) -> usize {
        self.errs.skipped()
    }
}

impl Iterator for Input {
    type Item = Transaction;

    fn next(&mut self) -> Option<Transaction> {
        loop {
            let record = match self.rdr.read_byte_record(&mut self.record) {
                Ok(true) => &self.record,
                Ok(false) => return None,
                Err(e) => {
                    self.errs.report(e.position(), None, e.to_string());
                    continue;
                }
            };
            let pos = record.position().cloned();

            // Deal with more columns than expected
            if record.len() > self.headers.len() {
                let msg = format!(
                    "expected at most {} fields, found {}",
                    self.headers.len(),
                    record.len()
                );
                self.errs.report(pos.as_ref(), None, msg);
                continue;
            }

            let rec = match record.deserialize::<InputCsv>(Some(&self.headers)) {
                Ok(r) => r,
                Err(e) => {
                    let msg = match e.kind() {
                        ErrorKind::Deserialize { err, .. } => err.to_string(),
                        _ => e.to_string(),
                    };
                    let field = culprit_field(record);
                    self.errs.report(pos.as_ref(), field, msg);
                    continue;
                }
            };
            trace!(record = ?rec, "record parsed");

            match rec.into_transaction() {
                Ok(transaction) => return Some(transaction),
                Err(err) => self.errs.report(pos.as_ref(), Some(err.field), err.msg),
            }
        }
    }
}
