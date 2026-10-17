use tracing::warn;

use rust_decimal::Decimal;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineError {
    AccountFrozen,
    NegativeAmount,
    InsufficientFunds,
    TxNotFound,
    ClientMismatch { expected: u16, got: u16 },
    AlreadyInDispute,
    NotInDispute,
    TxRejected,
}

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
    client: u16,
}

#[derive(Default)]
pub struct Account {
    pub available: Decimal,
    pub held: Decimal,
    pub locked: bool,
}

#[derive(Default)]
pub struct PaymentsEngine {
    accounts: BTreeMap<u16, Account>,
    txs_amounts: BTreeMap<u32, TxState>,
}

impl PaymentsEngine {
    pub fn deposit(&mut self, txid: u32, id: u16, amount: Decimal) -> Result<(), EngineError> {
        if self.is_locked(id) {
            warn!(client = id, txid = txid, "account frozen; deposit ignored");
            return Err(EngineError::AccountFrozen);
        }
        if amount.is_sign_negative() {
            warn!(client = id, txid = txid, "negative amount; deposit ignored");
            return Err(EngineError::NegativeAmount);
        }
        let account = self.get_account(id);
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
        Ok(())
    }

    pub fn withdrawal(&mut self, txid: u32, id: u16, amount: Decimal) -> Result<(), EngineError> {
        if self.is_locked(id) {
            warn!(
                client = id,
                txid = txid,
                "account frozen; withdrawal ignored"
            );
            return Err(EngineError::AccountFrozen);
        }
        if amount.is_sign_negative() {
            warn!(
                client = id,
                txid = txid,
                "negative amount; withdrawal ignored"
            );
            return Err(EngineError::NegativeAmount);
        }

        // Read the available balance without creating an account
        let available = self
            .accounts
            .get(&id)
            .map(|a| a.available)
            .unwrap_or_default();
        if available >= amount {
            let account = self.get_account(id);
            account.available -= amount;
            self.txs_amounts.insert(
                txid,
                TxState {
                    status: TxStatus::Normal,
                    amount,
                    kind: TxKind::Withdrawal,
                    client: id,
                },
            );
            Ok(())
        } else {
            self.txs_amounts.insert(
                txid,
                TxState {
                    status: TxStatus::Rejected,
                    amount,
                    kind: TxKind::Withdrawal,
                    client: id,
                },
            );
            Err(EngineError::InsufficientFunds)
        }
    }

    pub fn dispute(&mut self, txid: u32, id: u16) -> Result<(), EngineError> {
        if self.is_locked(id) {
            warn!(client = id, txid = txid, "account frozen; dispute ignored");
            return Err(EngineError::AccountFrozen);
        }

        let (kind, amount, status) = self.validate_tx(txid, id)?;

        match (status, kind) {
            (TxStatus::Normal, TxKind::Deposit) => {
                // Spec: move the deposited funds from available to held.
                let account = self.get_account(id);
                account.available -= amount;
                account.held += amount;
            }
            (TxStatus::Normal, TxKind::Withdrawal) => {
                // The withdrawn funds already left the account; there is
                // nothing in `available` to move to `held`. We only mark
                // the tx as under dispute so a later chargeback can
                // reverse (refund) it, keeping `held` non-negative.
            }
            (TxStatus::InDispute, _) => {
                warn!(txid = txid, "Tx was already in dispute");
                return Err(EngineError::AlreadyInDispute);
            }
            (TxStatus::Rejected, _) => {
                warn!(txid = txid, "Cannot dispute a rejected tx");
                return Err(EngineError::TxRejected);
            }
        }
        if let Some(tx) = self.txs_amounts.get_mut(&txid) {
            tx.status = TxStatus::InDispute;
        }
        Ok(())
    }

