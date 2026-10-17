mod error;
mod input;
mod log;

use crate::input::Input;
use crate::log::setup_log;
use anyhow::{Context, bail};
use csv::WriterBuilder;
use payengine::engine::PaymentsEngine;
use payengine::sharded::ShardedPaymentEngine;
use rust_decimal::Decimal;
use serde::Serialize;
use tokio::runtime::Handle;

const SHARDS_ENV: &str = "PAYENGINE_SHARDS";
const SORT_OUTPUT_ENV: &str = "PAYENGINE_SORT_OUTPUT";

#[derive(Debug, Serialize)]
struct OutputCsv {
    client: u16,
    available: Decimal,
    held: Decimal,
    total: Decimal,
    locked: bool,
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    setup_log();

    match run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn run() -> anyhow::Result<()> {
    let shards = quantity_of_shards()?;
    let mut sharded = ShardedPaymentEngine::new(shards, &Handle::current());

    let (sharded, skipped) = tokio::task::spawn_blocking(move || {
        let mut input = Input::from_args()?;
        for transaction in &mut input {
            sharded.apply(transaction)?;
        }
        anyhow::Ok((sharded, input.skipped()))
    })
    .await
    .context("parser task panicked")??;

    let engines = sharded.finish().await.context("shard task panicked")?;

    if skipped > 0 {
        eprintln!("warning: skipped {skipped} invalid input row(s)");
    }

    write_output(&engines)
}

fn quantity_of_shards() -> anyhow::Result<usize> {
    match std::env::var(SHARDS_ENV) {
        Ok(value) => match value.parse::<usize>() {
            Ok(shards) if shards > 0 => Ok(shards),
            _ => bail!("{SHARDS_ENV} must be a positive integer, got {value:?}"),
        },
        Err(_) => {
            // By default leave one thread to the CSV Parser
            let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
            let shards = cores.checked_sub(1).unwrap_or(1).max(1);
            Ok(shards)
        }
    }
}

fn write_output(engines: &[PaymentsEngine]) -> anyhow::Result<()> {
    let mut rows = engines
        .iter()
        .flat_map(|engine| engine.iter_accounts())
        .map(|(id, account)| OutputCsv {
            client: *id,
            available: account.available.normalize(),
            held: account.held.normalize(),
            total: (account.available + account.held).normalize(),
            locked: account.locked,
        })
        .collect::<Vec<_>>();
    if std::env::var_os(SORT_OUTPUT_ENV).is_some() {
        rows.sort_unstable_by_key(|row| row.client);
    }

    let mut writer = WriterBuilder::new()
        .has_headers(false)
        .from_writer(std::io::stdout());
    writer
        .write_record(["client", "available", "held", "total", "locked"])
        .context("failed to write output header")?;
    for row in rows {
        writer
            .serialize(row)
            .context("failed to write output row")?;
    }
    writer.flush().context("failed to flush output")?;
    Ok(())
}
