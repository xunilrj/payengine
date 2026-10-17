//! A pool of worker threads fed through a channel: jobs are sent to the pool,
//! and their results are received in the order the jobs were sent.
//!
//! At most `in_flight` jobs are outstanding (queued, running, or finished but
//! not yet received): [`OrderedSender::send`] blocks until the receiver
//! catches up, so memory stays bounded however much input there is.
//!
//! The sender and the receiver must be used from different threads (or
//! interleaved): sending more than `in_flight` jobs without receiving
//! deadlocks.

use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Arc;
use std::thread;

/// Spawns `workers` threads named `{name}-{i}` that run `f` on every job sent
/// to the returned sender. Panics if `workers` or `in_flight` is 0.
pub fn ordered_thread_pool<T, R, F>(
    name: &str,
    workers: usize,
    in_flight: usize,
    f: F,
) -> (OrderedSender<T>, OrderedReceiver<R>)
where
    T: Send + 'static,
    R: Send + 'static,
    F: Fn(T) -> R + Send + Sync + 'static,
{
    assert!(workers > 0, "at least one worker is required");
    assert!(in_flight > 0, "in_flight must be at least 1");

    let (work_tx, work_rx) = flume::bounded::<(usize, T)>(in_flight);
    let (result_tx, result_rx) = flume::bounded::<(usize, thread::Result<R>)>(in_flight);
    let (permit_tx, permit_rx) = flume::bounded::<()>(in_flight);
    for _ in 0..in_flight {
        let _ = permit_tx.send(());
    }

    let f = Arc::new(f);
    for i in 0..workers {
        let (work_rx, result_tx, f) = (work_rx.clone(), result_tx.clone(), f.clone());
        thread::Builder::new()
            .name(format!("{name}-{i}"))
            .spawn(move || {
                // Ends once the sender is dropped and the queue is empty.
                for (seq, job) in work_rx {
                    let result = catch_unwind(AssertUnwindSafe(|| f(job)));
                    if result_tx.send((seq, result)).is_err() {
                        break; // the receiver is gone
                    }
                }
            })
            .expect("failed to spawn worker thread");
    }

    let sender = OrderedSender {
        work: work_tx,
        permits: permit_rx,
        seq: 0,
    };
    let receiver = OrderedReceiver {
        results: result_rx,
        permits: permit_tx,
        ring: (0..in_flight).map(|_| None).collect(),
        next: 0,
    };
    (sender, receiver)
}

/// Sends jobs. Not `Clone`: with a single producer, "send order" is well
/// defined. Dropping it lets the workers finish and the receiver end.
pub struct OrderedSender<T> {
    work: flume::Sender<(usize, T)>,
    permits: flume::Receiver<()>,
    seq: usize,
}

impl<T> OrderedSender<T> {
    /// Blocks while `in_flight` jobs are outstanding. Fails if the receiver
    /// was dropped.
    pub fn send(&mut self, job: T) -> Result<(), Disconnected> {
        // Permits may still be queued after the receiver is dropped.
        if self.permits.is_disconnected() {
            return Err(Disconnected);
        }
        self.permits.recv().map_err(|_| Disconnected)?;
        // Never blocks: holding a permit means there is room in the queue.
        self.work.send((self.seq, job)).map_err(|_| Disconnected)?;
        self.seq += 1;
        Ok(())
    }
}

/// Receives results in send order, as an iterator. It ends once the sender is
/// dropped and every result has been received. A panic in a job is resumed
/// here, at that job's position.
pub struct OrderedReceiver<R> {
    results: flume::Receiver<(usize, thread::Result<R>)>,
    permits: flume::Sender<()>,
    /// Results that arrived before their turn, at `seq % in_flight`. Never
    /// more than `in_flight` are outstanding, so slots cannot collide.
    ring: Vec<Option<thread::Result<R>>>,
    next: usize,
}

impl<R> Iterator for OrderedReceiver<R> {
    type Item = R;

    fn next(&mut self) -> Option<R> {
        let len = self.ring.len();
        let slot = self.next % len;
        while self.ring[slot].is_none() {
            // Closed only when every worker is done: nothing is pending.
            let (seq, result) = self.results.recv().ok()?;
            debug_assert!(
                self.ring[seq % len].is_none(),
                "two results landed in the same ring slot; ordering would be lost"
            );
            self.ring[seq % len] = Some(result);
        }
        self.next += 1;
        // Never blocks: at most `in_flight` permits exist.
        let _ = self.permits.send(());
        let result = self.ring[slot].take().expect("slot was just filled");
        Some(result.unwrap_or_else(|panic| resume_unwind(panic)))
    }
}

/// The other side of the channel was dropped.
#[derive(Debug, PartialEq, Eq)]
pub struct Disconnected;

impl fmt::Display for Disconnected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the other side of the ordered thread pool was dropped")
    }
}

impl std::error::Error for Disconnected {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// Sends `jobs` from another thread and collects every result.
    fn run<R: Send + 'static>(
        workers: usize,
        in_flight: usize,
        jobs: Vec<u64>,
        f: impl Fn(u64) -> R + Send + Sync + 'static,
    ) -> Vec<R> {
        let (mut sender, receiver) = ordered_thread_pool("test", workers, in_flight, f);
        let producer = thread::spawn(move || {
            for job in jobs {
                sender.send(job).unwrap();
            }
        });
        let results = receiver.collect();
        producer.join().unwrap();
        results
    }

    #[test]
    fn results_come_out_in_send_order() {
        // Later jobs often finish first: the sleep shrinks as the job grows.
        let jobs: Vec<u64> = (0..200).collect();
        let results = run(8, 16, jobs.clone(), |job| {
            thread::sleep(Duration::from_micros((200 - job) * 20));
            job * 2
        });
        assert_eq!(results, jobs.iter().map(|job| job * 2).collect::<Vec<_>>());
    }

    #[test]
    fn never_more_than_in_flight_outstanding() {
        let outstanding = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let (mut sender, receiver) = {
            let (outstanding, peak) = (outstanding.clone(), peak.clone());
            ordered_thread_pool("test", 8, 4, move |job: u64| {
                let now = outstanding.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                thread::sleep(Duration::from_micros(200));
                job
            })
        };
        let producer = thread::spawn(move || {
            for job in 0..300 {
                sender.send(job).unwrap();
            }
        });
        for _ in receiver {
            // A job stops being outstanding once its result is received.
            outstanding.fetch_sub(1, Ordering::SeqCst);
        }
        producer.join().unwrap();
        assert!(peak.load(Ordering::SeqCst) <= 4, "peak = {peak:?}");
    }

    #[test]
    fn in_flight_one_is_sequential() {
        let results = run(4, 1, (0..50).collect(), |job| job + 1);
        assert_eq!(results, (1..51).collect::<Vec<_>>());
    }

    #[test]
    fn more_workers_than_jobs_and_no_jobs() {
        assert_eq!(run(16, 32, vec![7, 8], |job| job), vec![7, 8]);
        assert!(run(4, 4, vec![], |job| job).is_empty());
    }

    #[test]
    #[should_panic(expected = "job 3 failed")]
    fn a_panic_in_a_job_resumes_on_the_receiver() {
        run(4, 8, (0..10).collect(), |job| {
            if job == 3 {
                panic!("job 3 failed");
            }
            job
        });
    }

    #[test]
    fn send_fails_once_the_receiver_is_dropped() {
        let (mut sender, receiver) = ordered_thread_pool("test", 1, 1, |job: u64| job);
        drop(receiver);
        assert_eq!(sender.send(1), Err(Disconnected));
    }
}
