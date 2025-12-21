.PHONY: dev dev-api dev-frontend dev-all ui-dev \
        build build-wasm build-frontend build-full \
        bump-auto release release-auto publish \
        serve clean check test lint fmt help

# Default target
.DEFAULT_GOAL := help

# =============================================================================
# Development
# =============================================================================

dev: ## Start dev servers (shows instructions)
	@echo "🚀 Starting development servers..."
	@echo ""
	@echo "Run these in separate terminals:"
	@echo "  Terminal 1: make dev-api"
	@echo "  Terminal 2: make dev-frontend"
	@echo ""
	@echo "Or use: make dev-all (requires tmux)"

dev-api: ## Start API server on port 3000 (serves WASM from frontend/public/wasm)
	cargo run -p vibe-graph-cli -- serve --port 3000 --wasm-dir frontend/public/wasm

dev-frontend: ## Start frontend dev server (port 5173)
	cd frontend && pnpm dev

dev-all: ## Start both servers using tmux
	@command -v tmux >/dev/null 2>&1 || { echo "Error: tmux not installed"; exit 1; }
	tmux new-session -d -s vibe 'make dev-api' \; \
		split-window -h 'make dev-frontend' \; \
		attach

ui-dev: ## Run native egui app (vibe-graph-viz example runner)
	cargo run -p vibe-graph-viz --example native --features native

# =============================================================================
# Building
# =============================================================================

check: ## Check all crates compile
	cargo check --workspace

build-wasm: ## Build WASM to frontend/public/wasm/ and update embedded assets
	@command -v wasm-pack >/dev/null 2>&1 || { echo "Installing wasm-pack..."; cargo install wasm-pack; }
	@echo "📦 Building WASM..."
	cd crates/vibe-graph-viz && wasm-pack build --target web --release --out-dir ../../frontend/public/wasm
	@echo "✅ WASM built to frontend/public/wasm/"
	@echo "📦 Updating embedded assets..."
	@mkdir -p crates/vibe-graph-cli/assets
	@cp frontend/public/wasm/vibe_graph_viz_bg.wasm crates/vibe-graph-cli/assets/
	@cp frontend/public/wasm/vibe_graph_viz.js crates/vibe-graph-cli/assets/
	@echo "✅ Embedded assets updated"

build-frontend: build-wasm ## Build frontend (TS + WASM)
	@echo "📦 Building frontend..."
	cd frontend && pnpm install && pnpm build
	@echo "✅ Frontend built to frontend/dist/"

build-cli-embedded: ## Build CLI with embedded WASM
	@echo "📦 Building WASM for embedding..."
	@command -v wasm-pack >/dev/null 2>&1 || { echo "Installing wasm-pack..."; cargo install wasm-pack; }
	cd crates/vibe-graph-viz && wasm-pack build --target web --release
	@mkdir -p crates/vibe-graph-cli/assets
	cp crates/vibe-graph-viz/pkg/vibe_graph_viz_bg.wasm crates/vibe-graph-cli/assets/
	cp crates/vibe-graph-viz/pkg/vibe_graph_viz.js crates/vibe-graph-cli/assets/
	@echo "📦 Building CLI with embedded viz..."
	cargo build --release -p vibe-graph-cli --features embedded-viz
	@echo "✅ Built: target/release/vg ($$(ls -lh target/release/vg | awk '{print $$5}'))"

build: ## Build minimal CLI (D3.js fallback)
	cargo build --release -p vibe-graph-cli
	@echo "✅ Built: target/release/vg ($$(ls -lh target/release/vg | awk '{print $$5}'))"

build-full: build-frontend build ## Full production build
	@echo ""
	@echo "✅ Production build complete!"
	@echo "   Frontend: frontend/dist/"
	@echo "   CLI: target/release/vg"

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

typecheck: ## Type check frontend
	cd frontend && pnpm typecheck

ci: fmt-check lint test typecheck ## Run all CI checks
	@echo "✅ All CI checks passed!"

# =============================================================================
# Serving
# =============================================================================

serve: ## Serve with legacy mode (D3.js fallback)
	cargo run --bin vg -- serve

serve-prod: ## Serve production build
	./target/release/vg serve

# =============================================================================
# Release
# =============================================================================

PUBLISH_CRATES ?= vibe-graph-core vibe-graph-git vibe-graph-api vibe-graph-cli

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
	cargo publish -p vibe-graph-core
	cargo publish -p vibe-graph-git
	cargo publish -p vibe-graph-api
	cargo publish -p vibe-graph-cli

release-auto: bump-auto release ## Auto-bump (changed crates) then publish

publish: ## Publish to crates.io
	$(MAKE) release

# =============================================================================
# Cleanup
# =============================================================================

clean: ## Clean all build artifacts
	cargo clean
	rm -rf crates/vibe-graph-viz/pkg
	rm -rf frontend/dist
	rm -rf frontend/node_modules
	rm -rf frontend/public/wasm/*.wasm
	rm -rf frontend/public/wasm/*.js

clean-wasm: ## Clean only WASM artifacts
	rm -rf crates/vibe-graph-viz/pkg
	rm -rf frontend/public/wasm/*.wasm
	rm -rf frontend/public/wasm/*.js

# =============================================================================
# Setup
# =============================================================================

setup: ## Install development dependencies
	@echo "📦 Installing Rust tools..."
	rustup target add wasm32-unknown-unknown
	cargo install wasm-pack
	@echo "📦 Installing frontend dependencies..."
	cd frontend && pnpm install
	@echo "✅ Setup complete!"

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
	@echo "  3. make dev-api      # Terminal 1: Start API server"
	@echo "  4. make dev-frontend # Terminal 2: Start frontend"
	@echo "  5. Open http://localhost:5173"
