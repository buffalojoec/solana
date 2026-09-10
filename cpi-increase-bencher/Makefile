# Builds the recursive CPI program, then benches messages against it.
#
#   make wall-clock                 time every scenario, under criterion
#   make wall-clock FILTER=32x1     just the ones matching FILTER
#   make resources                  compute units, peak memory, copied bytes,
#                                   then the comparisons they imply
#   make sbf                        build the program only

HERE := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))
ROOT := $(abspath $(HERE)/..)
REPORT := $(ROOT)/target/cpi-bench/report.json
SBF_OUT_DIR := $(ROOT)/target/sbf-out
PROGRAM := $(SBF_OUT_DIR)/solana_sbf_rust_cpi_depth.so
MANIFEST := $(ROOT)/programs/sbf/rust/cpi_depth/Cargo.toml

.PHONY: wall-clock resources sbf clean

wall-clock: sbf
	SBF_OUT_DIR=$(SBF_OUT_DIR) $(ROOT)/cargo stable bench \
		-p cpi-increase-bencher --bench wall_clock -- $(FILTER)

resources: sbf
	@mkdir -p $(dir $(REPORT))
	SBF_OUT_DIR=$(SBF_OUT_DIR) $(ROOT)/cargo stable run --release \
		-p cpi-increase-bencher --example resources -- $(REPORT)
	@python3 $(HERE)/report.py $(REPORT) $(ROOT)/target/criterion/frames

sbf: $(PROGRAM)

$(PROGRAM): $(ROOT)/programs/sbf/rust/cpi_depth/src/lib.rs $(MANIFEST)
	cargo-build-sbf --manifest-path $(MANIFEST) --sbf-out-dir $(SBF_OUT_DIR)

clean:
	rm -rf $(SBF_OUT_DIR)
