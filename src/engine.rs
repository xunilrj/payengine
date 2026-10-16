use tracing::warn;

use rust_decimal::Decimal;
use std::collections::HashMap;

#[derive(Clone, Copy)]
enum TxKind {
    Deposit,
    Withdrawal,
}

#[derive(Clone, Copy)]
enum TxStatus {
    Normal,
    InDispute,
    Rejected,
}

struct TxState {
    status: TxStatus,
    amount: Decimal,
    kind: TxKind,
    client: u64,
}

#[derive(Default)]
pub struct Account {
    pub available: Decimal,
    pub held: Decimal,
    pub locked: bool,
}

#[derive(Default)]
pub struct PaymentsEngine {
    accounts: HashMap<u64, Account>,
    txs_amounts: HashMap<u64, TxState>,
}

impl PaymentsEngine {
    pub fn deposit(&mut self, txid: u64, id: u64, amount: Decimal) {
        let account = self.accounts.entry(id).or_default();
        account.available += amount;

        self.txs_amounts.insert(
            txid,
            TxState {
                status: TxStatus::Normal,
                amount,
                kind: TxKind::Deposit,
                client: id,
            },
        );
    }

    pub fn withdrawal(&mut self, txid: u64, id: u64, amount: Decimal) {
        let account = self.accounts.entry(id).or_default();

        let tx = if account.available >= amount {
            account.available -= amount;
            TxState {
                status: TxStatus::Normal,
                amount,
                kind: TxKind::Withdrawal,
                client: id,
            }
        } else {
            TxState {
                status: TxStatus::Rejected,
                amount,
                kind: TxKind::Withdrawal,
                client: id,
            }
        };

        self.txs_amounts.insert(txid, tx);
    }

    pub fn dispute(&mut self, txid: u64, id: u64) {
        let Some(tx) = self.txs_amounts.get_mut(&txid) else {
            warn!(txid = txid, "Tx not found");
            return;
        };
        if tx.client != id {
            warn!(
                txid = txid,
                expected = tx.client,
                got = id,
                "Client mismatch"
            );
            return;
        }

        match tx.status {
            TxStatus::Normal => {
                match tx.kind {
                    TxKind::Deposit => {
                        // Spec: move the deposited funds from available to held.
                        let account = self.accounts.entry(id).or_default();
                        account.available -= tx.amount;
                        account.held += tx.amount;
                    }
                    TxKind::Withdrawal => {
                        // The withdrawn funds already left the account; there is
                        // nothing in `available` to move to `held`. We only mark
                        // the tx as under dispute so a later chargeback can
                        // reverse (refund) it, keeping `held` non-negative.
                    }
                }
                tx.status = TxStatus::InDispute;
            }
            TxStatus::InDispute => {
                warn!(txid = txid, "Tx was already in dispute");
            }
            TxStatus::Rejected => {
                warn!(txid = txid, "Cannot dispute a rejected tx");
            }
        }
    }

    pub fn resolve(&mut self, txid: u64, id: u64) {
        let Some(tx) = self.txs_amounts.get_mut(&txid) else {
            warn!(txid = txid, "Tx not found");
            return;
        };
        if tx.client != id {
            warn!(
                txid = txid,
                expected = tx.client,
                got = id,
                "Client mismatch"
            );
            return;
        }

        match tx.status {
            TxStatus::Normal => {
                warn!(txid = txid, "Tx was NOT in dispute");
            }
            TxStatus::InDispute => {
                match tx.kind {
                    TxKind::Deposit => {
                        let account = self.accounts.entry(id).or_default();
                        account.available += tx.amount;
                        account.held -= tx.amount;
                    }
                    TxKind::Withdrawal => {
                        // Nothing was moved on dispute, so nothing to release.
                    }
                }
                tx.status = TxStatus::Normal;
            }
            TxStatus::Rejected => {
                warn!(txid = txid, "Cannot resolve a rejected tx");
            }
        }
    }

    pub fn chargeback(&mut self, txid: u64, id: u64) {
        let Some(tx) = self.txs_amounts.get_mut(&txid) else {
            warn!(txid = txid, "Tx not found");
            return;
        };
        if tx.client != id {
            warn!(
                txid = txid,
                expected = tx.client,
                got = id,
                "Client mismatch"
            );
            return;
        }

        match tx.status {
            TxStatus::Normal => {
                warn!(txid = txid, "Tx was NOT in dispute");
            }
            TxStatus::InDispute => {
                let account = self.accounts.entry(id).or_default();
                match tx.kind {
                    TxKind::Deposit => {
                        // Reverse the deposit: remove the held funds.
                        account.held -= tx.amount;
                    }
                    TxKind::Withdrawal => {
                        // Reverse the withdrawal: refund the withdrawn funds.
                        account.available += tx.amount;
                    }
                }
                account.locked = true;
                tx.status = TxStatus::Normal;
            }
            TxStatus::Rejected => {
                warn!(txid = txid, "Cannot chargeback a rejected tx");
            }
        }
    }

    pub fn iter_accounts(&self) -> impl Iterator<Item = (&u64, &Account)> {
        self.accounts.iter()
    }
}