    pub fn resolve(&mut self, txid: u32, id: u16) -> Result<(), EngineError> {
        if self.is_locked(id) {
            warn!(client = id, txid = txid, "account frozen; resolve ignored");
            return Err(EngineError::AccountFrozen);
        }
        let (kind, amount, status) = self.validate_tx(txid, id)?;

        match (status, kind) {
            (TxStatus::Normal, _) => {
                warn!(txid = txid, "Tx was NOT in dispute");
                return Err(EngineError::NotInDispute);
            }
            (TxStatus::InDispute, TxKind::Deposit) => {
                let account = self.get_account(id);
                account.available += amount;
                account.held -= amount;
            }
            (TxStatus::InDispute, TxKind::Withdrawal) => {
                // Nothing was moved on dispute, so nothing to release.
            }
            (TxStatus::Rejected, _) => {
                warn!(txid = txid, "Cannot resolve a rejected tx");
                return Err(EngineError::TxRejected);
            }
        }
        if let Some(tx) = self.txs_amounts.get_mut(&txid) {
            tx.status = TxStatus::Normal;
        }
        Ok(())
    }

    pub fn chargeback(&mut self, txid: u32, id: u16) -> Result<(), EngineError> {
        if self.is_locked(id) {
            warn!(
                client = id,
                txid = txid,
                "account frozen; chargeback ignored"
            );
            return Err(EngineError::AccountFrozen);
        }
        let (kind, amount, status) = self.validate_tx(txid, id)?;

        match (status, kind) {
            (TxStatus::Normal, _) => {
                warn!(txid = txid, "Tx was NOT in dispute");
                return Err(EngineError::NotInDispute);
            }
            (TxStatus::InDispute, TxKind::Deposit) => {
                // Reverse the deposit: remove the held funds.
                self.get_account(id).held -= amount;
            }
            (TxStatus::InDispute, TxKind::Withdrawal) => {
                // Reverse the withdrawal: refund the withdrawn funds.
                self.get_account(id).available += amount;
            }
            (TxStatus::Rejected, _) => {
                warn!(txid = txid, "Cannot chargeback a rejected tx");
                return Err(EngineError::TxRejected);
            }
        }
        self.lock_account(id);
        if let Some(tx) = self.txs_amounts.get_mut(&txid) {
            tx.status = TxStatus::Normal;
        }
        Ok(())
    }

    fn is_locked(&self, id: u16) -> bool {
        self.accounts.get(&id).map(|a| a.locked).unwrap_or(false)
    }

    fn get_account(&mut self, id: u16) -> &mut Account {
        self.accounts.entry(id).or_default()
    }

    fn lock_account(&mut self, id: u16) {
        self.get_account(id).locked = true;
    }

    /// Validate this transaction belongs to this user
    fn validate_tx(&self, txid: u32, id: u16) -> Result<(TxKind, Decimal, TxStatus), EngineError> {
        match self.txs_amounts.get(&txid) {
            None => {
                warn!(txid = txid, "Tx not found");
                Err(EngineError::TxNotFound)
            }
            Some(tx) if tx.client != id => {
                warn!(
                    txid = txid,
                    expected = tx.client,
                    got = id,
                    "Client mismatch"
                );
                Err(EngineError::ClientMismatch {
                    expected: tx.client,
                    got: id,
                })
            }
            Some(tx) => Ok((tx.kind, tx.amount, tx.status)),
        }
    }

