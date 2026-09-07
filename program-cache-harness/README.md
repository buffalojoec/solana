# Program Cache Harness

```
./cargo nightly fuzz run <v1|v2> --fuzz-dir program-cache-harness/fuzz -- -max_len=128
```

> `-max_len=128` is important. Below ~64 bytes the generator cannot emit longer
> op sequences, and several previous known bugs need at least six ops.

Corpus lives in `program-cache-harness/fuzz/corpus/v1`.

## Coverage

```
./cargo nightly fuzz coverage <v1|v2> --fuzz-dir program-cache-harness/fuzz
```

The instrumented binary lands in the *workspace* target directory, not under
`fuzz/target`:

```
target/x86_64-unknown-linux-gnu/coverage/x86_64-unknown-linux-gnu/release/<v1|v2>
```

Report against `program-runtime/src/loaded_programs.rs` with `llvm-cov`. The
toolchain needs `llvm-tools`, or the merge step fails with no `llvm-profdata`:

```
rustup component add llvm-tools --toolchain nightly-<date>
```

Read the report with `--show-functions -Xdemangler=rustfilt`, and ignore the
`ProgramCache<_>` rows.

## Triage

```
cargo run -p solana-program-cache-harness --bin decode-scenario \
    -- program-cache-harness/fuzz/artifacts/<v1|v2>/<artifact>
```

Transcribe the decoded scenario into `tests/scenario.rs`.

## Invariants

All four are defined in [`src/invariants.rs`](src/invariants.rs) and are
`Critical`. A run also fails if two executions of one scenario leave different
cache contents, reported as `deterministic`.

| Invariant | Asserts |
| --- | --- |
| `wrong-entry` | The entry served matches the deployment slot, owner and environment the caller asked for. |
| `entry-is-on-the-callers-fork` | The entry served comes from the caller's own lineage. |
| `entry-matches-ledger` | The entry served is one the scenario actually placed, compared by pointer. |
| `no-repeated-reload` | A load is not requested twice for the same deployment slot. |
