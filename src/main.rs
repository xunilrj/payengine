mod engine;
mod error;
mod input;
mod log;

use crate::engine::PaymentsEngine;
use crate::input::{Input, TransactionType};
use crate::log::setup_log;
use csv::Writer;
use rust_decimal::Decimal;
use serde::Serialize;

#[derive(Debug, Serialize)]
struct OutputCsv {
    client: u16,
    available: Decimal,
    held: Decimal,
    total: Decimal,
    locked: bool,
}

fn main() {
    setup_log();

    let mut engine = PaymentsEngine::default();

    process_input(&mut engine);
    write_output(&mut engine);
}

fn write_output(engine: &mut PaymentsEngine) {
    let mut writer = Writer::from_writer(std::io::stdout());
    for (id, account) in engine.iter_accounts() {
        writer
            .serialize(OutputCsv {
                client: *id,
                available: account.available,
                held: account.held,
                total: account.available + account.held,
                locked: account.locked,
            })
            .unwrap();
    }
}

fn process_input(engine: &mut PaymentsEngine) {
    let input = Input::from_args();
    for transaction in input {
        let _ = match transaction.ty {
            TransactionType::Deposit => engine.deposit(
                transaction.tx,
                transaction.client,
                transaction.amount.unwrap(),
            ),
            TransactionType::Withdrawal => engine.withdrawal(
                transaction.tx,
                transaction.client,
                transaction.amount.unwrap(),
            ),
            TransactionType::Dispute => engine.dispute(transaction.tx, transaction.client),
            TransactionType::Resolve => engine.resolve(transaction.tx, transaction.client),
            TransactionType::Chargeback => engine.chargeback(transaction.tx, transaction.client),
        };
    }
}
