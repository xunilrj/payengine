mod workload;

use micromeasure::{BenchContext, BenchmarkRuntimeOptions, Throughput, benchmark_main, black_box};
use payengine::engine::PaymentsEngine;
use payengine::sharded::ShardedPaymentEngine;
use std::time::Duration;
use workload::Workload;

/// Same seed for every sample, so every sample processes the same workload.
const SEED: u64 = 0x5EED;

struct EngineContext<const N: usize> {
    engine: PaymentsEngine,
}

impl<const N: usize> BenchContext for EngineContext<N> {
    fn prepare(_chunk_size: usize) -> Self {
        Self {
            engine: PaymentsEngine::default(),
        }
    }

    fn chunk_size() -> Option<usize> {
        Some(N)
    }
}

struct ShardedContext<const SHARDS: usize, const N: usize> {
    runtime: tokio::runtime::Runtime,
    // Kept here so dropping the engines is not part of the measurement.
    engines: Vec<PaymentsEngine>,
}

impl<const SHARDS: usize, const N: usize> BenchContext for ShardedContext<SHARDS, N> {
    fn prepare(_chunk_size: usize) -> Self {
        Self {
            runtime: tokio::runtime::Builder::new_multi_thread()
                .build()
                .expect("failed to build tokio runtime"),
            engines: Vec::new(),
        }
    }

    fn chunk_size() -> Option<usize> {
        Some(N)
    }
}

fn process<const N: usize>(ctx: &mut EngineContext<N>, chunk_size: usize, _chunk_num: usize) {
    let engine = &mut ctx.engine;
    for transaction in Workload::new(SEED).take(chunk_size) {
        let _ = engine.apply(transaction);
    }
    black_box(engine);
}

fn process_sharded<const SHARDS: usize, const N: usize>(
    ctx: &mut ShardedContext<SHARDS, N>,
    chunk_size: usize,
    _chunk_num: usize,
) {
    let mut sharded = ShardedPaymentEngine::new(SHARDS, ctx.runtime.handle());

    for transaction in Workload::new(SEED).take(chunk_size) {
        sharded.apply(transaction).expect("shard stopped");
    }

    ctx.engines = ctx
        .runtime
        .block_on(sharded.finish())
        .expect("shard panicked");
    black_box(&ctx.engines);
}

benchmark_main!(|runner| {
    runner.set_runtime(BenchmarkRuntimeOptions {
        warm_up_duration: Duration::from_millis(1),
        benchmark_duration: Duration::from_secs(5),
        min_samples: 3,
        max_samples: 5000,
    });

    macro_rules! size {
        ($name:literal, $n:literal) => {
            runner.group::<EngineContext<$n>>("engine", |g| {
                g.throughput(Throughput::per_operation(1, "tx"))
                    .bench($name, process::<$n>);
            });
        };
    }
    size!("10k", 10_000);
    size!("100k", 100_000);
    size!("1M", 1_000_000);
    size!("10M", 10_000_000);
    size!("100M", 100_000_000);

    macro_rules! shards {
        ($name:literal, $shards:literal) => {
            runner.group::<ShardedContext<$shards, 10_000_000>>("sharded", |g| {
                g.throughput(Throughput::per_operation(1, "tx"))
                    .bench($name, process_sharded::<$shards, 10_000_000>);
            });
        };
    }
    shards!("shards-1", 1);
    shards!("shards-2", 2);
    shards!("shards-4", 4);
    shards!("shards-8", 8);
});
