use tracing::warn;

use crate::Account;
use std::collections::HashMap;

#[derive(Clone, Copy)]
enum TxKind {
    Deposit,
    Withdrawal,
}

enum TxState {
    Normal { amount: f32, kind: TxKind },
    InDispute { amount: f32, kind: TxKind },
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

        self.txs_amounts.insert(
            txid,
            TxState::Normal {
                amount,
                kind: TxKind::Deposit,
            },
        );
    }

    pub fn withdrawal(&mut self, txid: u64, id: u64, amount: f32) {
        let account = self.accounts.entry(id).or_default();

        // TODO: equals with f32 here
        let tx = if account.available >= amount {
            account.available -= amount;
            TxState::Normal {
                amount,
                kind: TxKind::Withdrawal,
            }
        } else {
            TxState::Rejected { amount }
        };

        self.txs_amounts.insert(txid, tx);
    }

    pub fn dispute(&mut self, txid: u64, id: u64) {
        let Some(tx) = self.txs_amounts.get_mut(&txid) else {
            warn!(txid = txid, "Tx not found");
            return;
        };

        *tx = match tx {
            TxState::Normal { amount, kind } => {
                match kind {
                    TxKind::Deposit => {
                        // Spec: move the deposited funds from available to held.
                        let account = self.accounts.entry(id).or_default();
                        account.available -= *amount;
                        account.held += *amount;
                    }
                    TxKind::Withdrawal => {
                        // The withdrawn funds already left the account; there is
                        // nothing in `available` to move to `held`. We only mark
                        // the tx as under dispute so a later chargeback can
                        // reverse (refund) it, keeping `held` non-negative.
                    }
                }
                TxState::InDispute {
                    amount: *amount,
                    kind: *kind,
                }
            }
            TxState::InDispute { amount, kind } => {
                warn!(txid = txid, "Tx was already in dispute");
                TxState::InDispute {
                    amount: *amount,
                    kind: *kind,
                }
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
            TxState::Normal { amount, kind } => {
                warn!(txid = txid, "Tx was NOT in dispute");
                TxState::Normal {
                    amount: *amount,
                    kind: *kind,
                }
            }
            TxState::InDispute { amount, kind } => {
                match kind {
                    TxKind::Deposit => {
                        let account = self.accounts.entry(id).or_default();
                        account.available += *amount;
                        account.held -= *amount;
                    }
                    TxKind::Withdrawal => {
                        // Nothing was moved on dispute, so nothing to release.
                    }
                }
                TxState::Normal {
                    amount: *amount,
                    kind: *kind,
                }
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
            TxState::Normal { amount, kind } => {
                warn!(txid = txid, "Tx was NOT in dispute");
                TxState::Normal {
                    amount: *amount,
                    kind: *kind,
                }
            }
            TxState::InDispute { amount, kind } => {
                let account = self.accounts.entry(id).or_default();
                match kind {
                    TxKind::Deposit => {
                        account.held -= *amount;
                    }
                    TxKind::Withdrawal => {
                        // Reverse the withdrawal: refund the withdrawn funds.
                        account.available += *amount;
                    }
                }
                account.locked = true;
                TxState::Normal {
                    amount: *amount,
                    kind: *kind,
                }
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
