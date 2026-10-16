# Roadmap — second pass

Derived from a re-read of the spec (`Rust Coding Challenge.md`) against the current implementation. The first pass (dispute state machine, missing-tx handling, insufficient-funds guard, decimal type, client-mismatch validation, parser robustness, deterministic output ordering) is implemented and tested. This pass lists the **remaining** spec-compliance gaps, ordered by impact. Nothing here is implemented yet.

## 1. Use the spec integer types: `client: u16`, `tx: u32` — DONE

The spec states the client column is a valid **u16** client ID and tx is a valid **u32** transaction ID (and repeats "transaction IDs are valid u32 values" as an efficiency hint). The code previously used `u64` for both.

- ✅ Changed `client` to `u16` and `tx` to `u32` across `input.rs` (`InputCsv`), `engine.rs` (`TxState.client`, `accounts: BTreeMap<u16, _>`, `txs_amounts: BTreeMap<u32, _>`, every `deposit`/`withdrawal`/`dispute`/`resolve`/`chargeback` signature, `iter_accounts` return type), `main.rs` (`OutputCsv.client`), and `error.rs` (`culprit_field` re-validates client as `u16` and tx as `u32` so the culprit column is flagged correctly on out-of-range values).
- ✅ `read_line_at`'s `byte_start: u64` is kept — it is a file byte offset from `csv::Position::byte()`, not an id.
- ✅ Out-of-range ids are now rejected at deserialize time (`field 1: number too large to fit in target type`) and flow through the existing ariadne path, which underlines the offending `client`/`tx` token. No new error-handling code was needed.
- Tests: `tests/nok/out_of_range_client.csv` (client `70000` > `u16::MAX` → rejected, `client` field underlined, run continues and exits 0). All 33 e2e snapshots pass; existing snapshots are byte-identical (Debug/CSV serialization of small ids is the same for u16/u32/u64).

## 2. Chargeback is the final state of the *dispute*, not the transaction — no change needed (DONE)

Spec: *"A chargeback is the final state of a dispute and represents the client reversing a transaction."* The "final state" is the **dispute's** lifecycle ending (`dispute → resolve | chargeback`), **not** the transaction becoming permanently non-disputable. The engine already implements this: `chargeback` concludes the dispute by returning the tx to a non-disputed state and freezing the account; a later `resolve`/`chargeback` on that tx is ignored because the tx is no longer `InDispute` — the *dispute* is final and cannot be continued.

- ❌ Do **not** add a terminal `TxStatus::ChargedBack` variant. An earlier draft of this item proposed one by misreading "final state of a dispute" as "final state of the transaction." The spec says the former. A terminal tx status would be stricter than the spec requires, and contradicts the first-pass roadmap (item 1), which already recorded: *"After chargeback, the tx returns to Normal. The spec does not require a terminal tx state; it only requires the account to be locked on chargeback."*
- The residual concern — a *new* `dispute` opened on an already-charged-back tx would re-hold funds — is real **only** if frozen accounts do not reject operations. That is item 3's scope (frozen-account semantics), not a tx-state concern. With item 3's bank-like freeze, the locked account blocks the re-dispute at the account level before any tx lookup. The protection belongs on the account, not the tx.
- No new tests for a "terminal tx" behavior — that behavior is not spec-required. `tests/ok/dispute_state/chargeback_not_disputed.csv` already covers "chargeback/resolve on a tx not under dispute → ignored."

## 3. Define frozen/locked account semantics — TODO

Spec: *"If a chargeback occurs the client's account should be immediately frozen."* The spec does not define what frozen means operationally. `Account.locked` is set on chargeback but never read — deposits, withdrawals, disputes, resolves, and chargebacks all still process on a locked account. The spec invites bank-like assumptions ("think an ATM or a transaction processor").

- ✅ Treat `locked == true` as frozen: `deposit`, `withdrawal`, `dispute`, `resolve`, and `chargeback` on a locked account are ignored with a `WARN` and no balance change. (A frozen ATM account does not accept new activity.)
- ✅ Record this assumption in the README, replacing the current "this one seems wrong" note.
- Tests: `tests/ok/locked/{deposit_after, withdrawal_after, dispute_after}.csv`.
- Open question (decide before implementing): should a `chargeback` on a *different* tx of an already-locked account still be allowed to move `held`/`available`? Bank-like answer: no — once frozen, no movement at all.

## 4. Reject negative amounts — TODO

