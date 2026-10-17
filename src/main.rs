mod engine;
mod error;
mod input;
mod log;

use crate::engine::PaymentsEngine;
use crate::input::{Input, TransactionType};
use crate::log::setup_log;
use anyhow::Context;
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

fn main() -> std::process::ExitCode {
    setup_log();

    let mut engine = PaymentsEngine::default();

    match run(&mut engine) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run(engine: &mut PaymentsEngine) -> anyhow::Result<()> {
    process_input(engine)?;
    write_output(engine)?;
    Ok(())
}

fn write_output(engine: &mut PaymentsEngine) -> anyhow::Result<()> {
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
            .context("failed to write output row")?;
    }
    writer.flush().context("failed to flush output")?;
    Ok(())
}

fn process_input(engine: &mut PaymentsEngine) -> anyhow::Result<()> {
    let input = Input::from_args()?;
    for transaction in input {
        let _ = match transaction.ty {
            TransactionType::Deposit => engine.deposit(
                transaction.tx,
                transaction.client,
                transaction.amount.expect("validate guarantees amount present for deposit"),
            ),
            TransactionType::Withdrawal => engine.withdrawal(
                transaction.tx,
                transaction.client,
                transaction.amount.expect("validate guarantees amount present for withdrawal"),
            ),
            TransactionType::Dispute => engine.dispute(transaction.tx, transaction.client),
            TransactionType::Resolve => engine.resolve(transaction.tx, transaction.client),
            TransactionType::Chargeback => engine.chargeback(transaction.tx, transaction.client),
        };
    }
    Ok(())
}
