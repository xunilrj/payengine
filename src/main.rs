mod engine;

use csv::{ReaderBuilder, Terminator, Trim, Writer};
use serde::{Deserialize, Serialize};
use std::env::args;
use tracing::trace;
use tracing_subscriber::EnvFilter;

use crate::engine::PaymentsEngine;

#[derive(Debug, Deserialize)]
enum TransactionType {
    #[serde(rename = "deposit")]
    Deposit,
}

#[derive(Debug, Deserialize)]
struct InputCsv {
    #[serde(rename = "type")]
    ty: TransactionType,
    client: u64,
    tx: u64,
    amount: f32,
}

#[derive(Debug, Serialize)]
struct OutputCsv {
    client: u64,
    available: f32,
    held: f32,
    total: f32,
    locked: bool,
}

#[derive(Default)]
struct Account {
    available: f32,
    held: f32,
    locked: bool,
}

fn main() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

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
            TransactionType::Deposit => engine.deposit(record.client, record.amount),
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
