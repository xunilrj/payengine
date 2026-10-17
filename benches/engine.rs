use micromeasure::{BenchContext, BenchmarkRuntimeOptions, Throughput, benchmark_main, black_box};
use payengine::engine::{PaymentsEngine, Transaction};
use payengine::sharded::ShardedPaymentEngine;
use rust_decimal::Decimal;
use std::time::Duration;

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

/// SplitMix64: tiny and deterministic, good enough to spread clients/amounts.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn amount(&mut self) -> Decimal {
        Decimal::new((self.next() % 1_000_000) as i64, 4)
    }

    fn existing_tx(&mut self, next_tx: u32) -> u32 {
        1 + (self.next() % u64::from(next_tx - 1)) as u32
    }
}

fn client_of(tx: u32) -> u16 {
    (tx.wrapping_mul(2_654_435_761) >> 16) as u16
}

/// Endless stream of transactions. Per 100_000 transactions: 70% deposits,
/// 25% withdrawals, 3% disputes, ~2% resolves and 1 chargeback. Chargebacks
/// lock clients for good, so they are kept rare: even at 100M only ~1.5% of
/// the clients end up locked.
struct Workload {
    rng: Rng,
    next_tx: u32,
    last_disputed: Option<u32>,
}

impl Workload {
    fn new() -> Self {
        Self {
            rng: Rng(SEED),
            next_tx: 1,
            last_disputed: None,
        }
    }
}

impl Iterator for Workload {
    type Item = Transaction;

    fn next(&mut self) -> Option<Transaction> {
        let roll = self.rng.next() % 100_000;

        // Until a tx exists, there is nothing to dispute.
        let transaction = if roll < 95_000 || self.next_tx == 1 {
            let tx = self.next_tx;
            self.next_tx += 1;
            let (client, amount) = (client_of(tx), self.rng.amount());
            if roll < 70_000 {
                Transaction::Deposit { client, tx, amount }
            } else {
                Transaction::Withdrawal { client, tx, amount }
            }
        } else if roll < 98_000 {
            let tx = self.rng.existing_tx(self.next_tx);
            self.last_disputed = Some(tx);
            Transaction::Dispute {
                client: client_of(tx),
                tx,
            }
        } else {
            // Target the latest dispute so resolve/chargeback take their real
            // path instead of being rejected as "not in dispute".
            let tx = self
                .last_disputed
                .take()
                .unwrap_or_else(|| self.rng.existing_tx(self.next_tx));
            let client = client_of(tx);
            if roll < 99_999 {
                Transaction::Resolve { client, tx }
            } else {
                Transaction::Chargeback { client, tx }
            }
        };
        Some(transaction)
    }
}

fn process<const N: usize>(ctx: &mut EngineContext<N>, chunk_size: usize, _chunk_num: usize) {
    let engine = &mut ctx.engine;
    for transaction in Workload::new().take(chunk_size) {
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

    for transaction in Workload::new().take(chunk_size) {
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
