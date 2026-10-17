# Roadmap

What is still open after the final spec review. Each item was checked by running
the release binary.

## Documentation

### 1. The README TLDR has no known limitations

- An account whose `available + held` does not fit in a `Decimal` is left out of
  the output, with a warning on stderr (hostile input only).
- Memory: every deposit and withdrawal stays in memory for possible disputes,
  about 50 bytes each.
