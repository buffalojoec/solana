# VmMemoryPool Under SIMD-0296: In-Depth Security Analysis

## Issue

The `VmMemoryPool` in `program-runtime/src/mem_pool.rs` is sized to
`MAX_INSTRUCTION_STACK_DEPTH` (5), but SIMD-0296 allows up to 9 nested
instruction frames. If all frames are SBF programs, the pool is exhausted
at depth 6 and falls back to fresh allocations for depths 6-9.

## Pool Architecture

```rust
pub struct VmMemoryPool {
    stack: Pool<AlignedMemory<{ HOST_ALIGN }>, MAX_INSTRUCTION_STACK_DEPTH>,  // SIZE = 5
    heap: Pool<AlignedMemory<{ HOST_ALIGN }>, MAX_INSTRUCTION_STACK_DEPTH>,   // SIZE = 5
}
```

The pool is **thread-local** (persists across transactions on the same
thread) and operates as a LIFO stack. Pre-allocated on first use:
- 5 stack buffers: `STACK_FRAME_SIZE * MAX_CALL_DEPTH` = 4,096 * 64 = **256 KB each**
- 5 heap buffers: `MAX_HEAP_FRAME_BYTES` = **256 KB each**
- Total pool memory: 5 * 2 * 256 KB = **2.5 MB per thread**

## Allocation Lifecycle

Each `execute()` call:
1. `create_vm!` → `pool.get_stack()` + `pool.get_heap()`
2. `vm.execute_program()` — nested CPIs re-enter `execute()`
3. `pool.put_stack()` + `pool.put_heap()` after program returns

The get/put cycle is nested (LIFO). Only SBF programs use the pool —
builtins use a mock VM with empty memory mapping.

## Worst Case: 9 SBF Programs Nested

```
Depth 1: get_stack → pool[4] taken (4 remaining)
  Depth 2: get_stack → pool[3] taken (3 remaining)
    Depth 3: get_stack → pool[2] taken (2 remaining)
      Depth 4: get_stack → pool[1] taken (1 remaining)
        Depth 5: get_stack → pool[0] taken (0 remaining)
          Depth 6: get_stack → pool EMPTY → fresh alloc (256 KB)
            Depth 7: get_stack → pool EMPTY → fresh alloc (256 KB)
              Depth 8: get_stack → pool EMPTY → fresh alloc (256 KB)
                Depth 9: get_stack → pool EMPTY → fresh alloc (256 KB)
```

Then unwinding (innermost returns first):
```
                Depth 9: put_stack → pool[0] = depth9_buf, next_empty = 1
              Depth 8: put_stack → pool[1] = depth8_buf, next_empty = 2
            Depth 7: put_stack → pool[2] = depth7_buf, next_empty = 3
          Depth 6: put_stack → pool[3] = depth6_buf, next_empty = 4
        Depth 5: put_stack → pool[4] = depth5_buf, next_empty = 5 (FULL)
      Depth 4: put_stack → next_empty=5=SIZE → items[5] OOB → returns false → DROPPED
    Depth 3: put_stack → dropped
  Depth 2: put_stack → dropped
Depth 1: put_stack → dropped
```

**Result:** Pool ends up with 4 fresh + 1 original buffer. 4 original
buffers are dropped (freed). Same pattern for heap.

## Security Assessment

### 1. Correctness

**No issue.** `get_stack()` and `get_heap()` handle exhaustion gracefully:

```rust
pub fn get_stack(&mut self, size: usize) -> AlignedMemory<{ HOST_ALIGN }> {
    self.stack
        .get()
        .unwrap_or_else(|| AlignedMemory::zero_filled(size))
}
```

Pool empty → `None` → `unwrap_or_else` → fresh zero-filled allocation.
The fresh allocation is functionally identical to a pooled one.

### 2. Memory Safety

**No issue.** All allocations (pooled and fresh) are:
- Zero-initialized (`AlignedMemory::zero_filled`)
- Correctly sized (`STACK_FRAME_SIZE * MAX_CALL_DEPTH` for stack,
  `MAX_HEAP_FRAME_BYTES` for heap)
- Properly aligned (`HOST_ALIGN`)
- Dropped when not returned to pool (no leak — freed by Rust drop)

### 3. Memory Exhaustion / DoS

**No issue.** Maximum extra allocation per transaction:
- 4 extra stacks * 256 KB = 1 MB
- 4 extra heaps * 256 KB = 1 MB
- Total extra: **2 MB per deep-nesting transaction**

This is bounded by the CPI nesting limit (9) and is modest compared to
normal transaction memory usage (accounts, serialization buffers, etc.).
Each CPI also consumes `invoke_units` compute units, so the total depth
is CU-limited.

### 4. Pool Churn

**No issue.** After a deep-nesting transaction, the pool contains a mix
of fresh and original buffers. Since all buffers are the same size and
zeroed on `put` (via `Reset::reset`), there is no functional difference.
Subsequent transactions reuse pooled buffers normally.

The steady state after repeated deep-nesting transactions: pool always
has 5 entries, 4 extra allocs per deep transaction are dropped at the
end. No accumulating debt.

### 5. `put` Return Value Silently Discarded

**No issue for safety.** At `vm.rs:268`:
```rust
memory_pool.put_stack(stack);
```
When the pool is full, `put_stack` returns `false` and the buffer is
dropped. The return value is ignored. This is correct — there's nothing
useful to do with a failed put except drop the buffer, which Rust does
automatically.

### 6. Debug Assertion Is a Tautology

**Not a security issue, but a code quality note.** At `vm.rs:270-271`:
```rust
debug_assert!(memory_pool.stack_len() <= MAX_INSTRUCTION_STACK_DEPTH);
debug_assert!(memory_pool.heap_len() <= MAX_INSTRUCTION_STACK_DEPTH);
```

`stack_len()` returns `SIZE` (the const generic = 5), not the current
occupancy. This assertion is always `5 <= 5` — it can never fire. The
intent was likely to check that we're not putting back more than we took,
but the implementation checks pool capacity (constant) not pool fill
level.

### 7. Builtin Programs Don't Use Pool

**Confirmed safe.** The `process_executable_chain` path for builtins
creates a mock VM with `empty_memory_mapping` and zero stack size. It
does not call `MEMORY_POOL.get_stack()`. So in a mixed chain
(SBF→Builtin→SBF), only the SBF levels consume pool entries.

This means the actual pool pressure depends on how many of the 9 nesting
levels are SBF programs. A chain like SBF→Builtin→SBF→Builtin→... would
only consume ~5 pool entries even at full depth.

## Verdict

**No security vulnerability.** The pool gracefully handles over-capacity
via fallback allocation. Memory is bounded, correctly initialized, and
properly freed. The only impact is:

1. **Performance:** 4 extra 256 KB allocations + zeroing at depth 6-9.
   Cost: ~microseconds per alloc. Negligible vs CPI overhead.
2. **Pool replacement:** After a deep-nesting transaction, 4 of the 5
   pooled buffers are replaced with fresh ones. No functional difference.
3. **Debug assertion:** The tautological debug_assert is a missed
   opportunity for a useful check but is not a safety concern.

### Recommendations (non-blocking)

1. **Size the pool to `MAX_INSTRUCTION_STACK_DEPTH_SIMD_0296` (9)** to
   eliminate fallback allocations entirely. This would increase the
   thread-local pool from 2.5 MB to 4.5 MB — a modest increase for
   avoiding per-transaction allocation churn.

2. **Fix the debug_assert** to check actual occupancy instead of
   capacity, or remove it. Currently it provides no value.
