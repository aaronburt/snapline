# Snapline metrics

Data branch. CI writes it on every release; do not edit by hand.

- `memory.json` holds one entry per release: raw private working set, private bytes and working set per stage, plus the commit, build profile and runner details. It is the source of truth.
- `memory.svg` is drawn from `memory.json` and embedded in the main README.
