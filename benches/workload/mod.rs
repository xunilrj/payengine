//! Deterministic, endless stream of transactions shared by the benchmarks and
//! `examples/generate_csv.rs`.

use payengine::engine::Transaction;
use rust_decimal::Decimal;

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
pub struct Workload {
    rng: Rng,
    next_tx: u32,
    last_disputed: Option<u32>,
}

impl Workload {
    /// The same seed always produces the same transactions.
    pub fn new(seed: u64) -> Self {
        Self {
            rng: Rng(seed),
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
