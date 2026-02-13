# ════════════════════════════════════════════════════════════════════════════
# Discord Demo (Edgar) - Build, install, and run
# Install just: cargo install just
# ════════════════════════════════════════════════════════════════════════════

install_dir := env_var_or_default("CARGO_HOME", env_var("HOME") + "/.cargo") + "/bin"
bin := "discord-demo"

# ════════════════════════════════════════════════════════════════════════════
# Building
# ════════════════════════════════════════════════════════════════════════════

# Build in debug mode
build:
    cargo build

# Build in release mode
build-release:
    @echo "🔨 Building release..."
    cargo build --release

# Quick type-check (no codegen)
check:
    cargo chec

# ════════════════════════════════════════════════════════════════════════════
# Running
# ════════════════════════════════════════════════════════════════════════════

# Run the bot (debug mode)
run *args:
    RUST_BACKTRACE=1 cargo run -- {{args}}

# Run the bot (release mode)
run-release *args:
    cargo run --release -- {{args}}

# Run with verbose tracing
run-debug *args:
    RUST_LOG=debug RUST_BACKTRACE=1 cargo run -- {{args}}

# Watch and rebuild on changes (requires cargo-watch)
watch:
    cargo watch -x run

# ════════════════════════════════════════════════════════════════════════════
# Installation
# ════════════════════════════════════════════════════════════════════════════

# Install binary to ~/.cargo/bin
install: build-release
    @echo "📦 Installing to {{install_dir}}"
    @cp target/release/{{bin}} {{install_dir}}/edgar
    @echo "✅ Installed as: edgar"

# Uninstall
uninstall:
    @rm -f {{install_dir}}/edgar
    @echo "🗑️  Uninstalled: edgar"

# ════════════════════════════════════════════════════════════════════════════
# Testing
# ════════════════════════════════════════════════════════════════════════════

# Run all tests
test:
    cargo test

# Run tests with output
test-verbose:
    cargo test -- --nocapture

# Run only the REPL parser tests
test-repl:
    cargo test repl -- --nocapture

# ════════════════════════════════════════════════════════════════════════════
# Environment & Setup
# ════════════════════════════════════════════════════════════════════════════

# Show current .env config (redacted)
env:
    @echo "Environment:"
    @echo "  DISCORD_TOKEN:    $(if [ -n \"$DISCORD_TOKEN\" ] || grep -q DISCORD_TOKEN .env 2>/dev/null; then echo 'set'; else echo 'NOT SET'; fi)"
    @echo "  DISCORD_GUILD_ID: $(grep DISCORD_GUILD_ID .env 2>/dev/null | cut -d= -f2 | tr -d '\"' || echo 'NOT SET')"
    @echo "  LLM_BASE_URL:     $(grep LLM_BASE_URL .env 2>/dev/null | cut -d= -f2 | tr -d '\"' || echo 'default: http://localhost:1234/v1')"
    @echo "  LLM_MODEL:        $(grep LLM_MODEL .env 2>/dev/null | cut -d= -f2 | tr -d '\"' || echo 'default: qwen/qwen3-8b')"
    @echo ""
    @echo "Rust:  $(rustc --version)"
    @echo "Python: $(python3 --version 2>/dev/null || echo 'NOT FOUND (needed for PyO3)')"

# Validate that required env vars are configured
preflight:
    #!/bin/bash
    set -e
    echo "🔍 Preflight checks..."
    ERRORS=0

    # Check .env exists
    if [ ! -f .env ]; then
        echo "  ❌ .env file not found — copy from .env.example or create one"
        ERRORS=$((ERRORS+1))
    fi

    # Check DISCORD_TOKEN
    source .env 2>/dev/null || true
    if [ -z "$DISCORD_TOKEN" ]; then
        echo "  ❌ DISCORD_TOKEN not set"
        ERRORS=$((ERRORS+1))
    else
        echo "  ✅ DISCORD_TOKEN"
    fi

    # Check Python (for PyO3)
    if ! python3 --version &>/dev/null; then
        echo "  ❌ python3 not found (required for PyO3 sandbox)"
        ERRORS=$((ERRORS+1))
    else
        echo "  ✅ python3 ($(python3 --version 2>&1 | cut -d' ' -f2))"
    fi

    # Check LLM endpoint
    LLM_URL="${LLM_BASE_URL:-http://localhost:1234/v1}"
    if curl -s --max-time 2 "$LLM_URL/models" >/dev/null 2>&1; then
        echo "  ✅ LLM endpoint ($LLM_URL)"
    else
        echo "  ⚠️  LLM endpoint not reachable ($LLM_URL) — bot will start but /edgar ask will fail"
    fi

    if [ "$ERRORS" -gt 0 ]; then
        echo ""
        echo "❌ $ERRORS preflight check(s) failed"
        exit 1
    else
        echo ""
        echo "✅ All preflight checks passed"
    fi

# ════════════════════════════════════════════════════════════════════════════
# Data Management
# ════════════════════════════════════════════════════════════════════════════

# Wipe ingested document storage
clean-data:
    @echo "🗑️  Removing document storage..."
    @rm -rf ./data
    @echo "✅ Data cleared"

# Clean build artifacts
clean:
    cargo clean

# Clean everything (build + data)
clean-all: clean clean-data

# ════════════════════════════════════════════════════════════════════════════
# Quality
# ════════════════════════════════════════════════════════════════════════════

# Clippy lints
clippy:
    cargo clippy -- -D warnings

# Format code
fmt:
    cargo fmt

# Format check
fmt-check:
    cargo fmt -- --check

# Full CI pipeline
ci: fmt-check clippy test build-release
    @echo "✅ CI passed"

# ════════════════════════════════════════════════════════════════════════════
# Help
# ════════════════════════════════════════════════════════════════════════════

# List all recipes
help:
    @just --list

# ════════════════════════════════════════════════════════════════════════════
# Shortcuts
# ════════════════════════════════════════════════════════════════════════════

alias b := build
alias r := run
alias t := test
alias c := check
alias i := install
