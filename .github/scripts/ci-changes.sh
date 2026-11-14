#!/usr/bin/env bash
# Decides which CI jobs a run needs. Outputs:
#   code  - false when only docs changed, so build jobs can be skipped
#   test  - false when the lint/clippy/test/ui-e2e jobs can be skipped (docs only,
#           or a push to main whose exact tree a green PR run already tested)
#   tree  - git tree hash of the commit under test
set -euo pipefail

out() { echo "$1=$2" >> "$GITHUB_OUTPUT"; echo "$1=$2"; }
note() { echo "$*" >> "$GITHUB_STEP_SUMMARY"; echo "$*"; }

tree=$(git rev-parse 'HEAD^{tree}')
out tree "$tree"

base=''
if [ "$EVENT_NAME" = pull_request ]; then
  base=$(git rev-parse --verify -q 'HEAD^1' || true)
elif [ -n "${BEFORE:-}" ] && [ "$BEFORE" != 0000000000000000000000000000000000000000 ]; then
  if git fetch --no-tags --depth=1 -q origin "$BEFORE"; then
    base=$BEFORE
  fi
fi

if [ -z "$base" ]; then
  note 'Could not work out what changed, so every job runs.'
  out code true
  out test true
  exit 0
fi

is_doc() {
  case "$1" in
    docs/* | design/v2/* | *.md | LICENSE) return 0 ;;
  esac
  return 1
}

mapfile -t files < <(git diff --name-only "$base" HEAD)
code=false
manifests=false
for f in "${files[@]}"; do
  is_doc "$f" || code=true
  case "$f" in
    Cargo.lock | Cargo.toml | */Cargo.toml | rust-toolchain.toml | .github/*) manifests=true ;;
  esac
done

out code "$code"
if [ "$code" = false ]; then
  note 'Only docs changed: build and test jobs are skipped.'
  out test false
  exit 0
fi

# Reuse a green PR run on main. The merge commit's tree must be byte-identical to the
# tree the PR run tested. Dependency, toolchain and workflow changes always run in full
# so the main-only Rust caches are refreshed and CI edits are exercised.
if [ "$EVENT_NAME" != push ] || [ "$manifests" = true ]; then
  out test true
  exit 0
fi

mapfile -t heads < <(gh api "repos/$REPO/commits/$SHA/pulls" --jq \
  ".[] | select(.merged_at != null and .base.ref == \"main\" and .merge_commit_sha == \"$SHA\") | .head.sha")

for head in "${heads[@]}"; do
  mapfile -t runs < <(gh api \
    "repos/$REPO/actions/workflows/ci.yml/runs?event=pull_request&status=success&head_sha=$head&per_page=10" \
    --jq '.workflow_runs[].id')
  for run in "${runs[@]}"; do
    dir="$RUNNER_TEMP/tested-$run"
    if gh run download "$run" -R "$REPO" -n tested-tree -D "$dir" 2>/dev/null &&
      [ "$(tr -d '[:space:]' < "$dir/tree.txt")" = "$tree" ]; then
      note "Tree $tree was already tested by green PR run $run, so lint, clippy, tests and UI e2e are skipped."
      out test false
      exit 0
    fi
  done
done

note 'No green PR run tested this exact tree, so every job runs.'
out test true
