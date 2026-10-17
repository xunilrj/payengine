accept-tests:
    cargo t || true
    cargo insta accept

# Search the entire repo history (commits, patches, deleted lines, file names, refs, working tree) for a term.
# Usage: just search-history <term>
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

bench filter="":
    cargo bench --bench engine -- {{filter}}

# Peak memory (max RSS) of each benchmark. Each size runs in its own process,
# so the peak belongs to that size only. Usage: just bench-memory [sizes]
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
    printf '%-6s %12s\n' size "peak RSS"
    for size in {{sizes}}; do
        /usr/bin/time "$flag" "$exe" "$size" --bench > /dev/null 2> "$log"
        rss=$(grep "$pattern" "$log" | grep -oE '[0-9]+' | head -1)
        printf '%-6s %9d MB\n' "$size" $(( rss * unit / 1024 / 1024 ))
    done
    rm -f "$log"
