# SIMD-0296 Test Coverage Analysis & Recommendations

## Existing Coverage

### Unit Test: `test_instruction_stack_height`

**File:** `program-runtime/src/invoke_context.rs:1218-1271`

Parameterized with `#[test_case]` for both feature states. Pushes
frames until `CallDepth` fires and asserts that:
- At least one frame was pushed (`depth_reached != 0`)
- The limit was hit before `one_more_than_max_depth`

This verifies the enforcement point works, but it tests via the
mock invoke context — not through actual SBF program execution.
The test does NOT check the exact depth at which it breaks; it only
confirms that *some* limit exists and was reached.

### Integration Test: `test_program_sbf_invoke_sanity`

**File:** `programs/sbf/tests/programs.rs:738-1317`

The main integration test. Runs both C and Rust SBF programs through
a real bank. Feature activation is toggled via `bank_with_feature_activated`
/ `bank_with_feature_deactivated` helpers.

**Success cases (depth N succeeds):**

| Line | Test Constant | Depth | Feature | Lang |
|------|--------------|-------|---------|------|
| 324 | `TEST_SUCCESS` (calls `do_nested_invokes(4, ...)`) | 4 | ON (default) | Both |
| 991 | `TEST_NESTED_INVOKE_SIMD_0268_OK` | 8 | ON (explicit) | Both |

**Failure cases (depth N fails with `CallDepth`):**

| Line | Test Constant | Depth | Feature | Lang |
|------|--------------|-------|---------|------|
| 1277 | `TEST_NESTED_INVOKE_TOO_DEEP` | 5 | OFF (explicit) | Both |
| 1301 | `TEST_NESTED_INVOKE_SIMD_0268_TOO_DEEP` | 9 | ON (explicit) | Both |

### SBF Test Programs

**`programs/sbf/rust/invoke/src/lib.rs:30-65`** — `do_nested_invokes`:
Transfers 5 lamports from ARGUMENT to INVOKED_ARGUMENT, then invokes the
invoked program with `NESTED_INVOKE` and a remaining-invokes counter. Each
level transfers 1 lamport back. Two invocations per level.

**`programs/sbf/rust/invoked/src/lib.rs:207-240`** — `NESTED_INVOKE` handler:
Decrements remaining-invokes counter. If > 1, recurses via CPI. At
the last level, writes data to the invoked argument account.

**`programs/sbf/c/src/invoke/invoke.c:77-87`** — C version of
`do_nested_invokes`. Same logic.

**`programs/sbf/c/src/invoked/invoked.c:53`** — C invoked program.

### CPI Module Tests

**File:** `program-runtime/src/cpi.rs:1416+`

Tests for `update_caller_account`, `update_callee_account`,
`translate_instruction`, `translate_signers`, `translate_accounts_rust`,
`caller_account_from_account_info`. All operate at a single CPI level —
none exercise nesting depth.

## Coverage Gaps

### Gap 1: No "depth 4 succeeds" test with feature OFF

**Severity: Medium**

The test at line 1277 confirms depth 5 **fails** with feature OFF, but
there is no explicit test that depth 4 **succeeds** with feature OFF.
The `TEST_SUCCESS` path (line 324) does call `do_nested_invokes(4, ...)`
but runs on a bank where the feature is ON (all features active by
default in the genesis config). There is no test that isolates:
feature OFF + depth 4 = success.

**Recommendation:** Add a test case that explicitly deactivates the
feature, then invokes `do_nested_invokes(4, ...)` and asserts success.
This pins the lower boundary of the old limit.

```
Location: programs/sbf/tests/programs.rs, near line 1266
Pattern:  bank_with_feature_deactivated(..., &raise_cpi_nesting_limit_to_8::id())
          do_invoke_success(TEST_SUCCESS, ..., &bank)  // depth 4
```

### Gap 2: Intermediate depths 5, 6, 7 not tested with feature ON

**Severity: Medium**

`TEST_NESTED_INVOKE_SIMD_0268_OK` tests depth 8 (maximum). Depths 5, 6,
and 7 are never explicitly tested with the feature ON. An off-by-one
error in the enforcement logic at an intermediate depth would go
undetected.

**Recommendation:** Add at least one intermediate depth test (e.g.
depth 5 or 6 succeeds with feature ON). This doesn't need a new SBF
program — `do_nested_invokes` already accepts an arbitrary depth. Add a
new test constant like `TEST_NESTED_INVOKE_INTERMEDIATE` with depth 6.

```
Location: programs/sbf/rust/invoke_dep/src/lib.rs (add constant)
          programs/sbf/rust/invoke/src/lib.rs (add handler)
          programs/sbf/c/src/invoke/invoke.c (add handler)
          programs/sbf/tests/programs.rs (add success assertion)
```

