#!/usr/bin/env bash
# Adds one release's memory numbers to memory.json on the `metrics` branch,
# redraws memory.svg from it and pushes. Re-running for the same version
# replaces that version's entry. Needs GH_TOKEN and REPO.
set -euo pipefail

run=$1
scripts=$(cd "$(dirname "$0")" && pwd)
run=$(realpath "$run")
work=$(mktemp -d)
url="https://x-access-token:${GH_TOKEN}@github.com/${REPO}.git"

rc=0
git ls-remote --exit-code --heads "$url" metrics >/dev/null 2>&1 || rc=$?
if [ "$rc" = 0 ]; then
  git clone --quiet --depth 1 --branch metrics "$url" "$work"
elif [ "$rc" = 2 ]; then
  git init --quiet -b metrics "$work"
  git -C "$work" remote add origin "$url"
else
  echo "could not reach $REPO (ls-remote exit $rc)" >&2
  exit 1
fi

python3 -I - "$work/memory.json" "$run" <<'PY'
import json, os, sys
path, run_path = sys.argv[1:]
data = {"schema": 1, "runs": []}
if os.path.exists(path):
    with open(path, encoding="utf-8") as f:
        data = json.load(f)
with open(run_path, encoding="utf-8-sig") as f:
    run = json.load(f)
data["runs"] = [r for r in data["runs"] if r["version"] != run["version"]] + [run]
with open(path, "w", encoding="utf-8") as f:
    json.dump(data, f, indent=2)
    f.write("\n")
PY

python3 -I "$scripts/render-memory-chart.py" "$work/memory.json" "$work/memory.svg"

cat > "$work/README.md" <<'MD'
# Snapline metrics

Data branch. CI writes it on every release; do not edit by hand.

- `memory.json` holds one entry per release: raw private working set, private bytes and working set per stage, plus the commit, build profile and runner details. It is the source of truth.
- `memory.svg` is drawn from `memory.json` and embedded in the main README.
MD

cd "$work"
git config user.name "github-actions[bot]"
git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
git add memory.json memory.svg README.md
if git diff --cached --quiet; then
  echo "nothing to commit"
  exit 0
fi
version=$(python3 -I -c "import json,sys;print(json.load(open(sys.argv[1],encoding='utf-8-sig'))['version'])" "$run")
git commit --quiet -m "Record memory use for $version"
for attempt in 1 2 3; do
  if git push --quiet origin HEAD:metrics; then
    echo "pushed memory data for $version"
    exit 0
  fi
  git pull --quiet --rebase origin metrics
done
echo "push to metrics failed" >&2
exit 1
