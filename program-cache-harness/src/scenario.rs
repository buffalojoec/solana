/// Test epochs are 32 slots: the preparation window opens at slot index 16
/// and the boundary falls on slot 32.
pub const SLOTS_PER_EPOCH: u64 = 32;

#[derive(Clone, Debug)]
pub struct Scenario {
    /// Distinct program addresses, all sharing one no-op ELF.
    pub num_programs: usize,
    /// Concurrent transaction submitter threads.
    pub num_submitter_threads: usize,
    /// Invocations of each program per submitter thread per slot.
    pub num_invocations_per_slot: usize,
    /// Concurrent forks to advanced round-robin.
    pub num_forks: usize,
    /// Slots to walk past the epoch boundary before rerooting.
    pub num_post_boundary_slots: u64,
}

impl Scenario {
    pub fn spike() -> Self {
        Self {
            num_programs: 20,
            num_submitter_threads: 2,
            num_invocations_per_slot: 2,
            num_forks: 2,
            num_post_boundary_slots: 1,
        }
    }
}
