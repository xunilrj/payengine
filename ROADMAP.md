# Roadmap

What is still open after the final spec review. Each item was checked by running
the release binary.

## Correctness

### 1. Arithmetic overflow aborts the whole run with no output

| Input | Result |
|---|---|
| Two deposits adding up past `Decimal::MAX` | a shard panics (`Addition overflowed`), exit 1, empty stdout |
| `available + held` past `Decimal::MAX` when writing the output | the main thread panics, exit 101 |

Only reachable with hostile input, but one bad row loses every account's output.

**Fix:** `checked_add`/`checked_sub` in the engine, rejecting the transaction
like any other business error (logged at `warn`), and a checked `total` in
`write_output`. No rounding detection: that was tried and rolled back.

### 2. Quoted fields containing a newline depend on the block size

```
deposit,1,2,"5
"
```

This is valid CSV. When it fits in one block it is parsed correctly. When a block
boundary falls inside the quotes (forced with `PAYENGINE_BLOCK_SIZE=8`), the row
is split: a bogus "unknown transaction type" error and one more skipped row.
With 64 KiB blocks this is rare, and the spec data never contains such fields.

**Fix, either:** track quote parity when looking for a block's line boundaries
(a fast search for `"` and `\n`), or document it as unsupported.

## Documentation

### 3. The README TLDR has no known limitations

Items 1 and 2 (if not fixed), and the memory cost: every deposit and withdrawal
stays in memory for possible disputes, about 50 bytes each.

### 4. The Testing section is missing `tests/blocks/`

The fixture list doesn't mention `tests/blocks/`, which runs inputs with tiny
blocks to exercise block splitting.

## Suggested order

1. #1: the only item that can make the output empty.
2. #2: decide between supporting and documenting.
3. #3 and #4.
