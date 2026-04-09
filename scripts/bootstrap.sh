#!/usr/bin/env bash
set -euo pipefail

echo "Optima bootstrap"
echo "- Start rust kernel: make run-kernel"
echo "- Start go orchestrator: make run-orchestrator"
echo "- Test orchestrator: curl -s http://localhost:8080/v1/schedules"
