use tracing::warn;

use crate::Account;
use std::collections::HashMap;

enum TxState {
    Normal { amount: f32 },
    InDispute { amount: f32 },
    Rejected { amount: f32 },
}

#[derive(Default)]
pub struct PaymentsEngine {
    accounts: HashMap<u64, Account>,
    txs_amounts: HashMap<u64, TxState>,
}

impl PaymentsEngine {
    pub fn deposit(&mut self, txid: u64, id: u64, amount: f32) {
        let account = self.accounts.entry(id).or_default();
        account.available += amount;

        self.txs_amounts.insert(txid, TxState::Normal { amount });
    }

    pub fn withdrawal(&mut self, txid: u64, id: u64, amount: f32) {
        let account = self.accounts.entry(id).or_default();

        // TODO: equals with f32 here
        let tx = if account.available >= amount {
            account.available -= amount;
            TxState::Normal { amount: -amount }
        } else {
            TxState::Rejected { amount: -amount }
        };

        self.txs_amounts.insert(txid, tx);
    }

    pub fn dispute(&mut self, txid: u64, id: u64) {
        let Some(tx) = self.txs_amounts.get_mut(&txid) else {
            warn!(txid = txid, "Tx not found");
            return;
        };

        *tx = match tx {
            TxState::Normal { amount } => {
                // TODO: What to do if the dispute is a withdrawal?
                let account = self.accounts.entry(id).or_default();
                account.available -= *amount;
                account.held += *amount;

                TxState::InDispute { amount: *amount }
            }
            TxState::InDispute { amount } => {
                warn!(txid = txid, "Tx was already in dispute");
                TxState::InDispute { amount: *amount }
            }
            TxState::Rejected { amount } => {
                warn!(txid = txid, "Cannot dispute a rejected tx");
                TxState::Rejected { amount: *amount }
            }
        };
    }

    pub fn resolve(&mut self, txid: u64, id: u64) {
        let Some(tx) = self.txs_amounts.get_mut(&txid) else {
            warn!(txid = txid, "Tx not found");
            return;
        };

        *tx = match tx {
            TxState::Normal { amount } => {
                warn!(txid = txid, "Tx was NOT in dispute");
                TxState::Normal { amount: *amount }
            }
            TxState::InDispute { amount } => {
                let account = self.accounts.entry(id).or_default();
                account.available += *amount;
                account.held -= *amount;
                TxState::Normal { amount: *amount }
            }
            TxState::Rejected { amount } => {
                warn!(txid = txid, "Cannot resolve a rejected tx");
                TxState::Rejected { amount: *amount }
            }
        };
    }

    pub fn chargeback(&mut self, txid: u64, id: u64) {
        let Some(tx) = self.txs_amounts.get_mut(&txid) else {
            warn!(txid = txid, "Tx not found");
            return;
        };

        *tx = match tx {
            TxState::Normal { amount } => {
                warn!(txid = txid, "Tx was NOT in dispute");
                TxState::Normal { amount: *amount }
            }
            TxState::InDispute { amount } => {
                let account = self.accounts.entry(id).or_default();
                account.held -= *amount;
                account.locked = true;
                TxState::Normal { amount: *amount }
            }
            TxState::Rejected { amount } => {
                warn!(txid = txid, "Cannot chargeback a rejected tx");
                TxState::Rejected { amount: *amount }
            }
        };
    }

    pub fn iter_accounts(&self) -> impl Iterator<Item = (&u64, &Account)> {
        self.accounts.iter()
    }
}
