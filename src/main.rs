mod engine;

use csv::{ReaderBuilder, Terminator, Trim, Writer};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::env::args;
use tracing::trace;
use tracing_subscriber::EnvFilter;

use crate::engine::PaymentsEngine;

#[derive(Debug, Deserialize)]
enum TransactionType {
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
struct InputCsv {
    #[serde(rename = "type")]
    ty: TransactionType,
    client: u64,
    tx: u64,
    amount: Option<Decimal>,
}

#[derive(Debug, Serialize)]
struct OutputCsv {
    client: u64,
    available: Decimal,
    held: Decimal,
    total: Decimal,
    locked: bool,
}

#[derive(Default)]
struct Account {
    available: Decimal,
    held: Decimal,
    locked: bool,
}

fn main() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let fmt = tracing_subscriber::fmt().with_env_filter(filter);

    if std::env::var("RUST_LOG_NO_TIMESTAMP").is_ok() {
        fmt.without_time().init();
    } else {
        fmt.init();
    }

    let mut engine = PaymentsEngine::default();

    let input = args().nth(1).unwrap();
    trace!(input = input, "Input file");
    let mut rdr = ReaderBuilder::new()
        .delimiter(b',')
        .has_headers(true)
        .trim(Trim::All)
        .terminator(Terminator::CRLF)
        .from_path(input)
        .unwrap();
    for result in rdr.deserialize::<InputCsv>() {
        let record = result.expect("a CSV record");
        trace!(record = ?record, "Record");

        match record.ty {
            TransactionType::Deposit => {
                engine.deposit(record.tx, record.client, record.amount.unwrap())
            }
            TransactionType::Withdrawal => {
                engine.withdrawal(record.tx, record.client, record.amount.unwrap())
            }
            TransactionType::Dispute => engine.dispute(record.tx, record.client),
            TransactionType::Resolve => engine.resolve(record.tx, record.client),
            TransactionType::Chargeback => engine.chargeback(record.tx, record.client),
        };
    }

    for (id, account) in engine.iter_accounts() {
        let mut wtr = Writer::from_writer(std::io::stdout());
        wtr.serialize(OutputCsv {
            client: *id,
            available: account.available,
            held: account.held,
            total: account.available + account.held,
            locked: account.locked,
        })
        .unwrap();
    }
}
