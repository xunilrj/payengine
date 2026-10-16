use crate::Account;
use std::collections::HashMap;

#[derive(Default)]
pub struct PaymentsEngine {
    accounts: HashMap<u64, Account>,
}

impl PaymentsEngine {
    pub fn deposit(&mut self, id: u64, amount: f32) {
        let account = self.accounts.entry(id).or_default();
        account.available += amount;
    }

    pub fn withdrawal(&mut self, id: u64, amount: f32) {
        let account = self.accounts.entry(id).or_default();
        account.available -= amount;
    }

    pub fn iter_accounts(&self) -> impl Iterator<Item = (&u64, &Account)> {
        self.accounts.iter()
    }
}
