# Roadmap

What is still open after the final spec review. Each item was checked by running
the release binary.

## Correctness

### 1. Arithmetic overflow: only the output total is left

The engine now uses `checked_add`/`checked_sub`: a transaction that would
overflow a balance is ignored (`balance would overflow`, logged at `warn`) and
the run continues.

Still open: when `available` and `held` both fit but their sum does not, writing
the output stops with "total of client N does not fit in a decimal" and exit 1,
with no output. Avoiding it would need the engine to also check
`available + held` on every change.

## Documentation

### 3. The README TLDR has no known limitations

Item 1 (if not fixed), and the memory cost: every deposit and withdrawal
stays in memory for possible disputes, about 50 bytes each.

### 4. The Testing section is missing `tests/blocks/`

The fixture list doesn't mention `tests/blocks/`, which runs inputs with tiny
blocks to exercise block splitting.

## Suggested order

1. #1: only if the remaining case matters.
2. #3 and #4.
