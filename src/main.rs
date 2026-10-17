mod error;
mod input;
mod log;

use crate::error::{RowError, report_all};
use crate::input::Input;
use crate::log::setup_log;
use anyhow::{Context, bail};
use csv::WriterBuilder;
use payengine::engine::PaymentsEngine;
use payengine::ordered_pool::ordered_thread_pool;
use payengine::sharded::{ShardMessage, ShardedPaymentEngine};
use rust_decimal::Decimal;
use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const SHARDS_ENV: &str = "PAYENGINE_SHARDS";
const PARSERS_ENV: &str = "PAYENGINE_PARSERS";
const BLOCK_SIZE_ENV: &str = "PAYENGINE_BLOCK_SIZE";
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

    let input = Arc::new(Input::from_args()?);
    let parsers = quantity_of_parsers()?;
    let block_size = block_size()?;

    // Blocks are read and parsed by `parsers` threads, and received here in
    // file order, at most two per parser ahead of the shards.
    let (mut blocks, parsed_blocks) = ordered_thread_pool("csv-parser", parsers, 2 * parsers, {
        let input = input.clone();
        move |block| input.parse_block(block)
    });
    let producer = std::thread::Builder::new()
        .name("block-producer".to_string())
        .spawn({
            let input = input.clone();
            move || {
                for block in input.blocks(block_size) {
                    if blocks.send(block).is_err() {
                        break;
                    }
                }
            }
        })
        .context("failed to start the block producer")?;

    let mut errors = Vec::new();
    // Block line numbers start at 1; this turns them into file line numbers.
    let mut lines_before = input.first_line() - 1;
    for parsed in parsed_blocks {
        let parsed = parsed.context("failed to read the input file")?;
        for mut raw in parsed.rows {
            raw.line += lines_before;
            sharded.apply(ShardMessage::Raw(raw))?;
        }
        errors.extend(parsed.errors.into_iter().map(|mut error| {
            if let Some(position) = &mut error.position {
                position.set_line(position.line() + lines_before);
            }
            error
        }));
        lines_before += parsed.lines;
    }
    producer
        .join()
        .map_err(|_| anyhow::anyhow!("block producer panicked"))?;

    let finished = runtime
        .block_on(sharded.finish())
        .context("shard task panicked")?;

    errors.extend(finished.errors.into_iter().map(RowError::from));
    let skipped = errors.len();
    report_all(input.path(), errors);
    if skipped > 0 {
        eprintln!("warning: skipped {skipped} invalid input row(s)");
    }

    write_output(&finished.engines)
}

/// A positive integer from the environment, or `None` if it is not set.
fn positive_env(name: &str) -> anyhow::Result<Option<usize>> {
    match std::env::var(name) {
        Ok(value) => match value.parse::<usize>() {
            Ok(n) if n > 0 => Ok(Some(n)),
            _ => bail!("{name} must be a positive integer, got {value:?}"),
        },
        Err(_) => Ok(None),
    }
}

fn cores() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

/// By default half the cores go to shards and half to parsers.
fn half_the_cores() -> usize {
    (cores() / 2).max(1)
}

fn quantity_of_shards() -> anyhow::Result<usize> {
    Ok(positive_env(SHARDS_ENV)?.unwrap_or_else(half_the_cores))
}

fn quantity_of_parsers() -> anyhow::Result<usize> {
    Ok(positive_env(PARSERS_ENV)?.unwrap_or_else(half_the_cores))
}

/// Bytes per parsed block; small values are useful to test block splitting.
fn block_size() -> anyhow::Result<u64> {
    Ok(positive_env(BLOCK_SIZE_ENV)?.unwrap_or(1 << 16) as u64)
}

fn write_output(engines: &[PaymentsEngine]) -> anyhow::Result<()> {
    let mut rows = engines
        .iter()
        .flat_map(|engine| engine.iter_accounts())
        .map(|(id, account)| {
            let total = account
                .available
                .checked_add(account.held)
                .with_context(|| format!("total of client {id} does not fit in a decimal"))?;
            anyhow::Ok(OutputCsv {
                client: *id,
                available: account.available.normalize(),
                held: account.held.normalize(),
                total: total.normalize(),
                locked: account.locked,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
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
