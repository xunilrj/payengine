use crate::engine::{PaymentsEngine, Transaction};
use crate::raw::{RawTransaction, RawTransactionError};
use std::fmt;
use tokio::runtime::Handle;
use tokio::task::{JoinError, JoinHandle};
use tracing::trace;

const BATCH_SIZE: usize = 1024;
const CHANNEL_CAPACITY: usize = 4;

pub enum ShardMessage {
    Transaction(Transaction),
    Raw(RawTransaction),
}

impl ShardMessage {
    fn client(&self) -> u16 {
        match self {
            ShardMessage::Transaction(transaction) => transaction.client(),
            ShardMessage::Raw(raw) => raw.client,
        }
    }
}

/// The result of [`ShardedPaymentEngine::finish`]: the engines in shard order,
/// and every raw transaction that failed to parse.
pub struct Finished {
    pub engines: Vec<PaymentsEngine>,
    pub errors: Vec<RawTransactionError>,
}

/// Will spawn N independent [`PaymentsEngine`]s and route its transactions to the
/// `client % N` shard.
///
/// Transactions are buffered until a batch is full, so [`Self::finish`] must
/// be called to process the rest: dropping it discards them.
pub struct ShardedPaymentEngine {
    senders: Vec<flume::Sender<Vec<ShardMessage>>>,
    batches: Vec<Vec<ShardMessage>>,
    shards: Vec<JoinHandle<(PaymentsEngine, Vec<RawTransactionError>)>>,
}

impl ShardedPaymentEngine {
    /// Spawns `shards` engine tasks on `runtime`. Panics if `shards` is 0.
    pub fn new(shards: usize, runtime: &Handle) -> Self {
        assert!(shards > 0, "at least one shard is required");

        let batches = (0..shards)
            .map(|_| Vec::with_capacity(BATCH_SIZE))
            .collect();
        let (senders, shards) = (0..shards)
            .map(|_| {
                let (sender, receiver) = flume::bounded(CHANNEL_CAPACITY);
                (sender, runtime.spawn(run_shard(receiver)))
            })
            .unzip();

        Self {
            senders,
            batches,
            shards,
        }
    }

    pub fn apply(&mut self, message: ShardMessage) -> Result<(), ShardStopped> {
        let shard = usize::from(message.client()) % self.senders.len();
        self.batches[shard].push(message);
        if self.batches[shard].len() == BATCH_SIZE {
            self.flush(shard)?;
        }
        Ok(())
    }

    fn flush(&mut self, shard: usize) -> Result<(), ShardStopped> {
        let batch = std::mem::replace(&mut self.batches[shard], Vec::with_capacity(BATCH_SIZE));
        self.senders[shard].send(batch).map_err(|_| ShardStopped)
    }

    pub async fn finish(mut self) -> Result<Finished, JoinError> {
        for shard in 0..self.senders.len() {
            if !self.batches[shard].is_empty() {
                let _ = self.flush(shard);
            }
        }

        let Self {
            senders, shards, ..
        } = self;

        // Dropping the senders closes the channels, which ends the shards.
        drop(senders);

        let mut finished = Finished {
            engines: Vec::with_capacity(shards.len()),
            errors: Vec::new(),
        };
        for shard in shards {
            let (engine, errors) = shard.await?;
            finished.engines.push(engine);
            finished.errors.extend(errors);
        }

        Ok(finished)
    }
}

async fn run_shard(
    receiver: flume::Receiver<Vec<ShardMessage>>,
) -> (PaymentsEngine, Vec<RawTransactionError>) {
    let mut engine = PaymentsEngine::default();
    let mut errors = Vec::new();

    while let Ok(batch) = receiver.recv_async().await {
        for message in batch {
            let transaction = match message {
                ShardMessage::Transaction(transaction) => transaction,
                ShardMessage::Raw(raw) => match raw.into_transaction() {
                    Ok(transaction) => transaction,
                    Err(error) => {
                        errors.push(error);
                        continue;
                    }
                },
            };
            trace!(?transaction, "transaction parsed");
            let _ = engine.apply(transaction);
        }
    }

    (engine, errors)
}

#[derive(Debug)]
pub struct ShardStopped;

impl fmt::Display for ShardStopped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("engine shard stopped before the input was fully processed")
    }
}

impl std::error::Error for ShardStopped {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "at least one shard is required")]
    fn zero_shards_is_rejected() {
        let runtime = tokio::runtime::Builder::new_multi_thread().build().unwrap();
        ShardedPaymentEngine::new(0, runtime.handle());
    }
}
