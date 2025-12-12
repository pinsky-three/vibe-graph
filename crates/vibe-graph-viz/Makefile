# vibe-graph-viz Development Makefile
#
# Quick Start:
#   make dev          # Native desktop with hot reload (recommended)
#   make dev-wasm     # WASM in browser with hot reload

.PHONY: help dev dev-native dev-wasm build build-wasm clean check fmt lint test

# Default target
help:
	@echo "vibe-graph-viz Development Commands"
	@echo "===================================="
	@echo ""
	@echo "Development (hot reload):"
	@echo "  make dev          - Run native desktop app with hot reload"
	@echo "  make dev-wasm     - Run WASM app in browser with hot reload"
	@echo ""
	@echo "Build:"
	@echo "  make build        - Build native release"
	@echo "  make build-wasm   - Build WASM release"
	@echo ""
	@echo "Quality:"
	@echo "  make check        - Run cargo check"
	@echo "  make fmt          - Format code"
	@echo "  make lint         - Run clippy"
	@echo "  make test         - Run tests"
	@echo ""
	@echo "Utilities:"
	@echo "  make clean        - Clean build artifacts"
	@echo "  make deps         - Install dev dependencies"

# Install development dependencies
deps:
	@echo "Installing development tools..."
	cargo install cargo-watch
	cargo install trunk
	rustup target add wasm32-unknown-unknown
	@echo "✅ Dependencies installed"

# === Development (Hot Reload) ===

# Native desktop development with hot reload (RECOMMENDED)
# Uses cargo-watch to rebuild and restart on file changes
dev: dev-native

dev-native:
	@echo "🚀 Starting native dev server with hot reload..."
	@echo "   Press Ctrl+C to stop"
	@echo ""
	cargo watch -x 'run --example native --features native' -w src -w examples

# WASM development with hot reload
# Uses trunk serve for live reload in browser
dev-wasm:
	@echo "🌐 Starting WASM dev server with hot reload..."
	@echo "   Opening http://127.0.0.1:8080"
	@echo "   Press Ctrl+C to stop"
	@echo ""
	trunk serve

# === Build ===

build:
	cargo build --example native --features native --release

build-wasm:
	trunk build --release

# === Quality ===

check:
	cargo check --all-features
	cargo check --target wasm32-unknown-unknown

fmt:
	cargo fmt

lint:
	cargo clippy --all-features -- -D warnings
	cargo clippy --target wasm32-unknown-unknown -- -D warnings

test:
	cargo test --all-features

# === Clean ===

clean:
	cargo clean
	rm -rf target/trunk dist