### Gap 3: No test for depth 5 failure with feature OFF followed by depth 5 success with feature ON

**Severity: Low-Medium**

The existing tests toggle the feature and test different depths, but
never test the *same* depth under both feature states. The critical
boundary is depth 5:
- Feature OFF: depth 5 should FAIL (`CallDepth`)
- Feature ON: depth 5 should SUCCEED

This would catch any bug where the feature gate doesn't actually change
the limit.

**Recommendation:** In the same test function, after the existing depth 5
failure test (line 1277), activate the feature and test depth 5 success.
This uses the existing `TEST_NESTED_INVOKE_TOO_DEEP` constant (which
calls `do_nested_invokes(5, ...)`).

```
Location: programs/sbf/tests/programs.rs, after line 1289
Pattern:
    // Activate feature, same depth should now succeed.
    let bank = bank_with_feature_activated(
        &bank_forks, bank, &raise_cpi_nesting_limit_to_8::id(),
    );
    // Reset account balances for the deeper nesting.
    bank.store_account(&argument_keypair.pubkey(),
        &AccountSharedData::new(42, 100, &invoke_program_id));
    bank.store_account(&invoked_argument_keypair.pubkey(),
        &AccountSharedData::new(20, 10, &invoked_program_id));
    do_invoke_success(
        TEST_NESTED_INVOKE_TOO_DEEP,
        &[],
        &[invoked_program_id.clone(); 10],  // 5 depth * 2 invocations
        &bank,
    );
```

### Gap 4: No mixed SBF-to-builtin nesting test

**Severity: Medium**

All nesting tests use purely SBF→SBF chains (the invoked program is
always an SBF program). There is no test where an SBF program CPIs
into a builtin (e.g. system program) which then returns, and the SBF
program CPIs again, building up depth through mixed program types.

This matters because builtins and SBF programs take different code paths
through `process_executable_chain` vs `execute`. Verifying that the
stack depth counter increments correctly for both types is important.

**Recommendation:** Write a targeted unit test in
`program-runtime/src/invoke_context.rs` that:
1. Sets up a chain: SBF program → builtin → SBF program
2. Verifies depth reaches the expected limit
3. Tests both feature states

This is easier as a unit test (using mock invoke context) than an SBF
integration test, since it requires mixing program types.

```
Location: program-runtime/src/invoke_context.rs, in mod tests
Pattern:  Similar to test_instruction_stack_height but alternating
          program types (Builtin vs Loaded) at each push level.
```

### Gap 5: No test for account data integrity across deep nesting

**Severity: Medium**

`TEST_STACK_HEAP_ZEROED` (`programs/sbf/tests/programs.rs:5466`) tests
that stack and heap memory are zeroed between invocations and recurses
to max depth. However, it runs with all features active and doesn't
specifically test the new depth range (5-8). More importantly, there
is no test that verifies **account data** (not stack/heap) is correctly
propagated across 5-8 CPI levels.

The `do_nested_invokes` function does transfer lamports at each level,
which provides implicit account data verification. But it doesn't test:
- Account data writes at deep nesting (only writes at the leaf level)
- Account reallocation at deep nesting
- Data passed back from a depth-8 callee to the top-level caller

**Recommendation:** Extend the nested invoke test to verify that account
data modifications at the deepest level are visible to the top-level
caller after all CPIs return. The existing `do_nested_invokes` already
does lamport verification — add data content verification.

```
Location: programs/sbf/rust/invoked/src/lib.rs, NESTED_INVOKE handler
Pattern:  At each nesting level, write the current depth to a known
          offset in the account data. After all CPIs return, the
          top-level program verifies the data shows the deepest level
          reached.
```

### Gap 6: No CU exhaustion test at deep nesting

**Severity: Low**

No test verifies what happens when a deeply nested CPI chain exhausts
the compute budget. Specifically: if a program at depth 7 runs out of
CUs, does the error propagate correctly back through all 7 levels?

**Recommendation:** Write a test that sets a tight CU limit and
verifies that a depth-8 nesting attempt fails with
`ComputationalBudgetExceeded` (not `CallDepth`). This confirms CU
metering takes precedence over depth limits when both are near their
boundary.

```
Location: programs/sbf/tests/programs.rs
Pattern:  Use ComputeBudgetInstruction::set_compute_unit_limit with
          a budget barely sufficient for depth 4 but insufficient for
          depth 8. Assert InstructionError::ComputationalBudgetExceeded.
```

### Gap 7: No program cache recompilation test at new depth

**Severity: Low**

`Bank::prepare_program_cache_for_upcoming_feature_set` (`runtime/src/bank.rs:1479`)
uses the upcoming feature set to determine the compute budget for
recompilation. The code correctly queries
`upcoming_feature_set.is_active(&raise_cpi_nesting_limit_to_8::id())`
(confirmed in the PR diff). However, there is no test that verifies
programs are recompiled with the correct `max_instruction_stack_depth`
when the feature activates at an epoch boundary.

