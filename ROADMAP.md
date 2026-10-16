# Roadmap

Derived from a review of the implementation against the "Types of Transactions" section of the spec. Items are ordered by impact. Nothing here is implemented yet.

## 1. Track dispute state per transaction — DONE

Implemented in `src/engine.rs` as a `TxState` enum (`Normal { amount }` / `InDispute { amount }`) replacing the flat `tx → amount` map.

- ✅ Per-transaction record with a state machine carrying amount + dispute status.
- ✅ `dispute` on a tx already under dispute is ignored (warns, no funds moved).
- ✅ `resolve` / `chargeback` on a tx not under dispute is ignored (warns, no funds moved).

Assumptions recorded (see also the spec note on locking):
- After `chargeback`, the tx returns to `Normal`. The spec does not require a terminal tx state; it only requires the *account* to be locked on chargeback, and a locked account has no specified operational effect. Treating chargeback as non-terminal is therefore acceptable.
- Storing the original client on the tx record is not part of this item; it is tracked under item 5.

## 2. Stop panicking on missing / invalid tx references — DONE

`dispute`, `resolve`, and `chargeback` previously called `self.txs_amounts.get(&txid).unwrap()`. The spec says each of these should be **ignored** (treated as a partner error) when the referenced tx does not exist or is not under dispute.

- ✅ Replaced every `unwrap()` on tx lookup with a guarded `let Some(tx) = ... else { warn; return; }` branch that silently skips the operation.

## 3. Guard withdrawals against insufficient funds — DONE

`withdrawal` does `account.available -= amount` unconditionally. Spec: *"If a client does not have sufficient available funds the withdrawal should fail and the total amount of funds should not change."*

- ✅ Check `available >= amount` before debiting; otherwise leave the account unchanged.
- ✅ Rejected withdrawals are recorded as `TxState::Rejected` (not `Normal`), and `dispute`/`resolve`/`chargeback` ignore a `Rejected` tx, so a failed withdrawal cannot be disputed as if it happened.
- The `// TODO: equals with f32 here` on the guard comparison remains, but that is an item-6 (decimal type) concern, not item 3.

## 4. Handle disputes on withdrawals — DONE

The spec's dispute/resolve/chargeback formula is deposit-centric and never shows a withdrawal being disputed; the case is underspecified. Implemented a direction-aware `TxKind` on `TxState` so deposits and withdrawals take different paths:

- ✅ Deposits follow the spec formula exactly (`available -= amount`, `held += amount` on dispute; reverse on resolve; `held -= amount` on chargeback).
- ✅ A disputed withdrawal only flags the tx as under dispute — `available` and `held` do not move, so `held` never goes negative.
- ✅ Chargeback on a disputed withdrawal refunds the withdrawn amount (`available += amount`) and locks the account.
- ✅ Resolve on a disputed withdrawal is a no-op (the withdrawal stands).
- ✅ `held` is non-negative for both deposit- and withdrawal-originated disputes.

Recorded deviation (documented in README): a withdrawal chargeback increases `total` by the disputed amount, contradicting the spec's deposit-centric chargeback wording. Refunding a disputed withdrawal is the correct reversal, so this is the intended semantics.

## 5. Validate the client on dispute/resolve/chargeback — DONE

Dispute/resolve/chargeback records carry their own `client` field, but the engine previously applied the operation to that client without checking it matches the original transaction's client — a mismatched record would move money on the wrong account (or create a phantom account with negative balances). Tx IDs are globally unique, so the original tx's client is authoritative.

- ✅ `TxState` now stores the original `client` (recorded when the tx is first seen on deposit/withdrawal).
- ✅ `dispute`/`resolve`/`chargeback` compare the record's client to the stored client; on mismatch they emit `WARN Client mismatch` and do nothing (no balance change, no phantom account, no lock). This matches the spec's "ignore partner errors" pattern used by items 1–2.
- ✅ `TxState` flattened from a per-variant enum to a struct `{ status, amount, kind, client }` to avoid repeating fields.
- Tests: `tests/client_mismatch/{dispute,resolve,chargeback}.csv`.

## 6. Replace `f32` with a fixed-point / decimal money type — DONE

`f32` accumulates rounding error across many sums and cannot represent four-decimal-place money exactly. The spec emphasizes correctness, safety, and behavior on larger data sets.

- ✅ Replaced `f32` with `rust_decimal::Decimal` (pinned `1.43.0`, `features = ["serde"]`) across `Account.available`/`held`, `TxState.amount`, `InputCsv.amount`, `OutputCsv.{available,held,total}`, and all engine method signatures.
- ✅ The `// TODO: equals with f32 here` is resolved — `Decimal` comparisons are exact, so the withdrawal guard `available >= amount` is now exact.
- ✅ Input: amounts are accepted verbatim with full precision (rust_decimal supports ~28 significant digits), so >4-place values are preserved.
- ✅ Output: Decimals serialize in their natural scale-preserving form (significant digits kept; input trailing zeros normalized at parse, e.g. `1.0` → `1`). The spec does not require always-4-decimal output (it states "displaying decimals for round values do not matter" and treats `2` == `2.0`), so this is compliant.
- No rounding policy needed: the engine only adds/subtracts, which `Decimal` does exactly; no division/multiplication occurs.
- Tests: `tests/decimal/{float_bug,accumulation,precision6,precision8}.csv`.

## 7. Chargeback correctness precondition — DONE (via items 1 & 2)

`chargeback` only decreases `held` (deposits) or refunds `available` (withdrawals). That is correct only when the tx was previously disputed. This is now guarded: a chargeback on a non-existent tx is ignored (item 2), and a chargeback on a tx not under dispute (`Normal`) is ignored with a warning (item 1), so `held` can no longer be driven negative by an out-of-order chargeback. Covered by `tests/dispute_state/chargeback_not_disputed.csv`.

## 8. Output precision — DONE (via item 6)

Spec: "displaying decimals for round values do not matter" — the spec does **not** require always-4-decimal output (it treats `2` == `2.0`). With `rust_decimal` (item 6), `available`/`held`/`total` serialize in their natural scale-preserving form, which is spec-compliant. No forced 4-place formatting is applied.

## 8b. Parser robustness

Moved out of item 2: this is a general input-parsing concern rather than a dispute/chargeback tx-reference concern.

- `record.amount.unwrap()` on deposit/withdrawal in `main.rs` aborts the whole run if a deposit/withdrawal row is missing its amount. Skip malformed rows instead of aborting.
- More broadly, harden the CSV deserialization loop so a single bad record (bad type, bad u16/u64, bad decimal, missing amount where required) is reported/skipped rather than taking down the entire engine.

## 9. Tests to add

No new implementation is required to scope these; they pin the behavior the items above introduce. Tests live in feature subfolders under `tests/`.

- ✅ Dispute / resolve / chargeback on a non-existent tx → ignored, no panic, balances unchanged.
- ✅ Resolve or chargeback on a tx not under dispute → ignored.
- ✅ Double dispute on the same tx → second is ignored.
- ✅ Withdrawal exceeding available → rejected, balances unchanged.
- ✅ Dispute on a withdrawal → correct sign, no negative `held`.
- ✅ Chargeback on a non-disputed tx → ignored (held not driven negative).
- ✅ Resolve / chargeback on a rejected withdrawal → ignored.
- ✅ Dispute/resolve/chargeback with a mismatched `client` → ignored.
- ✅ Accumulation/rounding test that would fail under `f32` but passes under the chosen decimal type (`tests/decimal/float_bug.csv`, `accumulation.csv`), plus >4-precision preservation (`precision6.csv`, `precision8.csv`).