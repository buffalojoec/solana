# VmMemoryPool Sizing: 5 vs 9

Follow-up to MEMORY_POOL_ANALYSIS.md. Recommendation on whether to resize
the pool after SIMD-0296.

## Where to Look

### Pool definition

`program-runtime/src/mem_pool.rs:14-17` — generic pool struct:
```rust
struct Pool<T: Reset, const SIZE: usize> {
    items: [Option<T>; SIZE],
    next_empty: usize,
}
```

`program-runtime/src/mem_pool.rs:60-63` — pool instantiated at SIZE=5:
```rust
pub struct VmMemoryPool {
    stack: Pool<AlignedMemory<{ HOST_ALIGN }>, MAX_INSTRUCTION_STACK_DEPTH>,
    heap: Pool<AlignedMemory<{ HOST_ALIGN }>, MAX_INSTRUCTION_STACK_DEPTH>,
}
```

`MAX_INSTRUCTION_STACK_DEPTH` is imported at `mem_pool.rs:3` from
`execution_budget.rs:8`:
```rust
pub const MAX_INSTRUCTION_STACK_DEPTH: usize = 5;
```

The SIMD-0296 constant lives at `execution_budget.rs:10`:
```rust
pub const MAX_INSTRUCTION_STACK_DEPTH_SIMD_0296: usize = 9;
```

### Pre-allocation

`program-runtime/src/mem_pool.rs:66-75` — buffers allocated at pool creation:
```rust
pub fn new() -> Self {
    Self {
        stack: Pool::new(array::from_fn(|_| {
            AlignedMemory::zero_filled(STACK_FRAME_SIZE * MAX_CALL_DEPTH)  // 4096 * 64 = 256 KB
        })),
        heap: Pool::new(array::from_fn(|_| {
            AlignedMemory::zero_filled(MAX_HEAP_FRAME_BYTES as usize)     // 256 KB
        })),
    }
}
```

Each pool entry: 256 KB. Two pools (stack + heap).
- SIZE=5: 5 * 2 * 256 KB = **2.5 MB per thread**
- SIZE=9: 9 * 2 * 256 KB = **4.5 MB per thread**

### Thread-local instantiation

`program-runtime/src/vm.rs:29-31`:
```rust
thread_local! {
    pub static MEMORY_POOL: RefCell<VmMemoryPool> = RefCell::new(VmMemoryPool::new());
}
```

Created lazily on first use per thread. Persists for the thread's lifetime.

### Get path (with fallback)

`program-runtime/src/mem_pool.rs:85-90` — stack get:
```rust
pub fn get_stack(&mut self, size: usize) -> AlignedMemory<{ HOST_ALIGN }> {
    debug_assert!(size == STACK_FRAME_SIZE * MAX_CALL_DEPTH);
    self.stack
        .get()
        .unwrap_or_else(|| AlignedMemory::zero_filled(size))
}
```

`program-runtime/src/mem_pool.rs:96-101` — heap get:
```rust
pub fn get_heap(&mut self, heap_size: u32) -> AlignedMemory<{ HOST_ALIGN }> {
    debug_assert!((MIN_HEAP_FRAME_BYTES..=MAX_HEAP_FRAME_BYTES).contains(&heap_size));
    self.heap
        .get()
        .unwrap_or_else(|| AlignedMemory::zero_filled(MAX_HEAP_FRAME_BYTES as usize))
}
```

Pool empty → fresh allocation. Identical buffer. No error.

### Put path (silent drop on full)

`program-runtime/src/mem_pool.rs:41-51` — put returns false when full:
```rust
fn put(&mut self, mut value: T) -> bool {
    self.items
        .get_mut(self.next_empty)
        .map(|item| {
            value.reset();
            item.replace(value);
            self.next_empty = self.next_empty.saturating_add(1);
            true
        })
        .unwrap_or(false)   // full → buffer dropped by Rust
}
```

