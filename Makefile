.PHONY: dev build build-wasm build-release \
        serve clean check test lint fmt help

# Default target
.DEFAULT_GOAL := help

# =============================================================================
# Development
# =============================================================================

dev: ## Start dev server (builds WASM if needed, then serves)
	@if [ ! -f crates/vibe-graph-cli/assets/vibe_graph_viz_bg.wasm ]; then \
		echo "📦 WASM assets not found, building..."; \
		$(MAKE) build-wasm; \
	fi
	cargo run -p vibe-graph-cli -- serve --port 3000

ui-dev: ## Run native egui app (vibe-graph-viz example runner)
	cargo run -p vibe-graph-viz --example native --features native

# =============================================================================
# Building
# =============================================================================

check: ## Check all crates compile
	cargo check --workspace

build-wasm: ## Build WASM visualization with WebGPU support
	@command -v wasm-pack >/dev/null 2>&1 || { echo "Installing wasm-pack..."; cargo install wasm-pack; }
	@echo "📦 Building WASM with WebGPU support..."
	cd crates/vibe-graph-viz && wasm-pack build --target web --release --out-dir pkg --features gpu-layout
	@echo "📦 Copying to CLI assets..."
	@mkdir -p crates/vibe-graph-cli/assets
	@cp crates/vibe-graph-viz/pkg/vibe_graph_viz_bg.wasm crates/vibe-graph-cli/assets/
	@cp crates/vibe-graph-viz/pkg/vibe_graph_viz.js crates/vibe-graph-cli/assets/
	@echo "✅ WASM built with GPU layout support (WebGPU)"

build: ## Build CLI with native viz and GPU layout
	cargo build --release -p vibe-graph-cli --features native-viz,gpu-layout
	@echo "✅ Built: target/release/vg ($$(ls -lh target/release/vg | awk '{print $$5}'))"
	@echo "   GPU layout available via: vg viz"

build-release: build-wasm ## Build production CLI with embedded WASM
	cargo build --release -p vibe-graph-cli
	@echo ""
	@echo "✅ Production build complete!"
	@echo "   CLI: target/release/vg"
	@ls -lh target/release/vg | awk '{print "   Size:", $$5}'
	@echo ""
	@echo "This binary includes embedded WASM visualization."

# =============================================================================
# Testing & Linting
# =============================================================================

test: ## Run all tests
	cargo test --workspace

lint: ## Run Rust lints (clippy)
	cargo clippy --workspace -- -D warnings

fmt: ## Run Rust formatter
	cargo fmt --all

fmt-check: ## Check Rust formatting
	cargo fmt --all -- --check

ci: fmt-check lint test ## Run all CI checks
	@echo "✅ All CI checks passed!"

# =============================================================================
# Serving
# =============================================================================

serve: ## Serve with production build
	./target/release/vg serve

# =============================================================================
# Release
# =============================================================================

PUBLISH_CRATES ?= vibe-graph-core vibe-graph-cli

bump-auto: ## Bump patch versions for crates changed since last tag
	@set -eu; \
	if [ -n "$$(git status --porcelain)" ]; then \
		echo "✋ Working tree is dirty. Commit or stash before bumping."; \
		exit 1; \
	fi; \
	for crate in $(PUBLISH_CRATES); do \
		dir="crates/$${crate}"; \
		tag="$$(git tag --list "$${crate}-v*" --sort=-v:refname | awk 'NR==1 { print; exit }')"; \
		if [ -z "$$tag" ]; then \
			echo "🔁 $$crate: no prior tag found -> bump patch"; \
			cargo release patch -p "$$crate" --no-publish --execute; \
			continue; \
		fi; \
		changed="$$(git diff --name-only "$$tag"..HEAD -- "$$dir" | awk 'NR==1 { print; exit }')"; \
		if [ -n "$$changed" ]; then \
			echo "🔁 $$crate: changed since $$tag -> bump patch"; \
			cargo release patch -p "$$crate" --no-publish --execute; \
		else \
			echo "⏭️  $$crate: no changes since $$tag -> skip"; \
		fi; \
	done

release: ## Publish crates to crates.io (dependency order)
	@echo "Publishing workspace crates (dependency order)..."
	@echo "1/5: vibe-graph-core"
	cargo publish -p vibe-graph-core
	@echo "2/5: vibe-graph-git"
	cargo publish -p vibe-graph-git
	@echo "3/5: vibe-graph-ops"
	cargo publish -p vibe-graph-ops
	@echo "4/5: vibe-graph-api"
	cargo publish -p vibe-graph-api
	@echo "5/5: vibe-graph-cli"
	cargo publish -p vibe-graph-cli
	@echo "✅ All crates published!"

release-auto: bump-auto release ## Auto-bump (changed crates) then publish

publish: ## Publish to crates.io
	$(MAKE) release

# =============================================================================
# Cleanup
# =============================================================================

clean: ## Clean all build artifacts
	cargo clean
	rm -rf crates/vibe-graph-viz/pkg
	rm -rf crates/vibe-graph-cli/assets/*.wasm
	rm -rf crates/vibe-graph-cli/assets/*.js

clean-wasm: ## Clean only WASM artifacts
	rm -rf crates/vibe-graph-viz/pkg
	rm -rf crates/vibe-graph-cli/assets/*.wasm
	rm -rf crates/vibe-graph-cli/assets/*.js

# =============================================================================
# Setup
# =============================================================================

setup: ## Install development dependencies
	@echo "📦 Installing Rust tools..."
	rustup target add wasm32-unknown-unknown
	cargo install wasm-pack
	@echo "✅ Setup complete!"
	@echo ""
	@echo "Quick start:"
	@echo "  1. make build-wasm   # Build WASM visualization"
	@echo "  2. make dev          # Start dev server"
	@echo "  3. Open http://localhost:3000"

# =============================================================================
# Help
# =============================================================================

help: ## Show this help message
	@echo "Vibe Graph - Development Commands"
	@echo ""
	@echo "Usage: make [target]"
	@echo ""
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-18s\033[0m %s\n", $$1, $$2}'
	@echo ""
	@echo "Quick Start:"
	@echo "  1. make setup        # Install dependencies"
	@echo "  2. make build-wasm   # Build WASM visualization"
	@echo "  3. make dev          # Start server"
	@echo "  4. Open http://localhost:3000"
