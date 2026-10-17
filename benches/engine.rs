use micromeasure::{BenchContext, BenchmarkRuntimeOptions, Throughput, benchmark_main, black_box};
use payengine::engine::PaymentsEngine;
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

/// Per 100_000 transactions: 70% deposits, 25% withdrawals, 3% disputes,
/// ~2% resolves and 1 chargeback. Chargebacks lock clients for good, so they
/// are kept rare: even at 100M only ~1.5% of the clients end up locked.
fn process<const N: usize>(ctx: &mut EngineContext<N>, chunk_size: usize, _chunk_num: usize) {
    let engine = &mut ctx.engine;
    let mut rng = Rng(SEED);
    let mut next_tx: u32 = 1;
    let mut last_disputed: Option<u32> = None;

    for _ in 0..chunk_size {
        // Rejections (insufficient funds, not in dispute, ...) are part of a
        // realistic workload, so results are ignored like `main` does.
        let roll = rng.next() % 100_000;

        // Until a tx exists, there is nothing to dispute.
        let _ = if roll < 95_000 || next_tx == 1 {
            let tx = next_tx;
            next_tx += 1;
            if roll < 70_000 {
                engine.deposit(tx, client_of(tx), rng.amount())
            } else {
                engine.withdrawal(tx, client_of(tx), rng.amount())
            }
        } else if roll < 98_000 {
            let tx = rng.existing_tx(next_tx);
            last_disputed = Some(tx);
            engine.dispute(tx, client_of(tx))
        } else {
            // Target the latest dispute so resolve/chargeback take their real
            // path instead of being rejected as "not in dispute".
            let tx = last_disputed
                .take()
                .unwrap_or_else(|| rng.existing_tx(next_tx));
            if roll < 99_999 {
                engine.resolve(tx, client_of(tx))
            } else {
                engine.chargeback(tx, client_of(tx))
            }
        };
    }

    black_box(engine);
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
});
