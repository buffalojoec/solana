# Audit Procedure

Procedure for auditing Joe C's sensitive PRs in anza-xyz/agave.

## Tickets

### Refactoring (complete)

| Ticket | PRs | Verdict | Reports |
|--------|-----|---------|---------|
| **CPI refactor** | #5559, #7836, #7861, #7941, #7983, #7998, #8046 | 2 PASS, 5 OKAY | `audit/refactor/cpi_refactor/` |
| **Deploy refactor** | #9588 (+ 2 already reviewed) | PASS | `audit/refactor/deploy_refactor/` |
| **VM Invoke refactor** | #9593 (+ 2 already reviewed) | PASS | `audit/refactor/vm_invoke_refactor/` |

### Implementation (pending)

| Ticket | PRs | Focus Area |
|--------|-----|------------|
| **SIMD-0296: Raise CPI Nesting Limit** | #6487 | Raises CPI nesting limit from 4 to 8 |
| **Harden native_invoke** | #10681 | native_invoke takes signer seeds |
| **GetEpochStake syscall** | #889 (draft, superseded), #1152 (merged) | New syscall for querying epoch stake |

---

## Shared Procedures

### Diff Extraction

Run `./audit/extract-diff.sh <pr_number>` for each PR. Outputs:
- `audit/diffs/PR_<number>.stat` — file-level change summary with commit metadata
- `audit/diffs/PR_<number>.diff` — full unified diff

### Verdicts

Each PR receives one of:

| Verdict | Meaning |
|---------|---------|
| **PASS** | No behavioral changes (refactor) or correct implementation (feature). |
| **OKAY** | Minor behavioral changes that are intentional, correct, and assessed safe. |
| **FLAG** | Changes that need further human review. |
| **FAIL** | Changes that appear incorrect or introduce risk. |

### Report Format

Each PR audit produces a report at `audit/<category>/<ticket_name>/PR_<number>.md`:

1. **Header** — PR number, title, merge commit, date
2. **Summary** — what the PR does
3. **File Summary** — table of files changed with classification
4. **Behavioral Analysis** — detailed analysis of any logic changes
5. **Security Checklist** — domain-specific checklist
6. **Verdict** — final assessment with rationale

### General Security Checklist

Applied to every PR regardless of audit type:

- [ ] No consensus-breaking changes without feature gates
- [ ] No non-deterministic operations introduced in consensus paths
- [ ] No `unwrap()` / `expect()` introduced in production code paths
- [ ] No `debug_assert!` replacing proper error returns
- [ ] No unsafe code introduced without justification
- [ ] Error codes assessed for consensus impact (see `refactor/cpi_refactor/ERROR_ANALYSIS.md`)

---

## Refactoring Audits

### Goal

Verify that each refactoring PR introduced **zero behavioral changes**.
Any variance between the before and after states must be identified,
classified, and assessed.

### Change Classification

For each file in the diff, classify changes:

**Category A: Pure Moves** — code relocated with no modifications other
than import paths, visibility modifiers, and `#[cfg(test)]` adjustments.
Verify by stripping imports and comparing byte-for-byte.

**Category B: Structural Changes** — signatures, types, or module
organization that don't alter runtime behavior (renaming, reordering
params, splitting functions). Trace call sites to confirm equivalence.

**Category C: Behavioral Changes** — anything that could alter runtime
behavior (control flow, error handling, arithmetic, validation,
constants, feature gates). Each requires individual assessment with
before/after code blocks.

**Category D: Test-Only Changes** — confined to `#[cfg(test)]` modules.
Review for removed/weakened assertions.

### Domain-Specific Checklists

**CPI:**
- [ ] Signer privilege checks preserved
- [ ] Account ownership validation preserved
- [ ] CPI depth/nesting enforcement preserved
- [ ] Account data serialization/deserialization unchanged
- [ ] Lamport balance invariants maintained
- [ ] Program ID validation on CPI targets preserved
- [ ] `is_writable` checks preserved
- [ ] PDA signer seed validation preserved
- [ ] Account realloc bounds preserved
- [ ] Duplicate account handling preserved

**Deploy:**
- [ ] Program validation checks preserved (ELF verification)
- [ ] Authority/signer checks on upgrade operations preserved
- [ ] Rent/size calculations preserved
- [ ] Program cache invalidation logic preserved
- [ ] Deployment CU metering preserved

**VM Invoke:**
- [ ] VM execution parameters (heap size, stack depth, CU limit) preserved
- [ ] Error propagation from VM to caller preserved
- [ ] Compute meter deduction logic preserved
- [ ] Return data handling preserved
- [ ] Log collection preserved

### Refactor Execution Order

#### CPI Refactor

| # | Verdict | PR | Commit | Title |
|---|---------|-----|--------|-------|
| 1 | **PASS** | #5559 | `92687e91ed` | Hoist syscalls into their own crate |
| 2 | **OKAY** | #7836 | `592bb0cb12` | Create new memory module |
| 3 | **OKAY** | #7861 | `3b9a881b7c` | Create new cpi module |
| 4 | **OKAY** | #7941 | `e9670b69fb` | Port over account update functions and tests |
| 5 | **OKAY** | #7983 | `1c06bd3601` | Port over translate account functions |
| 6 | **PASS** | #7998 | `8c90cc1548` | Port over SyscallInvokeSigned |
| 7 | **OKAY** | #8046 | `8b52ec885b` | Port over remaining CPI implementation |

#### Deploy Refactor

| # | Verdict | PR | Commit | Title |
|---|---------|-----|--------|-------|
| 1 | **PASS** | #9588 | `d6273a5fc2` | Extract cache deploy to program-runtime |

#### VM Invoke Refactor

| # | Verdict | PR | Commit | Title |
|---|---------|-----|--------|-------|
| 1 | **PASS** | #9593 | `a5b22fd5ea` | Extract vm execute to program-runtime |

---

## Implementation Audits

### Goal

Verify that each implementation PR is correct, secure, and properly
feature-gated. Unlike refactor audits where the goal is "zero behavioral
changes", implementation audits expect behavioral changes and must assess
whether those changes are correct.

### Approach

Implementation audits are direct code reviews, not diff-based move
verification. For each PR:

1. **Understand the specification** — read the SIMD (if applicable),
   PR description, and any linked design docs.
2. **Read the implementation** — review the current code in the repo
   (not just the diff), understanding how the feature integrates with
   surrounding code.
3. **Assess correctness** — verify the implementation matches the spec.
4. **Assess security** — apply domain-specific security review.
5. **Assess feature gating** — verify consensus-breaking changes are
   properly gated.

### Implementation Execution Order

| # | Verdict | PR | Commit | Title |
|---|---------|-----|--------|-------|
| 1 | | #6487 | `99c82435bd` | SIMD-0296: Raise CPI Nesting Limit to 8 |
| 2 | | #10681 | `76e33c3369` | Harden native_invoke to take signer seeds |
| 3 | | #1152 | `b1508010c0` | GetEpochStake syscall |

---

## Tooling

### Per-file diff
```bash
git diff <commit>^..<commit> -- <specific_file>
```

### Move detection (refactor audits)
```bash
# Extract removed lines (strip leading -)
git diff <commit>^..<commit> -- <source_file> | grep '^-' | sed 's/^-//'
# Extract added lines (strip leading +)
git diff <commit>^..<commit> -- <dest_file> | grep '^+' | sed 's/^+//'
# Diff the two
diff <(removed) <(added)
```
