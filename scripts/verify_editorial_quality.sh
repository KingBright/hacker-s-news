#!/bin/sh
# Deterministic local checks only; does not call models, publish or run schedules.
set -eu
cd "$(dirname "$0")/.."
python3 -B -m unittest discover -s skills/freshloop-content-agent/scripts -p 'test_*.py'
python3 -B -m unittest discover -s scripts -p 'test_freshloop_agent_runtime.py'
git diff --check -- skills/freshloop-content-agent scripts/verify_editorial_quality.sh
