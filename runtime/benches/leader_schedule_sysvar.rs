#![allow(clippy::arithmetic_side_effects)]
//! Benchmarking Leader Schedule sysvar account updates across an entire epoch.
//!
//! For sysvars that store the entire leader schedule (compact, crude), this
//! is only one single update at the beginning of the epoch.
//!
//! For the "upcoming leader" sysvar, the update is every time the leader window
//! changes (4 slots), therefore 432,000 / 4 = 108,000 iterations.

use {
    criterion::{Criterion, criterion_group, criterion_main},
    solana_account::AccountSharedData,
    solana_leader_schedule::{LeaderSchedule, NUM_CONSECUTIVE_LEADER_SLOTS, SlotLeader},
    solana_pubkey::{Pubkey, PubkeyHasherBuilder},
    solana_runtime::{
        bank::Bank,
        genesis_utils::{GenesisConfigInfo, create_genesis_config},
    },
    solana_sdk_ids::sysvar,
    std::{collections::HashMap, hint::black_box, sync::Arc, time::Duration},
};

#[cfg(not(any(target_env = "msvc", target_os = "freebsd")))]
#[global_allocator]
static GLOBAL: jemallocator::Jemalloc = jemallocator::Jemalloc;

const SLOTS_PER_WINDOW: u8 = NUM_CONSECUTIVE_LEADER_SLOTS.get() as u8;
const NUM_WINDOWS: u32 = 108_000;
const NUM_LEADERS: u32 = 1_300;
const INDEX_WIDTH: u8 = 2;

const EPOCH: u64 = 1;
const FIRST_SLOT: u64 = 432_000;

const COMPACT_HEADER_LEN: usize = 32;
const CRUDE_HEADER_LEN: usize = 24;
const ENTRY_LEN: usize = 8 + 32;

const COMPACT_LEN: usize =
    COMPACT_HEADER_LEN + 32 * NUM_LEADERS as usize + INDEX_WIDTH as usize * NUM_WINDOWS as usize;
const CRUDE_LEN: usize = CRUDE_HEADER_LEN + ENTRY_LEN * NUM_WINDOWS as usize;
const UPCOMING_LEN: usize = 2 * ENTRY_LEN;

fn num_windows(schedule: &LeaderSchedule) -> usize {
    schedule.num_slots() / SLOTS_PER_WINDOW as usize
}

fn window_leader(schedule: &LeaderSchedule, window: usize) -> Pubkey {
    schedule
        .get_slot_leader_at_index(window * SLOTS_PER_WINDOW as usize)
        .id
}

fn window_slot(window: usize) -> u64 {
    FIRST_SLOT + window as u64 * SLOTS_PER_WINDOW as u64
}

fn serialize_compact(schedule: &LeaderSchedule, out: &mut [u8]) -> usize {
    let num_windows = num_windows(schedule);
    let mut indices = HashMap::<Pubkey, u16, PubkeyHasherBuilder>::default();
    let mut window_indices = Vec::with_capacity(num_windows);
    let mut num_leaders = 0usize;

    for window in 0..num_windows {
        let leader = window_leader(schedule, window);
        let index = match indices.get(&leader) {
            Some(index) => *index,
            None => {
                let index = num_leaders as u16;
                let offset = COMPACT_HEADER_LEN + 32 * num_leaders;
                out[offset..offset + 32].copy_from_slice(leader.as_ref());
                indices.insert(leader, index);
                num_leaders += 1;
                index
            }
        };
        window_indices.push(index);
    }

    let index_offset = COMPACT_HEADER_LEN + 32 * num_leaders;
    for (index, chunk) in window_indices
        .iter()
        .zip(out[index_offset..].chunks_exact_mut(2))
    {
        chunk.copy_from_slice(&index.to_le_bytes());
    }

    out[0..8].copy_from_slice(&EPOCH.to_le_bytes());
    out[8..16].copy_from_slice(&FIRST_SLOT.to_le_bytes());
    out[16..20].copy_from_slice(&(num_leaders as u32).to_le_bytes());
    out[20..24].copy_from_slice(&(num_windows as u32).to_le_bytes());
    out[24] = SLOTS_PER_WINDOW;
    out[25] = INDEX_WIDTH;
    out[26..32].fill(0);

    index_offset + INDEX_WIDTH as usize * num_windows
}

fn serialize_crude(schedule: &LeaderSchedule, out: &mut [u8]) -> usize {
    let num_windows = num_windows(schedule);
    let (header, body) = out.split_at_mut(CRUDE_HEADER_LEN);

    header[0..8].copy_from_slice(&EPOCH.to_le_bytes());
    header[8..16].copy_from_slice(&FIRST_SLOT.to_le_bytes());
    header[16..20].copy_from_slice(&(num_windows as u32).to_le_bytes());
    header[20] = SLOTS_PER_WINDOW;
    header[21..24].fill(0);

    for (window, (leader, chunk)) in schedule
        .get_slot_leaders()
        .step_by(SLOTS_PER_WINDOW as usize)
        .zip(body.chunks_exact_mut(ENTRY_LEN))
        .enumerate()
    {
        chunk[..8].copy_from_slice(&window_slot(window).to_le_bytes());
        chunk[8..].copy_from_slice(leader.id.as_ref());
    }

    CRUDE_HEADER_LEN + ENTRY_LEN * num_windows
}

