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
