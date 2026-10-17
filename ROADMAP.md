# Roadmap

What is still open after the final spec review. Each item was checked by running
the release binary.

## Correctness

### 1. The output total can overflow

When `available` and `held` both fit in a `Decimal` but their sum does not,
writing the output stops with "total of client N does not fit in a decimal" and
exit 1, with no output. Only reachable with hostile input. Avoiding it would need
the engine to also check `available + held` on every change.

## Documentation

### 2. The README TLDR has no known limitations

Item 1 (if not fixed), and the memory cost: every deposit and withdrawal stays in
memory for possible disputes, about 50 bytes each.

## Suggested order

1. #1: only if the remaining case matters.
2. #2.
