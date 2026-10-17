# Roadmap: final spec review

Review of HEAD `8a596b3` against the challenge spec. Every finding below was
checked by running the release binary, not only by reading the code.

## What already holds

| Check | Status |
|---|---|
| `cargo build`, `cargo fmt --check` | OK |
| `cargo test` | OK: 34 unit, 58 end-to-end |
| CLI: the input path is the only argument, output goes to stdout | OK |
| Spec example input → spec example output | OK |
| Logs and diagnostics go to stderr only, stdout is pure CSV | OK |
| Whitespace, up to 4 decimals, rows without the amount column | OK |
| Deposits, withdrawals, disputes, resolves, chargebacks, locking | OK, interpretations documented in the README TLDR |
| Missing tx / tx not in dispute → ignored | OK |
| Empty file or header only → header row, exit 0 | OK |
| Output identical for any shard count, parser count and block size | OK (checked on 1M rows and every fixture with tiny blocks) |
| No branding in the tree or git history | OK |
| No verbatim spec text in the README | OK |

---

## P1: correctness gaps

### 1. Duplicate tx ids across clients are accepted with more than one shard

```
deposit,1,1,10
deposit,2,1,10
```

With 1 shard the second deposit is rejected as a duplicate. With 2 or more shards
(the default is half the cores) both clients are credited 10. Tx ids are globally
unique in the spec, and the result now depends on the shard count. The README
TLDR says duplicated tx ids are ignored, which is only true within a shard.

**Fix:** check tx ids on the main thread, which receives every row in file
order before routing them: a set of seen ids (a bitset over `u32` is a fixed 512 MB,
a `HashSet` grows with the input). Then decide whether a rejected deposit or
withdrawal still uses up its id, and keep the engine's per-shard check or drop it.

### 2. Arithmetic overflow aborts the whole run with no output

| Input | Result |
|---|---|
| Two deposits adding up past `Decimal::MAX` | a shard panics (`Addition overflowed`), exit 1, empty stdout |
| `available + held` past `Decimal::MAX` when writing the output | the main thread panics, exit 101 |

Only reachable with hostile input, but one bad row loses every account's output.

**Fix:** `checked_add`/`checked_sub` in the engine, rejecting the transaction
like any other business error (logged at `warn`), and a checked `total` in
`write_output`. No rounding detection: that was tried and rolled back.

### 3. Quoted fields containing a newline depend on the block size

```
deposit,1,2,"5
"
```

This is valid CSV. When it fits in one block it is parsed correctly. When a block
boundary falls inside the quotes (forced here with `PAYENGINE_BLOCK_SIZE=8`), the
row is split: a bogus "unknown transaction type" error and one more skipped row.
With 64 KiB blocks this is rare, but the output then depends on where blocks fall.
The spec data never contains such fields.

**Fix, either:** track quote parity when looking for a block's line boundaries
(a fast search for `"` and `\n`), or document it as unsupported in the README TLDR,
which currently doesn't mention it.

---

## P2: documentation

### 4. ~~The README doesn't document the configuration~~ (FIXED: new "Configuration" section)

`PAYENGINE_SHARDS`, `PAYENGINE_PARSERS`, `PAYENGINE_BLOCK_SIZE`,
`PAYENGINE_SORT_OUTPUT` and `RUST_LOG` (default `error`, `warn` shows every ignored
transaction) are not mentioned anywhere in the README. A reviewer has no way to
know that output rows are unsorted by default, or how to see rejected transactions.

### 5. TLDR statements that are no longer fully accurate

- Duplicate tx ids "are ignored": not across shards (see #1).
- Nothing mentions that output rows are not sorted, or the known limitations
  (#1–#3, about 50 bytes of memory per stored transaction).

### 6. Testing section

The fixture list is missing `tests/blocks/` (block splitting), and `tests/ok/empty/`
is new since it was written.

---

## P3: polish

### 7. Unused `serde-str` feature

`rust_decimal` still enables `serde-str`, but input no longer goes through serde
(only the output does). It can be removed from `Cargo.toml`. Optionally, the output
could drop serde entirely with `write_record`, removing the `serde` dependency.

### 8. Clippy (7 warnings)

- `tests/e2e.rs`: 3 uses of `str::from_utf8` that need Rust 1.87 while `rust-version`
  is 1.85 (use `std::str::from_utf8`), a needless `.into_iter()`, a `while let`
  that could be a `for`, `&PathBuf` instead of `&Path`.
- `src/error.rs`: `repeat().take()` → `repeat_n()`.

### 9. Header matching is strict

A header with capitalised names (`Type,Client,…`) or a file without a header row
stops with "header row has no `type` column", exit 1. The spec always has a
lowercase header, so this is acceptable; a case-insensitive match would be a
one-line change if wanted. A UTF-8 BOM before the header works.

---

## Already decided, not open

- Amounts with more than 4 decimal places are kept as-is: documented in the README
  ("Output precision contradiction").
- Rounding near the `Decimal` limit: accepted, see the devlog.
- Disputes on withdrawals, locked accounts ignoring everything, zero amounts,
  negative `available` after a dispute: documented interpretations.

## Suggested order

1. #1 and #2: the only items that can make the output wrong or empty.
2. #3: decide between supporting and documenting.
3. #4–#6: what a reviewer reads first.
4. #7–#9: polish.
