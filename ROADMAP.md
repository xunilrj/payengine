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

## 2. Stop panicking on missing / invalid tx references

`dispute`, `resolve`, and `chargeback` all call `self.txs_amounts.get(&txid).unwrap()`. The spec says each of these should be **ignored** (treated as a partner error) when the referenced tx does not exist or is not under dispute.

- Replace every `unwrap()` on tx lookup with a guarded branch that silently skips the operation.
- Same for `record.amount.unwrap()` on deposit/withdrawal in `main.rs`: skip malformed rows instead of aborting the whole run.

## 3. Guard withdrawals against insufficient funds

`withdrawal` does `account.available -= amount` unconditionally. Spec: *"If a client does not have sufficient available funds the withdrawal should fail and the total amount of funds should not change."*

- Check `available >= amount` before debiting; otherwise leave the account unchanged and do not record the tx (or record it as failed/rejected so it cannot be disputed later).

## 4. Handle disputes on withdrawals

Flagged by the `// TODO` in `dispute`. Withdrawals are stored as negative amounts, so the current sign arithmetic produces a negative `held` when a withdrawal is disputed. Decide and implement the intended semantics:

- A dispute reverses a transaction. For a withdrawal, "reversing" means returning the withdrawn funds: `available += amount`, and the held concept needs to be defined for the withdrawal case.
- Confirm whether `held` should ever be negative, and ensure the dispute/resolve/chargeback signs are consistent for both deposit- and withdrawal-originated txs.

## 5. Validate the client on dispute/resolve/chargeback

Dispute/resolve/chargeback records carry their own `client` field, but the engine applies the operation to that client without checking it matches the original transaction's client. Tx IDs are globally unique, so the original tx's client is known — but it is not currently stored on the tx record (`TxState` holds only `amount`).

- Extend the per-transaction record (`TxState`) to also store the original client.
- Look up the original tx, compare its stored client to the record's `client`, and ignore the operation on mismatch (partner error), or document the chosen behavior.

## 6. Replace `f32` with a fixed-point / decimal money type

`f32` accumulates rounding error across many sums and cannot represent four-decimal-place money exactly. The spec emphasizes correctness, safety, and behavior on larger data sets.

- Evaluate `rust_decimal` or a scaled-integer (e.g. amounts in millionths, since precision is four places) representation.
- Apply consistently across `Account` fields, the tx amount store, and the CSV (de)serialization.

## 7. Chargeback correctness precondition

`chargeback` only decreases `held`. That is correct *only* when the tx was previously disputed (funds already moved from `available` → `held`). Because dispute state is not tracked today (item 1), a chargeback on a non-disputed tx would drive `held` negative and distort `total`. This is resolved automatically once item 1 lands, but should be covered by a test.

## 8. Output precision

Spec: output values with four places past the decimal. Current output relies on `f32` default formatting. Once the money type is settled (item 6), ensure the serialized `available`, `held`, and `total` are emitted with the required precision.

## 9. Tests to add

No new implementation is required to scope these; they pin the behavior the items above introduce.

- Dispute / resolve / chargeback on a non-existent tx → ignored, no panic, balances unchanged.
- Resolve or chargeback on a tx not under dispute → ignored.
- Double dispute on the same tx → second is ignored.
- Withdrawal exceeding available → rejected, balances unchanged.
- Dispute on a withdrawal → correct sign, no negative `held`.
- Chargeback on a non-disputed tx → ignored (held not driven negative).
- Dispute/resolve/chargeback with a mismatched `client` → ignored.
- Accumulation/rounding test that would fail under `f32` but passes under the chosen decimal type.