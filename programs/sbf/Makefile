SRC_DIR := c/src
OUT_DIR := target/deploy
TOOLCHAIN := 1.89.0-sbpf-solana-v1.54

test-v3:
	mkdir -p target/deploy ; \
	TEST_ARGS="--features=sbpf-v3" VER=v3 $(MAKE) rust-new ; \
	TEST_ARGS="--features=sbpf-v3" $(MAKE) test-rust

test-v2:
	VER=v2 $(MAKE) test-version

test-v1:
	VER=v1 $(MAKE) test-version

test-v0: all rust-v0 test-all

clean-all:
	rm -rf target/deploy target/sbpf*

test-all:
	SBF_OUT_DIR=$(OUT_DIR) cargo test --features="sbf_rust,sbf_c" $(TEST_ARGS)

test-rust:
	SBF_OUT_DIR=$(OUT_DIR) cargo test --features="sbf_rust" $(TEST_ARGS)

# Loader V3 max-extension profile. Builds the SBF programs, then prints the
# report. Release only: removing the "in this block already" guards leaves two
# deploys of one program in a slot, which trips a debug assert in the program
# cache.
extend-profile:
	mkdir -p $(OUT_DIR) ; \
	TEST_ARGS="--features=sbpf-v3" VER=v3 $(MAKE) rust-new ; \
	SBF_OUT_DIR=$(OUT_DIR) cargo test --release --features="sbf_rust" --test programs \
	  test_loader_v3_max_extend_bytes_with_pda_authority -- --exact --nocapture

test-version:
	SBPF_CPU=$(VER) $(MAKE) all ; \
	$(MAKE) rust-new ; \
	$(MAKE) test-all

rust-v0:
	cargo +$(TOOLCHAIN) build --release --target sbpf-solana-solana --workspace ; \
	cp -r target/sbpf-solana-solana/release/* target/deploy

rust-new:
	RUSTFLAGS="-C instrument-coverage=no" cargo +$(TOOLCHAIN) build --release --target sbpf$(VER)-solana-solana \
	--workspace $(TEST_ARGS) ; \
	cp -r target/sbpf$(VER)-solana-solana/release/* target/deploy

.PHONY: test-v0

include c/sbf.mk
