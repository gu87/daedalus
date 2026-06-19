#!/usr/bin/env bash
# smoke-phase5.sh — Phase 5 full-chain acceptance
set -euo pipefail

echo "=== Phase 5 Smoke ==="

failures=0
run() {
    local desc="$1"; shift
    printf "  %-60s " "$desc"
    if "$@" > /dev/null 2>&1; then
        echo "OK"
    else
        echo "FAIL"
        failures=$((failures + 1))
    fi
}

# 1. DAEDALUS.md prompt injection
run "prompt: daedalus between soul and memory" \
    cargo test --test prompt prompt_builder_new_includes_daedalus_between_soul_and_memory

# 2. SQLite schema (read-only, via Rust tests)
run "db_registry: migration v3 + schema" \
    cargo test --test db_registry

# 3. Durable Execution
run "durable: 15 tests" \
    cargo test --test durable

# 4. Pipeline state machine
run "pipeline status: 11 tests" \
    cargo test --lib pipeline

# 5. Pipeline DB CRUD
run "pipeline_db: 6 tests" \
    cargo test --test pipeline_db

# 6. Pipeline daemon wiring
run "pipeline_daemon: 4 tests" \
    cargo test --test pipeline_daemon

# 7. Clippy
run "clippy" \
    cargo clippy --workspace -- -D warnings

echo ""
if [ "$failures" -eq 0 ]; then
    echo "=== Phase 5 smoke: ALL PASSED ==="
else
    echo "=== Phase 5 smoke: $failures FAILED ==="
    exit 1
fi
