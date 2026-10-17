//! Writes the benchmark workload as an input CSV for the real CLI.
//! Usage: cargo run --release --example generate_csv -- <rows> <seed> <output.csv>

#[path = "../benches/workload/mod.rs"]
mod workload;

use payengine::engine::Transaction;
use std::io::{BufWriter, Write};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let usage = "usage: generate_csv <rows> <seed> <output.csv>";
    let rows: usize = args.next().ok_or_else(|| anyhow::anyhow!(usage))?.parse()?;
    let seed: u64 = args.next().ok_or_else(|| anyhow::anyhow!(usage))?.parse()?;
    let path = args.next().ok_or_else(|| anyhow::anyhow!(usage))?;

    let mut out = BufWriter::new(std::fs::File::create(&path)?);
    writeln!(out, "type,client,tx,amount")?;
    for transaction in workload::Workload::new(seed).take(rows) {
        match transaction {
            Transaction::Deposit { client, tx, amount } => {
                writeln!(out, "deposit,{client},{tx},{amount}")?
            }
            Transaction::Withdrawal { client, tx, amount } => {
                writeln!(out, "withdrawal,{client},{tx},{amount}")?
            }
            Transaction::Dispute { client, tx } => writeln!(out, "dispute,{client},{tx},")?,
            Transaction::Resolve { client, tx } => writeln!(out, "resolve,{client},{tx},")?,
            Transaction::Chargeback { client, tx } => writeln!(out, "chargeback,{client},{tx},")?,
        }
    }
    out.flush()?;
    Ok(())
}
