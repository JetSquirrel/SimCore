#!/usr/bin/env bash
# End-to-end simu-vs-SimPy comparison: build the Rust runner, set up a Python
# venv, run the correctness + performance harnesses, and write compare/REPORT.md.
#
# Usage:
#   compare/run_comparison.sh                 # full run
#   compare/run_comparison.sh --scale 0.1     # quick smoke run (fewer seeds)
#   compare/run_comparison.sh --no-perf       # correctness only
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COMPARE_DIR="$REPO_ROOT/compare"
VENV="$COMPARE_DIR/.venv"

echo "==> Building Rust comparison runner (release)"
cargo build --release --example compare --manifest-path "$REPO_ROOT/Cargo.toml"

if [ ! -d "$VENV" ]; then
  echo "==> Creating Python venv at $VENV"
  python3 -m venv "$VENV"
fi
# shellcheck disable=SC1091
source "$VENV/bin/activate"
echo "==> Installing Python deps"
pip install --quiet --upgrade pip
pip install --quiet -r "$COMPARE_DIR/requirements.txt"

echo "==> Running comparison harness"
PYTHONPATH="$COMPARE_DIR/harness:$COMPARE_DIR/models" \
  python "$COMPARE_DIR/harness/report.py" "$@"