**Recommendation:** This is a complex test to write (requires simulating
epoch transitions). Low priority since the code path is straightforward
and the feature gate plumbing is verified elsewhere.

## Coverage Matrix

| Scenario | Feature OFF | Feature ON |
|----------|:----------:|:---------:|
| Depth 1-3 succeed | implicit | implicit |
| Depth 4 succeeds | **GAP** | implicit |
| Depth 5 fails | **tested** (line 1277) | — |
| Depth 5 succeeds | — | **GAP** |
| Depth 6-7 succeed | — | **GAP** |
| Depth 8 succeeds | — | **tested** (line 991) |
| Depth 9 fails | — | **tested** (line 1301) |
| Mixed SBF/builtin chain | **GAP** | **GAP** |
| Account data at depth 5-8 | — | **GAP** |
| CU exhaustion at depth | not tested | not tested |
| Depth 5: fail OFF, succeed ON | **GAP** (same depth, both states) | |

## Implementation Plan

### Where to put each test

**`program-runtime/src/invoke_context.rs` (unit tests)** — for gaps
that test the enforcement point (TransactionContext::push) and don't
need real SBF execution. Fast, no SBF compilation.

**`programs/sbf/tests/programs.rs` (integration tests)** — for gaps
that require actual SBF programs executing through the full VM
pipeline, real bank state, and data flow verification.

| # | Gap | Location | Effort |
|---|-----|----------|--------|
| 1 | Depth 4 succeeds, feature OFF | invoke_context.rs | Low |
| 3 | Depth 5 toggle (fail OFF → succeed ON) | invoke_context.rs | Low |
| 4 | Mixed SBF/builtin chain | invoke_context.rs | Medium |
| 2 | Intermediate depth 6, feature ON | programs.rs | Low |
| 5 | Account data at depth 5-8 | programs.rs | Medium |
| 6 | CU exhaustion at deep nesting | programs.rs | Low |

### Gap 1 + 3: Exact boundary + feature toggle (invoke_context.rs)

The existing `test_instruction_stack_height` (`invoke_context.rs:1218`)
only asserts that *some* limit was hit, not the *exact* depth. Strengthen
it to assert the precise boundary values.

**Pattern to follow:** Same setup as `test_instruction_stack_height`
(line 1225-1251) — builds `invoke_stack`, `transaction_accounts`,
`instruction_accounts`, then pushes frames in a loop.

```rust
// After the existing test, add:

#[test]
fn test_instruction_stack_height_exact_boundaries() {
    // Feature OFF: exactly 5 pushes should succeed, 6th should fail.
    {
        let budget = SVMTransactionExecutionBudget::new_with_defaults(false);
        let max_depth = budget.max_instruction_stack_depth; // 5
        // ... setup transaction_accounts for max_depth + 1 ...
        // with_mock_invoke_context!(invoke_context, ...);

        for i in 0..max_depth {
            // configure_top_level_instruction_for_tests(...)
            assert!(invoke_context.push().is_ok(), "depth {i} should succeed (feature OFF)");
        }
        // configure_top_level_instruction_for_tests(...)
        assert_eq!(invoke_context.push(), Err(InstructionError::CallDepth));
    }

    // Feature ON: exactly 9 pushes should succeed, 10th should fail.
    {
        let budget = SVMTransactionExecutionBudget::new_with_defaults(true);
        let max_depth = budget.max_instruction_stack_depth; // 9
        // ... same pattern ...
    }
}
```

This covers gaps 1 (depth 4 succeeds OFF) and 3 (same enforcement point,
both feature states) with exact assertions. Uses the same setup pattern as
lines 1225-1251 but asserts `is_ok()` at each level instead of just
counting.

### Gap 4: Mixed SBF/builtin chain (invoke_context.rs)

**Pattern to follow:** The `native_invoke_signed` tests added in PR #10681
(`invoke_context.rs:1750+`) show how to set up a mock builtin
(`MockBuiltin::vm`) in the program cache and call through
`invoke_context.push()`. The `test_instruction_stack_height` test already
pushes multiple frames.

The key insight: `TransactionContext::push()` doesn't distinguish program
types — the depth counter increments uniformly. So this test is really
confirming that the counter works the same way regardless of what calls
`push()`. A targeted test would:

1. Use `with_mock_invoke_context!` (line 1251 pattern)
2. Push frames alternating between two program IDs (one "builtin",
   one "SBF" — from the counter's perspective they're identical)
3. Verify the limit is hit at exactly `max_instruction_stack_depth`

