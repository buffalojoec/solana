
CARGO?=cargo

test: | binaries tests/run_test_vectors

binaries:
	$(CARGO) build --manifest-path instr/harness/Cargo.toml --bins --release
	$(CARGO) build --manifest-path txn/harness/Cargo.toml --bins --release

tests/run_test_vectors:
	scripts/run_test_vectors.sh
