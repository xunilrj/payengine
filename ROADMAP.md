# Spec compliance report

Audit of the current tree (`master`, HEAD `63029fb`) against the challenge spec.
Nothing was changed; findings come from reading the code and running the binary.

**Verdict: not 100% compliant.** Two issues will very likely break the automated
scoring. Everything else either matches the spec or is a documented interpretation.

Baseline checks:

| Check | Result |
|---|---|
| `cargo build` | OK |
| `cargo fmt --check` | OK |
| `cargo test` | OK (27 unit + 46 e2e) |
| `cargo clippy --all-targets` | 6 warnings (none affect behavior) |
| Spec example input → spec example output | OK |
| No forbidden branding in tree or git history | OK (no matches) |
| Spec file not committed | OK |

---

## P0: breaks the CLI contract / automated scoring

### 1. ~~Log output goes to **stdout** and corrupts `accounts.csv`~~ (FIXED: `setup_log` now writes to stderr; snapshot separation still open)

`src/log.rs` uses `tracing_subscriber::fmt()` with no writer set, and that writes
to **stdout** by default. The default filter is `info`, so every `warn!` in
`src/engine.rs` (tx not found, insufficient funds, frozen account, client
mismatch, duplicate tx, and so on) ends up in the CSV output:

```
$ cargo run -- three.csv 2>/dev/null
2026-10-17T12:56:17.907308Z  WARN payengine::engine: Tx not found txid=99
client,available,held,total,locked
1,10,0,10,false
```

Any input that hits an "ignore it" path (very likely in the grader's samples)
gives an unparseable output. The README says "All diagnostics go to stderr",
which is not true for tracing logs.

The tests miss it because `tests/e2e.rs:46` uses `stderr_to_stdout()`, so the
snapshots cannot tell the two streams apart.

**Fix:** `.with_writer(std::io::stderr)` in `setup_log`. Optionally default the
filter to `off`/`error` so ANSI-coloured warnings don't spam the console. Snapshot
stdout and stderr separately so a regression can't hide again.

### 2. dispute/resolve/chargeback rows with 3 columns are rejected

The csv reader is not `flexible`, so a row like `dispute,1,1` (no trailing
comma) fails with *"found record with 3 fields, but the previous record has 4
fields"*. It is then skipped, so the dispute never happens.

The spec's tables only show an empty amount cell, so the exact CSV form is
ambiguous. Omitting the trailing comma is very common in real data and in
graders' sample files, though. Every fixture in `tests/` uses the trailing-comma
form, so this is untested.

**Fix:** `ReaderBuilder::flexible(true)`. `amount` is already `Option<Decimal>`,
so a missing column deserializes to `None`. Add `ok/` fixtures with 3-column rows.

---

## P1: correctness / safety

### 3. Silent precision loss on overflow

`Decimal`'s `+=`/`-=` does not panic near the 96-bit limit. It silently rescales
and drops digits:

```
deposit 7922816251426433759354395033.5  (x2)
→ available = 15845632502852868000000000000   (exact: 15845632502852867518708790067)
```

The total is wrong, and nothing reports it. Very unlikely with real data, but the
spec scores "Safety" explicitly, and a bank would refuse the operation.

**Fix:** use `checked_add`/`checked_sub` in the engine and return a new
`EngineError::Overflow`, which ignores the tx and logs a warning. Add a test.

### 4. Amounts with more than 4 decimal places are accepted as-is

`1.123456` is stored and printed as `1.123456`. The spec says to assume 4 places
and "output values with the same level of precision". The README documents
emitting more than 4 places as intentional. Inputs should never have more than 4,
so the grader shouldn't see this, but you still need a policy: reject, round
(banker's / `MidpointAwayFromZero`), or keep. Rejecting or rounding to 4 on input
is the safer reading for a bank.

---

## P2: spec ambiguities (already documented in the README; listed for completeness)

These are deliberate interpretations. They don't break the spec, but a reviewer
may disagree with them:

- **Locked accounts ignore everything**, including deposits. Defensible, and documented.
- **Disputed withdrawals.** Nothing moves to `held`, and a chargeback *credits*
  `available`. The spec's chargeback text says held and total "should decrease".
  The deviation is documented. A grader sample that disputes a withdrawal would
  disagree with any implementation, so the risk is low.
- **Client-id mismatch on dispute/resolve/chargeback is ignored.** Not in the spec,
  but sensible and documented.
- **A charged-back tx goes back to `Normal`.** It can then be disputed again, but
  only if the account weren't locked, and it always is. Fine in practice.
- **A dispute can make `available` negative.** Allowed by the spec arithmetic, and documented.
- **`-0.0` is accepted as a deposit/withdrawal amount, and so is scientific notation (`1e2`).**
  Harmless, but undocumented.

---

## P3: efficiency / maintainability (scoring categories, not hard requirements)

- **Memory:** every deposit and withdrawal stays in a `BTreeMap<u32, TxState>`
  for the whole run, rejected ones included. At worst that is about 4 billion
  entries. The README discusses this and gives reasons for leaving it. A
  `HashMap` would be faster, since only accounts need sorted output. Storing
  rejected withdrawals is only needed for duplicate-id detection.
- **Concurrency (the "thousands of TCP streams" question):** not addressed in the
  code or the README. A short README paragraph on how this would shard (for
  example per client id across workers, with the tx map behind the same shard)
  would answer the reviewer's prompt.
- **Clippy:** 6 warnings (`collapsible_if` in `error.rs`, `repeat().take()`,
  needless borrow, `while`-loop that could be a `for`, useless `into_iter`,
  `&PathBuf` instead of `&Path`). Fixing them is cheap polish.
- **`.expect()` in `main.rs`:** the deposit/withdrawal amount invariant is
  enforced by `InputCsv::validate`, but the type system doesn't enforce it. The
  README discusses this. You could instead convert to a `Transaction` enum after
  validation, which doesn't need serde support.
- **Typos in comments/README:** "garantees", "nothign", "agains", "callign",
  "enterily", "frozend", "wit hthe", "chargerback", "transaxtion".
- **ROADMAP.md tracked in history:** the README explains it's intentional. Remember
  to remove it, or keep it deliberately, before submitting.

---

## Suggested order

1. Logs → stderr (P0 #1) and separate stdout/stderr in snapshots.
2. `flexible(true)` plus 3-column fixtures (P0 #2).
3. Checked arithmetic (P1 #3).
4. Decide the policy for more than 4 decimals (P1 #4).
5. README note on concurrency; clippy clean-up; typos.