```rust
#[test_case(false; "SIMD-0268 disabled")]
#[test_case(true; "SIMD-0268 enabled")]
fn test_instruction_stack_height_mixed_program_types(simd_0268_active: bool) {
    let budget = SVMTransactionExecutionBudget::new_with_defaults(simd_0268_active);
    let max_depth = budget.max_instruction_stack_depth;

    // Setup with 2 program IDs (alternating "builtin" and "SBF")
    // ... same account setup as test_instruction_stack_height ...
    // Push max_depth frames, alternating program IDs
    // Assert all succeed, then the (max_depth + 1)th fails
}
```

Note: This will confirm the counter is program-type-agnostic, but it
does NOT test that builtins skip the memory pool (that's an Agave
implementation detail, not an enforcement property).

### Gap 2: Intermediate depth with real SBF execution (programs.rs)

**Pattern to follow:** The existing `TEST_NESTED_INVOKE_SIMD_0268_OK`
(line 991) with `do_invoke_success`. Add a new test constant for depth 6.

**SBF program changes:**

`programs/sbf/rust/invoke_dep/src/lib.rs` — add constant:
```rust
pub const TEST_NESTED_INVOKE_SIMD_0268_INTERMEDIATE: u8 = 48;
```

`programs/sbf/rust/invoke/src/lib.rs` — add handler (after line 698):
```rust
TEST_NESTED_INVOKE_SIMD_0268_INTERMEDIATE => {
    let _ = do_nested_invokes(6, accounts);
}
```

`programs/sbf/c/src/invoke/invoke.c` — add C handler (after line 637):
```c
case TEST_NESTED_INVOKE_SIMD_0268_INTERMEDIATE: {
    do_nested_invokes(6, accounts, params.ka_num);
    break;
}
```

`programs/sbf/tests/programs.rs` — add success assertion (after line 996):
```rust
// Reset balances for depth-6 test.
bank.store_account(&argument_keypair.pubkey(),
    &AccountSharedData::new(42, 100, &invoke_program_id));
bank.store_account(&invoked_argument_keypair.pubkey(),
    &AccountSharedData::new(20, 10, &invoked_program_id));
do_invoke_success(
    TEST_NESTED_INVOKE_SIMD_0268_INTERMEDIATE,
    &[],
    &[invoked_program_id.clone(); 12],  // 6 depth * 2 invocations
    &bank,
);
```

### Gap 5: Account data integrity at depth 5-8 (programs.rs)

**Pattern to follow:** The `NESTED_INVOKE` handler in the invoked program
(`programs/sbf/rust/invoked/src/lib.rs:207-240`) already writes data at the
leaf level (line 234-237). Extend this to write the current depth at each
level, then verify at the top.

This is the most invasive change — it modifies the invoked program's
behavior. An alternative: add a new test constant
`TEST_NESTED_INVOKE_DATA_INTEGRITY` that calls a variant of
`do_nested_invokes` which checks account data content (not just lamports)
after returning from the full depth-8 chain.

The simpler approach: the existing `do_nested_invokes` already verifies
lamport balances at the end (lines 56-67), which implicitly proves data
flows correctly through all levels (each level transfers lamports, and
the final balance depends on every level executing correctly). The
depth-8 success test (`TEST_NESTED_INVOKE_SIMD_0268_OK`) already
exercises this at full depth.

**Recommendation:** The lamport verification is sufficient for data
integrity at the CPI account update level. If you want stronger coverage,
add a test that writes `depth_counter` to a data byte at each level
and reads it back at the top. But this is lower priority given the
implicit coverage from lamport accounting.

### Gap 6: CU exhaustion at deep nesting (programs.rs)

**Pattern to follow:** `do_invoke_failure_test_local_with_compute_check`
(line 1055) already supports a `should_deplete_compute_meter` flag and
uses `ComputeBudgetInstruction::set_compute_unit_limit`. Use the same
pattern with a tight budget.

```rust
// After the SIMD_0268_TOO_DEEP failure test (line 1317):
//
// Test that CU exhaustion fires before CallDepth at depth 8
// when compute budget is insufficient.
do_invoke_failure_test_local_with_compute_check(
    TEST_NESTED_INVOKE_SIMD_0268_OK,  // attempts depth 8
    TransactionError::InstructionError(
        0,
        InstructionError::ComputationalBudgetExceeded,
    ),
    &[invoked_program_id.clone(); ..],  // partial chain before CU exhaustion
    None,
    true,  // should_deplete_compute_meter
    &bank,
);
```

The tricky part is predicting the exact CU consumption at each depth
level to set the budget just right. The `do_invoke_failure_test_local`
helper already adds a `ComputeBudgetInstruction::set_compute_unit_limit`
to the transaction (line 1067). Set it to a value that allows depth 4
but not depth 8.
