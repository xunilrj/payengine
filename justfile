# The checks a CI runs: formatting, clippy with warnings as errors, tests,
# documentation, and a dependency audit (vulnerabilities, licenses, sources).
# Builds with the committed Cargo.lock. Installs cargo-deny if missing.
[group('CI')]
ci:
    #!/usr/bin/env bash
    set -euo pipefail
    command -v cargo-deny >/dev/null || cargo install --locked cargo-deny
    cargo fmt --all --check
    cargo clippy --all-features --all-targets -- -D warnings
    # Every target except benches: --all-targets would run the criterion
    # benchmarks here. Clippy above still compiles them.
    cargo test --all-features --lib --bins --tests --examples --locked
    RUSTDOCFLAGS="-D warnings" cargo doc --all-features --no-deps --locked
    cargo deny check

# Run every benchmark in the repo: the in-process engine and sharded benches,
# the peak-RSS measurement per size, and the hyperfine CLI comparison.
# Usage: just bench-all
[group('Benchmarks and profiling')]
bench-all:
    #!/usr/bin/env bash
    set -euo pipefail
    just bench
    just bench-memory
    just bench-cli

# Usage: just bench [filter]
[group('Benchmarks and profiling')]
bench filter="":
    cargo bench --bench engine -- {{filter}}

# Peak memory (max RSS) of each benchmark. Each size runs in its own process,
# so the peak belongs to that size only. Usage: just bench-memory [sizes]
[group('Benchmarks and profiling')]
bench-memory sizes="10k 100k 1M 10M 100M":
    #!/usr/bin/env bash
    set -euo pipefail
    # Run the bench binary directly: wrapping `cargo` would report the
    # largest of cargo's and the benchmark's peaks.
    exe=$(cargo bench --bench engine --no-run --message-format=json \
        | grep -oE '"executable":"[^"]+/deps/engine-[^"]+"' | cut -d'"' -f4)
    case "$(uname)" in
        Darwin) flag=-l; pattern='maximum resident set size'; unit=1 ;;
        *)      flag=-v; pattern='Maximum resident set size'; unit=1024 ;;
    esac
    log=$(mktemp)
    printf '%-10s %12s\n' bench "peak RSS"
    for size in {{sizes}}; do
        /usr/bin/time "$flag" "$exe" "$size" --bench > /dev/null 2> "$log"
        rss=$(grep "$pattern" "$log" | grep -oE '[0-9]+' | head -1)
        printf '%-10s %9d MB\n' "$size" $(( rss * unit / 1024 / 1024 ))
    done
    rm -f "$log"

# Usage: just bench-cli [rows]
[group('Benchmarks and profiling')]
bench-cli rows="10000000":
    #!/usr/bin/env bash
    set -euo pipefail
    command -v hyperfine >/dev/null || cargo install --locked hyperfine
    seed=24301
    input="target/bench/transactions_{{rows}}_seed${seed}.csv"
    if [ ! -f "$input" ]; then
        mkdir -p target/bench
        cargo run --release --example generate_csv -- {{rows}} "$seed" "$input"
    fi
    cargo build --release
    exe=target/release/payengine
    export RUST_LOG=error
    hyperfine --warmup 1 \
        -n "1 shard, 1 parser"   "PAYENGINE_SHARDS=1 PAYENGINE_PARSERS=1 $exe $input" \
        -n "4 shards, 4 parsers" "PAYENGINE_SHARDS=4 PAYENGINE_PARSERS=4 $exe $input" \
        -n "8 shards, 8 parsers" "PAYENGINE_SHARDS=8 PAYENGINE_PARSERS=8 $exe $input" \
        -n "default"             "env -u PAYENGINE_SHARDS -u PAYENGINE_PARSERS $exe $input"

# Profile the CLI with samply on the same generated input as bench-cli, and
# print the command that opens the saved profile.
# Installs samply if missing. Usage: just profile-cli [rows]
[group('Benchmarks and profiling')]
[macos]
profile-cli rows="10000000":
    #!/usr/bin/env bash
    set -euo pipefail
    command -v samply >/dev/null || cargo install --locked samply
    seed=24301
    input="target/bench/transactions_{{rows}}_seed${seed}.csv"
    if [ ! -f "$input" ]; then
        mkdir -p target/bench
        cargo run --release --example generate_csv -- {{rows}} "$seed" "$input"
    fi
    cargo build --profile profiling
    profile="target/profiling/profile_{{rows}}.json.gz"
    RUST_LOG=error samply record --save-only -o "$profile" \
        target/profiling/payengine "$input" > /dev/null
    echo "To see run: samply load $profile"

# Accept the changed snapshot tests as the new expected output.
[group('Development helpers')]
accept-tests:
    cargo t || true
    cargo insta accept

# Search the entire repo history (commits, patches, deleted lines, file names, refs, working tree) for a term.
# Usage: just search-history <term>
[group('Development helpers')]
search-history term:
    #!/usr/bin/env bash
    set -euo pipefail
    q="{{term}}"
    export GIT_PAGER=cat
    echo "== commit messages =="
    git log --all --oneline -i --grep "$q" || true
    echo "== patch content (pickaxe) =="
    git log --all -p -S "$q" --pickaxe-regex || true
    echo "== deleted lines =="
    git log --all --diff-filter=D -p | grep -i "$q" || true
    echo "== file names in history =="
    git log --all --name-only --pretty=format: | grep -i "$q" || true
    echo "== refs =="
    git for-each-ref --format='%(refname)' | grep -i "$q" || true
    echo "== working tree =="
    grep -ri "$q" . --exclude-dir=.git || true
    echo "== done =="