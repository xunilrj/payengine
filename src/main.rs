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
        let output = OutputCsv {
            client: *id,
            available: account.available.normalize(),
            held: account.held.normalize(),
            total: (account.available + account.held).normalize(),
            locked: account.locked,
        };
        writer
            .serialize(output)
            .context("failed to write output row")?;
    }
    writer.flush().context("failed to flush output")?;
    Ok(())
}

fn process_input(engine: &mut PaymentsEngine) -> anyhow::Result<()> {
    let mut input = Input::from_args()?;

    for transaction in &mut input {
        let _ = match transaction.ty {
            TransactionType::Deposit => engine.deposit(
                transaction.tx,
                transaction.client,
                // SAFETY: Validation guarantees amount is Some
                transaction
                    .amount
                    .expect("unexpected amount = None on deposit"),
            ),
            TransactionType::Withdrawal => engine.withdrawal(
                transaction.tx,
                transaction.client,
                // SAFETY: Validation guarantees amount is Some
                transaction
                    .amount
                    .expect("unexpected amount = None on withdrawal"),
            ),
            TransactionType::Dispute => engine.dispute(transaction.tx, transaction.client),
            TransactionType::Resolve => engine.resolve(transaction.tx, transaction.client),
            TransactionType::Chargeback => engine.chargeback(transaction.tx, transaction.client),
        };
    }

    let skipped = input.skipped();
    if skipped > 0 {
        eprintln!("warning: skipped {skipped} invalid input row(s)");
    }

    Ok(())
}
