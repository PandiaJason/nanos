.PHONY: all build build-wasm test run-example serve clean

all: build build-wasm

build:
	@echo "⚡ Building nanos runtime binary in release mode..."
	cargo build --release

build-wasm:
	@echo "⚡ Compiling core agent to WASM..."
	cd nanos-core-agent && cargo build --target wasm32-unknown-unknown

test:
	@echo "⚡ Running main test suite..."
	cargo test
	@echo "⚡ Running nanos-core-agent test suite..."
	cd nanos-core-agent && cargo test

run-example: build build-wasm
	@echo "⚡ Running E2E agent run example..."
	cp examples/instruction.txt .
	./target/release/nanos run examples/agent.nano

serve: build
	@echo "⚡ Starting nanos HTTP daemon on port 8080..."
	./target/release/nanos serve --port 8080

clean:
	@echo "⚡ Cleaning up build artifacts..."
	cargo clean
	cd nanos-core-agent && cargo clean
	rm -f instruction.txt secret.txt mcp_output.txt
