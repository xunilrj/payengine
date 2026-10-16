use crate::Account;
use std::collections::HashMap;

#[derive(Default)]
pub struct PaymentsEngine {
    accounts: HashMap<u64, Account>,
    txs_amounts: HashMap<u64, f32>,
}

impl PaymentsEngine {
    pub fn deposit(&mut self, txid: u64, id: u64, amount: f32) {
        let account = self.accounts.entry(id).or_default();
        account.available += amount;

        self.txs_amounts.insert(txid, amount);
    }

    pub fn withdrawal(&mut self, txid: u64, id: u64, amount: f32) {
        let account = self.accounts.entry(id).or_default();
        account.available -= amount;

        self.txs_amounts.insert(txid, -amount);
    }

    pub fn dispute(&mut self, txid: u64, id: u64) {
        let amount = self.txs_amounts.get(&txid).unwrap();
        let account = self.accounts.entry(id).or_default();

        // TODO: What to do if the dispute is a withdrawal?
        account.available -= amount;
        account.held += amount;
    }

    pub fn resolve(&mut self, txid: u64, id: u64) {
        let amount = self.txs_amounts.get(&txid).unwrap();
        let account = self.accounts.entry(id).or_default();

        account.available += amount;
        account.held -= amount;
    }

    pub fn iter_accounts(&self) -> impl Iterator<Item = (&u64, &Account)> {
        self.accounts.iter()
    }
}