The spec's overview is explicitly about fraud ("a malicious actor may try to deposit fiat funds, purchase and withdraw BTC, and then reverse their fiat deposit"). A negative `deposit` would *decrease* `available`, and a negative `withdrawal` would *increase* it — both are dangerous and not something a bank permits. `rust_decimal::Decimal` happily accepts negatives and the engine applies them unchecked.

- ✅ In `InputCsv::validate` (or the engine), reject `amount < 0` for `deposit`/`withdrawal` with an ariadne report on the `amount` field (field index 3) and skip the row.
- Why: safety; the spec scores "are you doing something dangerous? tell us why."
- Tests: `tests/nok/negative_deposit.csv`, `tests/nok/negative_withdrawal.csv`.

## 5. Handle top-level I/O errors without panicking — TODO

`Input::from_args` does `args().nth(1).unwrap()`, `from_path(&path).unwrap()`, and `byte_headers().unwrap()`; `write_output` does `serialize(...).unwrap()`. On a missing argument, a missing/unreadable file, or a malformed header, the program panics with a Rust backtrace instead of a clean error. The row-level ariadne machinery is already graceful; the top level is not.

- ✅ Return a `Result` from `Input::from_args` (and `write_output`); on error, print a short human message to stderr and exit non-zero. Keep the existing row-skip behavior for per-record errors untouched.
- Why: the spec scores error handling explicitly ("how are you handling errors?"). A panic on a bad CLI arg is the kind of "dangerous" thing to remove.
- Tests: `tests/cli/missing_arg` and `tests/cli/missing_file` (these may need a small harness variant since they don't take a `.csv` glob path — or assert via a direct `process::Command` in a unit test rather than the snapshot glob).

## 6. Output precision — reconsider (low / spec-contradictory)

Spec (Precision): *"You can assume a precision of four places past the decimal and **should output values with the same level of precision**."* The implementation serializes `rust_decimal` in its natural scale-preserving form (`1.0` → `1`, `5.0` → `5`, `0.1×10` → `1.0`), i.e. not 4 places. The first pass (item 8) decided this was acceptable because the spec also says "displaying decimals for round values do not matter" and treats `2` == `2.0`.

- This is the one place the spec contradicts itself. Strict automated scoring that diffs against 4-place output could flag it. If we want strict compliance, force 4-decimal output (e.g. `Decimal::round_dp(4)` + serialize with fixed scale, or format with `{:>.*}`, 4).
- Recommendation: leave as-is unless the scorer requires 4 places; the "do not matter" clause is explicit. Listed here for completeness, not for action.

## 7. Don't create phantom accounts for rejected withdrawals — TODO (minor)

`withdrawal` calls `self.accounts.entry(id).or_default()` *before* the funds check, so a rejected withdrawal on a never-before-seen client creates a `0,0,0,false` row in the output. A bank does not open an account for a failed withdrawal.

- ✅ Move the account lookup so a rejected withdrawal on a non-existent client does not create an entry (or only `entry` once the funds check passes).
- Tests: `tests/ok/withdrawal/rejected_new_client.csv` (assert the client does not appear in output).

## 8. Guard against duplicate tx ids — TODO (low / defensive)

Spec: *"transaction IDs (tx) are globally unique."* A duplicate `deposit` tx id currently credits `available` twice but overwrites the single `TxState`, so the record no longer matches the balance and a later dispute would only reverse one of the two credits. This "can't happen" per the spec, but it silently corrupts state if it does.

- ✅ On `deposit`/`withdrawal`, if `txid` already exists in `txs_amounts`, emit a `WARN` and ignore the duplicate (no balance change, no overwrite). This makes the invariant self-enforcing rather than assumed.
- Tests: `tests/ok/duplicate_tx/{deposit, withdrawal}.csv`.

## Notes / accepted tradeoffs (no action)

- **Deterministic output ordering** is already handled: `accounts` is a `BTreeMap`, so `iter_accounts()` yields clients in ascending id order. (The spec says row ordering does not matter, but determinism is required for reproducible snapshot tests and stable automated scoring.)
- **Retaining all tx states indefinitely**: disputes can reference any past tx, so `txs_amounts` must be retained for the whole run. This is inherent to the spec's referential model, not a leak. Streaming the CSV (not slurping) is already done via `csv::Reader::byte_records()` and the lazy one-line error reader.
- **Variable output scale**: see item 6 — accepted unless strict 4-place scoring is required.