# CPI Refactor: Error Code Change Analysis

Comprehensive analysis of error type changes across PRs #5559 through
#8046 and their impact on validator behavior and consensus.

## Background: Error Codes and Consensus

Per [buffalojoec's analysis](https://gist.github.com/buffalojoec/6b463e94637deeb2e82af434214ecc64),
transaction error codes **do not affect consensus**:

- The bank hash includes parent hash, signature count, last blockhash,
  and accounts lattice hash — but **not error codes**.
- Signature count is a scalar count of processed transactions (pass and
  fail alike). The specific error code does not contribute.
- Failed transactions only modify the fee payer (fees deducted) and
  optionally the nonce account (nonce advanced). The error code never
  influences which accounts get stored.
- Error codes live in the status cache and blockstore — local metadata
  for deduplication and RPC queries — neither feeds into consensus.
- This was empirically validated: a mainnet validator patched to return
  different error codes remained in consensus with identical bank hashes.

**The pass/fail boundary is consensus-critical.** Within the failure
category, the specific error reason is irrelevant to consensus.

## Inventory of Error Type Changes

### PR #7836 — Memory translation errors

The `translate_type_inner!` and `translate_slice_inner!` macros moved
from `syscalls/src/lib.rs` to `program-runtime/src/memory.rs`. The
error variants they emit changed:

| Error Path | Before (boxed type) | After (boxed type) |
|-----------|--------------------|--------------------|
| Unaligned pointer | `SyscallError::UnalignedPointer` | `MemoryTranslationError::UnalignedPointer` |
| Invalid length | `SyscallError::InvalidLength` | `MemoryTranslationError::InvalidLength` |

### PR #7861 — CPI validation errors

Validation functions moved to `program-runtime/src/cpi.rs`. Error
variants changed:

| Error Path | Before | After |
|-----------|--------|-------|
| Invalid account info pointer | `SyscallError::InvalidPointer` | `CpiError::InvalidPointer` |
| Too many instruction accounts | `SyscallError::MaxInstructionAccountsExceeded{..}` | `CpiError::MaxInstructionAccountsExceeded{..}` |
| Instruction data too large | `SyscallError::MaxInstructionDataLenExceeded{..}` | `CpiError::MaxInstructionDataLenExceeded{..}` |
| Too many account infos | `SyscallError::MaxInstructionAccountInfosExceeded{..}` | `CpiError::MaxInstructionAccountInfosExceeded{..}` |
| Unauthorized CPI program | `SyscallError::ProgramNotSupported(..)` | `CpiError::ProgramNotSupported(..)` |

PR #7861 also added manual `From` impls that map these back:
- `From<MemoryTranslationError> for SyscallError` — flat variant mapping
- `From<CpiError> for SyscallError` — flat variant mapping

### PR #7941 — Account update errors

| Error Path | Before | After |
|-----------|--------|-------|
| Callee data slice out of bounds | `SyscallError::InvalidLength` | `CpiError::InvalidLength` |

### PR #7983 — Translate account errors

| Error Path | Before | After |
|-----------|--------|-------|
| Account info pointer in accounts region | `SyscallError::InvalidPointer` | `CpiError::InvalidPointer` |
| Account info bounds check | `SyscallError::InvalidLength` | `CpiError::InvalidLength` |

### PR #8046 — Signer translation errors

| Error Path | Before | After |
|-----------|--------|-------|
| Too many signers | `SyscallError::TooManySigners` | `CpiError::TooManySigners` |
| Bad PDA seeds | `SyscallError::BadSeeds(err)` | `CpiError::BadSeeds(err)` |

### Errors NOT changed

The following errors were already `InstructionError` variants (not
`SyscallError`) and were unchanged across the refactor:

- `InstructionError::InvalidRealloc` — realloc bounds violations
- `InstructionError::MissingAccount` — unknown account references
- `InstructionError::AccountDataTooSmall` — data access out of bounds
- `InstructionError::InvalidArgument` — invalid bool values in AccountMeta
- `InstructionError::MaxSeedLengthExceeded` — too many seeds per signer

These pass through `Box<dyn Error>` unchanged and are always
downcastable to `InstructionError` at the exit points.

## Error Propagation Path

CPI errors originate in `program-runtime/src/cpi.rs` functions and
propagate through the following chain:

```
1. CPI function returns Err(Box<dyn Error>)
      containing CpiError, MemoryTranslationError, or InstructionError
                        |
2. ? propagates through cpi_common() -> Result<u64, Error>
                        |
3. ? propagates to SyscallInvokeSignedRust::rust() -> Result<u64, Error>
      (the declare_builtin_function! handler)
                        |
4. SBPF VM catches the Err and wraps it:
      EbpfError::SyscallError(Box<dyn Error>)
      stored as ProgramResult::Err(EbpfError::SyscallError(err))
                        |
5. vm.rs execute() unwraps at line 394:
      if let EbpfError::SyscallError(err) = error { err }
      Returns: Result<(), Box<dyn Error>>
                        |
6. Caller (bpf_loader process_instruction) returns Box<dyn Error>
      through declare_builtin_function! back into the VM
                        |
7. invoke_context.rs process_executable_chain (line 607-621):
      Tries downcast_ref::<InstructionError>()
        Success -> returns the specific InstructionError
        Failure -> returns InstructionError::ProgramFailedToComplete
```

## Impact Analysis

### What changed

The concrete type inside `Box<dyn Error>` at step 1 changed from
`SyscallError` to `CpiError` or `MemoryTranslationError`.

### What happens at step 7

At the final downcast (invoke_context.rs line 609):

```rust
if let Some(instruction_err) = syscall_error.downcast_ref::<InstructionError>() {
    Err(instruction_err.clone())
} else {
    Err(InstructionError::ProgramFailedToComplete)
}
```

- **Before refactor:** `Box<SyscallError>` — fails `downcast_ref::<InstructionError>()`,
  falls through to `ProgramFailedToComplete`.
- **After refactor:** `Box<CpiError>` — fails `downcast_ref::<InstructionError>()`,
  falls through to `ProgramFailedToComplete`.

**Both paths produce the same `InstructionError::ProgramFailedToComplete`.**

The `SyscallError` type was never `InstructionError`, so it was never
successfully downcast at this point. The error code was always
`ProgramFailedToComplete` for these syscall error paths.

For errors that ARE `InstructionError` (e.g. `InvalidRealloc`,
`MissingAccount`, `AccountDataTooSmall`), these are boxed directly as
`Box<InstructionError>` and the downcast succeeds both before and after
the refactor.

### What about the From impls?

PR #7861 added:
- `From<MemoryTranslationError> for SyscallError`
- `From<CpiError> for SyscallError`

These conversions map back to the original `SyscallError` variants. But
they are **not invoked in the error propagation path**. The CPI
functions box errors directly via `.into()` or `Box::new()` into
`Box<dyn Error>`. The `From` impls exist for potential use in test
assertions or other code that explicitly converts, but the production
error path never converts `CpiError` -> `SyscallError`.

### RPC impact

The error code surfaced to RPC clients for CPI failures was
`ProgramFailedToComplete` before the refactor and remains
`ProgramFailedToComplete` after. The error *message* (from the
`Display` impl) may appear in logs, and the messages are identical
between old and new types:

| Old (`SyscallError`) | New (`CpiError` / `MemoryTranslationError`) | Message |
|-----|-----|---------|
| `UnalignedPointer` | `UnalignedPointer` | "Unaligned pointer" |
| `InvalidLength` | `InvalidLength` | "InvalidLength" |
| `InvalidPointer` | `InvalidPointer` | "Invalid pointer" |
| `TooManySigners` | `TooManySigners` | "Too many signers" |
| `BadSeeds(e)` | `BadSeeds(e)` | "Could not create program address with signer seeds: {e}" |
| `ProgramNotSupported(pk)` | `ProgramNotSupported(pk)` | "Program {pk} not supported by inner instructions" |
| `MaxInstructionAccountsExceeded{..}` | `MaxInstructionAccountsExceeded{..}` | "Invoked an instruction with too many accounts ({n} > {m})" |
| `MaxInstructionDataLenExceeded{..}` | `MaxInstructionDataLenExceeded{..}` | "Invoked an instruction with data that is too large ({n} > {m})" |
| `MaxInstructionAccountInfosExceeded{..}` | `MaxInstructionAccountInfosExceeded{..}` | "Invoked an instruction with too many account info's ({n} > {m})" |

## Security Assessment

1. **Consensus:** Not affected. Error codes do not enter the bank hash.
   The pass/fail boundary is unchanged — the same conditions cause
   failure before and after.

2. **Error code identity:** The `InstructionError` variant returned to
   the transaction pipeline is unchanged (`ProgramFailedToComplete`
   for syscall-originated errors, specific variants for
   `InstructionError`-originated errors).

3. **Log messages:** Identical. The `Display` impls produce the same
   strings.

4. **Downcast safety:** No production code downcasts `Box<dyn Error>`
   to `SyscallError`, `CpiError`, or `MemoryTranslationError`. The
   only downcast in the production path is to `InstructionError`
   (invoke_context.rs:609) and `EbpfError` (vm.rs:336). Test code
   that downcasts to `InstructionError` (e.g. `error.downcast_ref::<InstructionError>()`)
   continues to work because `InstructionError`-boxed errors were not
   changed.

5. **Pass/fail boundary:** Unchanged. Every error path that returned
   `Err` before still returns `Err` after. Every success path that
   returned `Ok` before still returns `Ok` after. No new error paths
   were added; no existing error paths were removed.

## Verdict

**No breaking or behavioral changes.** The error type changes are
contained within the `Box<dyn Error>` trait object layer and do not
affect the `InstructionError` variant returned to the transaction
pipeline, the bank hash, log messages, or any consensus-critical
behavior.