    pub fn iter_accounts(&self) -> impl Iterator<Item = (&u16, &Account)> {
        self.accounts.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;

    mod deposit {
        use super::*;

        #[test]
        fn ok() {
            let mut e = PaymentsEngine::default();
            assert!(e.deposit(1, 1, dec!(10)).is_ok());
            let a = e.get_account(1);
            assert_eq!(a.available, dec!(10));
            assert_eq!(a.held, dec!(0));
            assert!(!a.locked);
        }

        #[test]
        fn rejects_negative() {
            let mut e = PaymentsEngine::default();
            assert_eq!(e.deposit(1, 1, dec!(-5)), Err(EngineError::NegativeAmount));
            // No account is created for a rejected deposit.
            assert!(!e.iter_accounts().any(|(c, _)| *c == 1));
        }

        #[test]
        fn on_frozen_account() {
            let mut e = PaymentsEngine::default();
            e.lock_account(1);
            assert_eq!(e.deposit(2, 1, dec!(5)), Err(EngineError::AccountFrozen));
            // Funds unchanged by the rejected deposit.
            assert_eq!(e.get_account(1).available, dec!(0));
        }

        #[test]
        fn zero_is_recorded_and_disputable() {
            let mut e = PaymentsEngine::default();
            assert!(e.deposit(1, 1, dec!(0)).is_ok());

            // A zero deposit still opens the account and records a real tx.
            let a = e.get_account(1);
            assert_eq!(a.available, dec!(0));
            assert_eq!(a.held, dec!(0));

            // The tx is recorded and can be disputed, then charged back.
            assert!(e.dispute(1, 1).is_ok());
            assert!(e.chargeback(1, 1).is_ok());

            let a = e.get_account(1);
            assert!(a.locked);
            assert_eq!(a.available, dec!(0));
            assert_eq!(a.held, dec!(0));
        }
    }

    mod withdrawal {
        use super::*;

        #[test]
        fn ok() {
            let mut e = PaymentsEngine::default();
            e.deposit(1, 1, dec!(10)).unwrap();
            assert!(e.withdrawal(2, 1, dec!(4)).is_ok());
            assert_eq!(e.get_account(1).available, dec!(6));
        }

        #[test]
        fn rejects_negative() {
            let mut e = PaymentsEngine::default();
            e.deposit(1, 1, dec!(10)).unwrap();
            assert_eq!(
                e.withdrawal(2, 1, dec!(-5)),
                Err(EngineError::NegativeAmount)
            );
            assert_eq!(e.get_account(1).available, dec!(10));
        }

        #[test]
        fn insufficient_funds() {
            let mut e = PaymentsEngine::default();
            e.deposit(1, 1, dec!(10)).unwrap();
            assert_eq!(
                e.withdrawal(2, 1, dec!(15)),
                Err(EngineError::InsufficientFunds)
            );
            // Available is untouched, and the tx is recorded as rejected so a later
            // dispute on it is refused.
            assert_eq!(e.get_account(1).available, dec!(10));
            assert_eq!(e.dispute(2, 1), Err(EngineError::TxRejected));
        }

        #[test]
        fn on_frozen_account() {
            let mut e = PaymentsEngine::default();
            e.lock_account(1);
            assert_eq!(e.withdrawal(2, 1, dec!(4)), Err(EngineError::AccountFrozen));
            assert_eq!(e.get_account(1).available, dec!(0));
        }

        #[test]
        fn rejected_on_unknown_client_creates_no_account() {
            let mut e = PaymentsEngine::default();
            assert_eq!(
                e.withdrawal(1, 1, dec!(5)),
                Err(EngineError::InsufficientFunds)
            );
            // A bank does not open an account for a failed withdrawal.
            assert!(!e.iter_accounts().any(|(c, _)| *c == 1));
        }

        #[test]
        fn zero_on_existing_is_recorded_and_disputable() {
            let mut e = PaymentsEngine::default();

            e.deposit(1, 1, dec!(10)).unwrap();
            assert!(e.withdrawal(2, 1, dec!(0)).is_ok());
            assert_eq!(e.get_account(1).available, dec!(10));
            assert!(e.dispute(2, 1).is_ok());
            assert!(e.chargeback(2, 1).is_ok());

            let a = e.get_account(1);
            assert!(a.locked);
            assert_eq!(a.available, dec!(10));
        }

        #[test]
        fn zero_on_unknown_client_is_recorded_and_disputable() {
            let mut e = PaymentsEngine::default();

            assert!(e.withdrawal(1, 1, dec!(0)).is_ok());

            let a = e.get_account(1);
            assert_eq!(a.available, dec!(0));
            assert!(!a.locked);
            assert!(e.dispute(1, 1).is_ok());
            assert!(e.chargeback(1, 1).is_ok());
            assert!(e.get_account(1).locked);
        }
    }

    mod dispute {
        use super::*;

        #[test]
        fn unknown_tx() {
            let mut e = PaymentsEngine::default();
            assert_eq!(e.dispute(999, 1), Err(EngineError::TxNotFound));
        }

        #[test]
        fn client_mismatch() {
            let mut e = PaymentsEngine::default();
            e.deposit(1, 1, dec!(10)).unwrap();
            assert_eq!(
                e.dispute(1, 2),
                Err(EngineError::ClientMismatch {
                    expected: 1,
                    got: 2
                })
            );
            // The real owner's funds are untouched.
            assert_eq!(e.get_account(1).available, dec!(10));
            assert_eq!(e.get_account(1).held, dec!(0));
        }

        #[test]
        fn already_in_dispute() {
            let mut e = PaymentsEngine::default();
            e.deposit(1, 1, dec!(10)).unwrap();
            e.dispute(1, 1).unwrap();
            assert_eq!(e.dispute(1, 1), Err(EngineError::AlreadyInDispute));
            // No double-hold: held stays at the original amount.
            assert_eq!(e.get_account(1).held, dec!(10));
        }

        #[test]
        fn rejected_tx() {
            let mut e = PaymentsEngine::default();
            e.deposit(1, 1, dec!(10)).unwrap();
            e.withdrawal(2, 1, dec!(15)).err().unwrap(); // recorded as rejected
            assert_eq!(e.dispute(2, 1), Err(EngineError::TxRejected));
        }

        #[test]
        fn ok_moves_deposit_to_held() {
            let mut e = PaymentsEngine::default();
            e.deposit(1, 1, dec!(10)).unwrap();
            assert!(e.dispute(1, 1).is_ok());
            let a = e.get_account(1);
            assert_eq!(a.available, dec!(0));
            assert_eq!(a.held, dec!(10));
        }
    }

    mod resolve {
        use super::*;

        #[test]
        fn not_in_dispute() {
            let mut e = PaymentsEngine::default();
            e.deposit(1, 1, dec!(10)).unwrap();
            assert_eq!(e.resolve(1, 1), Err(EngineError::NotInDispute));
            assert_eq!(e.get_account(1).available, dec!(10));
        }

        #[test]
        fn ok_returns_deposit_to_available() {
            let mut e = PaymentsEngine::default();
            e.deposit(1, 1, dec!(10)).unwrap();
            e.dispute(1, 1).unwrap();
            assert!(e.resolve(1, 1).is_ok());
            let a = e.get_account(1);
            assert_eq!(a.available, dec!(10));
            assert_eq!(a.held, dec!(0));
        }

        #[test]
        fn rejected_tx() {
            let mut e = PaymentsEngine::default();
            e.deposit(1, 1, dec!(10)).unwrap();
            e.withdrawal(2, 1, dec!(15)).err().unwrap();
            assert_eq!(e.resolve(2, 1), Err(EngineError::TxRejected));
        }
    }

    mod chargeback {
        use super::*;

        #[test]
        fn not_in_dispute() {
            let mut e = PaymentsEngine::default();
            e.deposit(1, 1, dec!(10)).unwrap();
            assert_eq!(e.chargeback(1, 1), Err(EngineError::NotInDispute));
            assert!(!e.get_account(1).locked);
        }

        #[test]
        fn ok_locks_account_and_clears_held() {
            let mut e = PaymentsEngine::default();
            e.deposit(1, 1, dec!(10)).unwrap();
            e.dispute(1, 1).unwrap();
            assert!(e.chargeback(1, 1).is_ok());
            let a = e.get_account(1);
            assert_eq!(a.available, dec!(0));
            assert_eq!(a.held, dec!(0));
            assert!(a.locked);
        }

        #[test]
        fn rejected_tx() {
            let mut e = PaymentsEngine::default();
            e.deposit(1, 1, dec!(10)).unwrap();
            e.withdrawal(2, 1, dec!(15)).err().unwrap();
            assert_eq!(e.chargeback(2, 1), Err(EngineError::TxRejected));
        }
    }

    mod frozen {
        use super::*;

        #[test]
        fn blocks_dispute_resolve_chargeback() {
            let mut e = PaymentsEngine::default();
            e.lock_account(1);
            // The frozen check runs before any tx lookup, so all three are refused
            // with `AccountFrozen` rather than `TxNotFound`.
            assert_eq!(e.dispute(1, 1), Err(EngineError::AccountFrozen));
            assert_eq!(e.resolve(1, 1), Err(EngineError::AccountFrozen));
            assert_eq!(e.chargeback(1, 1), Err(EngineError::AccountFrozen));
        }
    }
}
