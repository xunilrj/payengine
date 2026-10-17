mod error;
mod input;
mod log;

use crate::error::{RowError, report_all};
use crate::input::Input;
use crate::log::setup_log;
use anyhow::{Context, bail};
use csv::WriterBuilder;
use payengine::engine::PaymentsEngine;
use payengine::sharded::{ShardMessage, ShardedPaymentEngine};
use rust_decimal::Decimal;
use serde::Serialize;
use std::sync::atomic::{AtomicUsize, Ordering};

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

fn main() -> std::process::ExitCode {
    setup_log();

    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<()> {
    let shards = quantity_of_shards()?;

    // One worker per shard, named so profilers and debuggers can tell them
    // apart from the parser.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(shards)
        .thread_name_fn(|| {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            format!("shard-worker-{}", NEXT.fetch_add(1, Ordering::Relaxed))
        })
        .build()
        .context("failed to start the runtime")?;
    let mut sharded = ShardedPaymentEngine::new(shards, runtime.handle());

    // There is no way to name a blocking thread in tokio, so we create one
    // here manually.
    let (sharded, path, mut errors) = std::thread::Builder::new()
        .name("csv-parser".to_string())
        .spawn(move || {
            let mut input = Input::from_args()?;
            for record in &mut input {
                sharded.apply(ShardMessage::Raw(record))?;
            }
            let (path, errors) = input.into_parts();
            anyhow::Ok((sharded, path, errors))
        })
        .context("failed to start the parser thread")?
        .join()
        .map_err(|_| anyhow::anyhow!("parser thread panicked"))??;

    let finished = runtime
        .block_on(sharded.finish())
        .context("shard task panicked")?;

    errors.extend(finished.errors.into_iter().map(RowError::from));
    let skipped = errors.len();
    report_all(&path, errors);
    if skipped > 0 {
        eprintln!("warning: skipped {skipped} invalid input row(s)");
    }

    write_output(&finished.engines)
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
