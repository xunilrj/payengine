use csv::{ByteRecord, ErrorKind, ReaderBuilder, Terminator, Trim};
use rust_decimal::Decimal;
use serde::Deserialize;
use std::env::args;
use tracing::trace;

use crate::error::{CsvErrors, culprit_field};

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
    pub client: u64,
    pub tx: u64,
    pub amount: Option<Decimal>,
}

/// A semantic validation failure on an otherwise well-formed record: the 0-based
/// culprit column and a human-readable message. serde cannot express these
/// checks (e.g. a `deposit` with no `amount` deserializes fine as `None`), so
/// [`InputCsv::validate`] performs them after deserialization.
struct FieldError {
    field: usize,
    msg: String,
}

impl InputCsv {
    /// Checks that cannot be expressed in the serde schema — currently that a
    /// `deposit` or `withdrawal` carries an `amount`. `dispute`/`resolve`/
    /// `chargeback` never need one. On success the caller may safely `unwrap`
    /// `amount` for the variants that require it.
    fn validate(&self) -> Result<(), FieldError> {
        let needs_amount = matches!(
            self.ty,
            TransactionType::Deposit | TransactionType::Withdrawal
        );
        if needs_amount && self.amount.is_none() {
            let name = match self.ty {
                TransactionType::Deposit => "deposit",
                TransactionType::Withdrawal => "withdrawal",
                _ => unreachable!("guarded by needs_amount"),
            };
            return Err(FieldError {
                field: 3,
                msg: format!("{name} requires an `amount`"),
            });
        }
        Ok(())
    }
}

/// Lazy streaming reader over the input CSV. Iterating yields only records that
/// deserialized **and** passed [`InputCsv::validate`]; malformed rows are
/// reported via [`CsvErrors`] (ariadne diagnostic on stderr) and skipped, so a
/// run never aborts on a single bad row. The file is streamed record-by-record
/// and, on error, only the single offending line is read back — never slurped.
pub struct Input {
    rdr: csv::Reader<std::fs::File>,
    headers: ByteRecord,
    errs: CsvErrors,
}

impl Input {
    /// Opens the CSV given as the first CLI argument and reads its header row.
    pub fn from_args() -> Self {
        let path = args().nth(1).unwrap();
        // Pinned to the crate target so log output stays stable regardless of
        // which module emits it.
        trace!(target: "payengine", input = &path, "Input file");
        let mut rdr = ReaderBuilder::new()
            .delimiter(b',')
            .has_headers(true)
            .trim(Trim::All)
            .terminator(Terminator::CRLF)
            .from_path(&path)
            .unwrap();
        let headers = rdr.byte_headers().unwrap().clone();
        Input {
            rdr,
            headers,
            errs: CsvErrors::new(&path),
        }
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