fn serialize_upcoming(schedule: &LeaderSchedule, window: usize, out: &mut [u8]) -> usize {
    let slot = window_slot(window);

    out[0..8].copy_from_slice(&slot.to_le_bytes());
    out[8..40].copy_from_slice(window_leader(schedule, window).as_ref());
    out[40..48].copy_from_slice(&(slot + SLOTS_PER_WINDOW as u64).to_le_bytes());
    out[48..80].copy_from_slice(window_leader(schedule, window + 1).as_ref());

    UPCOMING_LEN
}

fn setup_leader_schedule() -> LeaderSchedule {
    let leaders = (0..NUM_LEADERS)
        .map(|_| SlotLeader::new_unique())
        .collect::<Vec<_>>();

    let slot_leaders = (0..NUM_WINDOWS)
        .map(|window| leaders[(window % NUM_LEADERS) as usize])
        .collect();

    LeaderSchedule::new_from_schedule(slot_leaders, NUM_CONSECUTIVE_LEADER_SLOTS)
}

fn setup_bank() -> Arc<Bank> {
    let GenesisConfigInfo { genesis_config, .. } = create_genesis_config(1_000_000_000);
    Arc::new(Bank::new_for_tests(&genesis_config))
}

fn store_sysvar(bank: &Bank, pubkey: &Pubkey, data: Arc<Vec<u8>>) {
    let lamports = bank.get_minimum_balance_for_rent_exemption(data.len());
    let account =
        AccountSharedData::create_from_existing_shared_data(lamports, data, sysvar::id(), false, 0);
    bank.store_account(pubkey, &account);
}

// Profiles serialization of the sysvar data from the runtime's `LeaderSchedule`
// data structure, for one epoch of updates.
fn bench_serialize(c: &mut Criterion) {
    let schedule = setup_leader_schedule();

    let mut compact_buffer = vec![0u8; COMPACT_LEN];
    let mut crude_buffer = vec![0u8; CRUDE_LEN];
    let mut upcoming_buffer = vec![0u8; UPCOMING_LEN];

    let mut group = c.benchmark_group("leader_schedule_sysvar/serialize");
    group
        .sample_size(10)
        .measurement_time(Duration::from_secs(10));

    group.bench_function("compact", |b| {
        b.iter(|| {
            serialize_compact(&schedule, &mut compact_buffer);
            black_box(&compact_buffer);
        })
    });

    group.bench_function("crude", |b| {
        b.iter(|| {
            serialize_crude(&schedule, &mut crude_buffer);
            black_box(&crude_buffer);
        })
    });

    group.bench_function("upcoming", |b| {
        b.iter(|| {
            for window in 0..num_windows(&schedule) {
                serialize_upcoming(&schedule, window, &mut upcoming_buffer);
                black_box(&upcoming_buffer);
            }
        })
    });

    group.finish();
}

// Profiles the full sysvar update path: serialize, update account.
fn bench_bank_update(c: &mut Criterion) {
    let schedule = setup_leader_schedule();
    let compact_id = Pubkey::new_unique();
    let crude_id = Pubkey::new_unique();
    let upcoming_id = Pubkey::new_unique();

    let mut group = c.benchmark_group("leader_schedule_sysvar/bank_update");
    group
        .sample_size(10)
        .measurement_time(Duration::from_secs(20));

    group.bench_function("compact", |b| {
        let bank = setup_bank();
        b.iter(|| {
            let mut data = vec![0u8; COMPACT_LEN];
            serialize_compact(&schedule, &mut data);
            store_sysvar(&bank, &compact_id, Arc::new(data));
            bank.finish_accounts_lt_hash_updates();
        })
    });

    group.bench_function("crude", |b| {
        let bank = setup_bank();
        b.iter(|| {
            let mut data = vec![0u8; CRUDE_LEN];
            serialize_crude(&schedule, &mut data);
            store_sysvar(&bank, &crude_id, Arc::new(data));
            bank.finish_accounts_lt_hash_updates();
        })
    });

    group.bench_function("upcoming", |b| {
        let bank = setup_bank();
        b.iter(|| {
            for window in 0..num_windows(&schedule) {
                let mut data = vec![0u8; UPCOMING_LEN];
                serialize_upcoming(&schedule, window, &mut data);
                store_sysvar(&bank, &upcoming_id, Arc::new(data));
            }
            bank.finish_accounts_lt_hash_updates();
        })
    });

    group.finish();
}

criterion_group!(benches, bench_serialize, bench_bank_update);
criterion_main!(benches);
