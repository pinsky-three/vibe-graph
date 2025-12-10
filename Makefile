.PHONY: build build-viz build-full release publish serve clean check

# Check all crates compile
check:
	cargo check --workspace

# Build minimal CLI (D3.js fallback only, ~8MB)
build:
	cargo build --release -p vibe-graph-cli
	@echo "Built: target/release/vg ($$(ls -lh target/release/vg | awk '{print $$5}'))"

# Build WASM visualization assets
build-viz:
	@command -v wasm-pack >/dev/null 2>&1 || { echo "Installing wasm-pack..."; cargo install wasm-pack; }
	cd crates/vibe-graph-viz && wasm-pack build --target web --release
	@mkdir -p crates/vibe-graph-cli/assets
	cp crates/vibe-graph-viz/pkg/vibe_graph_viz_bg.wasm crates/vibe-graph-cli/assets/
	cp crates/vibe-graph-viz/pkg/vibe_graph_viz.js crates/vibe-graph-cli/assets/
	@echo "WASM assets updated in crates/vibe-graph-cli/assets/"

# Build full CLI with embedded WASM visualization (~11MB)
build-full: build-viz
	cargo build --release -p vibe-graph-cli --features embedded-viz
	@echo "Built: target/release/vg ($$(ls -lh target/release/vg | awk '{print $$5}'))"

# Serve with D3.js fallback (no build required)
serve:
	cargo run --bin vg -- serve

# Serve with embedded WASM (requires build-full first)
serve-full:
	cargo run --bin vg --features embedded-viz -- serve

# Release version bump
release:
	cargo release patch -p vibe-graph-cli --execute

# Publish to crates.io
publish:
	cargo publish -p vibe-graph-cli

# Clean build artifacts
clean:
	cargo clean
	rm -rf crates/vibe-graph-viz/pkg