`program-runtime/src/vm.rs:267-272` — return value discarded:
```rust
MEMORY_POOL.with_borrow_mut(|memory_pool| {
    memory_pool.put_stack(stack);   // bool return ignored
    memory_pool.put_heap(heap);     // bool return ignored
    debug_assert!(memory_pool.stack_len() <= MAX_INSTRUCTION_STACK_DEPTH);
    debug_assert!(memory_pool.heap_len() <= MAX_INSTRUCTION_STACK_DEPTH);
});
```

### Consumption point

`program-runtime/src/vm.rs:135-136` — inside `create_vm!` macro:
```rust
let (mut stack, mut heap) = $crate::__private::MEMORY_POOL
    .with_borrow_mut(|pool| (pool.get_stack(stack_size), pool.get_heap(heap_size)));
```

Only called from `execute()` (`vm.rs:156`), which is only invoked for
SBF programs. Builtins go through `process_executable_chain`
(`invoke_context.rs:540`) which creates a mock VM with empty memory —
does not touch the pool.

### Debug assert (tautology)

`program-runtime/src/vm.rs:270-271`:
```rust
debug_assert!(memory_pool.stack_len() <= MAX_INSTRUCTION_STACK_DEPTH);
```

`mem_pool.rs:27-29` — `len()` returns const SIZE, not occupancy:
```rust
fn len(&self) -> usize {
    SIZE
}
```

Always evaluates to `5 <= 5`. Can never fire.

## Recommendation: Keep at 5

The pool should stay at SIZE=5. Reasons:

**1. The hot path is depth 1-4.**

The overwhelming majority of Solana transactions nest 0-3 CPI levels.
Programs that hit depth 4 are uncommon. The pool at SIZE=5 covers the
entire pre-SIMD-0296 range with zero fallback allocations. This is where
the perf win matters.

**2. Depth 5-8 is a cold path.**

SIMD-0296 exists for complex composability (multi-hop DeFi routing,
protocol aggregators). These transactions are inherently expensive — they
consume significant CUs just from `invoke_units` overhead per CPI level.
A few extra microseconds for fallback allocation is noise against
hundreds of thousands of CUs of program execution.

**3. Pre-allocating for 9 wastes memory on cold buffers.**

Going from 5 to 9 adds 2 MB per thread. On a validator with dozens of
transaction-processing threads, that's 50-100 MB of pre-allocated
buffers that almost never get touched. Pre-allocated but untouched memory
still costs TLB entries and competes with cache. The pool exists to keep
hot buffers warm — adding cold entries defeats the purpose.

**4. The fallback path is correct and cheap.**

`AlignedMemory::zero_filled(256KB)` is a single `mmap` or `calloc`
under the hood. On Linux, the kernel's virtual memory system makes this
near-instant (pages are zero-on-demand). The cost is paid only when the
memory is actually touched during VM execution, at which point you're
already deep in an expensive CPI chain.

**5. No consensus impact.**

The pool is an Agave-internal optimization. Other validator
implementations (Firedancer, etc.) manage VM memory independently. The
SVM specification says nothing about memory pooling. The pool size is
invisible to programs and to consensus. There is no reason to change it
for correctness.

**6. If the distribution shifts, it's trivial to change.**

If SIMD-0296 adoption grows and deep nesting becomes common, increasing
the pool is a one-line change (`MAX_INSTRUCTION_STACK_DEPTH` →
`MAX_INSTRUCTION_STACK_DEPTH_SIMD_0296` at `mem_pool.rs:61-62`). No API
changes, no feature gates, no coordination needed. The data (validator
metrics on CPI depth distribution) should drive the decision.

### Optional: Add a comment

The import at `mem_pool.rs:3` could note the intentional choice:

```rust
use crate::execution_budget::{
    // Intentionally not MAX_INSTRUCTION_STACK_DEPTH_SIMD_0296.
    // Pool sized for the common case (depth 1-4). Deeper nesting
    // falls back to fresh allocation. See MEMORY_POOL_ANALYSIS.md.
    MAX_CALL_DEPTH, MAX_HEAP_FRAME_BYTES, MAX_INSTRUCTION_STACK_DEPTH,
    MIN_HEAP_FRAME_BYTES, STACK_FRAME_SIZE,
};
```
