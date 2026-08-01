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
    solana_pubkey::Pubkey,
    solana_runtime::{
        bank::Bank,
        genesis_utils::{GenesisConfigInfo, create_genesis_config},
    },
    solana_sdk_ids::sysvar,
    std::{hint::black_box, sync::Arc, time::Duration},
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

struct Compact {
    epoch: u64,
    first_slot: u64,
    leaders: Vec<Pubkey>,
    windows: Vec<u16>,
}

struct Crude {
    epoch: u64,
    first_slot: u64,
    windows: Vec<(u64, Pubkey)>,
}

type Upcoming = [(u64, Pubkey); 2];

fn serialize_compact(schedule: &Compact, out: &mut [u8]) -> usize {
    let (header, body) = out.split_at_mut(COMPACT_HEADER_LEN);
    header[0..8].copy_from_slice(&schedule.epoch.to_le_bytes());
    header[8..16].copy_from_slice(&schedule.first_slot.to_le_bytes());
    header[16..20].copy_from_slice(&(schedule.leaders.len() as u32).to_le_bytes());
    header[20..24].copy_from_slice(&(schedule.windows.len() as u32).to_le_bytes());
    header[24] = SLOTS_PER_WINDOW;
    header[25] = INDEX_WIDTH;
    header[26..32].fill(0);

    let (table, index) = body.split_at_mut(32 * schedule.leaders.len());
    for (leader, chunk) in schedule.leaders.iter().zip(table.chunks_exact_mut(32)) {
        chunk.copy_from_slice(leader.as_ref());
    }
    for (window, chunk) in schedule.windows.iter().zip(index.chunks_exact_mut(2)) {
        chunk.copy_from_slice(&window.to_le_bytes());
    }

    COMPACT_HEADER_LEN + 32 * schedule.leaders.len() + INDEX_WIDTH as usize * schedule.windows.len()
}

fn serialize_crude(schedule: &Crude, out: &mut [u8]) -> usize {
    let (header, body) = out.split_at_mut(CRUDE_HEADER_LEN);
    header[0..8].copy_from_slice(&schedule.epoch.to_le_bytes());
    header[8..16].copy_from_slice(&schedule.first_slot.to_le_bytes());
    header[16..20].copy_from_slice(&(schedule.windows.len() as u32).to_le_bytes());
    header[20] = SLOTS_PER_WINDOW;
    header[21..24].fill(0);

    for ((slot, leader), chunk) in schedule
        .windows
        .iter()
        .zip(body.chunks_exact_mut(ENTRY_LEN))
    {
        chunk[..8].copy_from_slice(&slot.to_le_bytes());
        chunk[8..].copy_from_slice(leader.as_ref());
    }

    CRUDE_HEADER_LEN + ENTRY_LEN * schedule.windows.len()
}

fn serialize_upcoming(upcoming: &Upcoming, out: &mut [u8]) -> usize {
    for ((slot, leader), chunk) in upcoming.iter().zip(out.chunks_exact_mut(ENTRY_LEN)) {
        chunk[..8].copy_from_slice(&slot.to_le_bytes());
        chunk[8..].copy_from_slice(leader.as_ref());
    }

    ENTRY_LEN * upcoming.len()
}

fn num_windows(schedule: &LeaderSchedule) -> usize {
    schedule.num_slots() / SLOTS_PER_WINDOW as usize
}

fn window_leader(schedule: &LeaderSchedule, window: usize) -> Pubkey {
    schedule
        .get_slot_leader_at_index(window * SLOTS_PER_WINDOW as usize)
        .id
}

fn derive_compact(schedule: &LeaderSchedule) -> Compact {
    let window_leaders = (0..num_windows(schedule))
        .map(|window| window_leader(schedule, window))
        .collect::<Vec<_>>();

    let mut leaders = window_leaders.clone();
    leaders.sort_unstable();
    leaders.dedup();

    let windows = window_leaders
        .iter()
        .map(|leader| leaders.binary_search(leader).unwrap() as u16)
        .collect();

    Compact {
        epoch: EPOCH,
        first_slot: FIRST_SLOT,
        leaders,
        windows,
    }
}

fn derive_crude(schedule: &LeaderSchedule) -> Crude {
    let windows = (0..num_windows(schedule))
        .map(|window| {
            (
                FIRST_SLOT + window as u64 * SLOTS_PER_WINDOW as u64,
                window_leader(schedule, window),
            )
        })
        .collect();

    Crude {
        epoch: EPOCH,
        first_slot: FIRST_SLOT,
        windows,
    }
}

fn derive_upcoming(schedule: &LeaderSchedule, window: usize) -> Upcoming {
    let num_windows = num_windows(schedule);
    let entry = |window: usize| {
        let window = window % num_windows;
        (
            FIRST_SLOT + window as u64 * SLOTS_PER_WINDOW as u64,
            window_leader(schedule, window),
        )
    };

    [entry(window), entry(window + 1)]
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

// Profiles memory derivation from the runtime's `LeaderSchedule` data
// structure to each in-memory layout, for one epoch of updates.
fn bench_derive(c: &mut Criterion) {
    let schedule = setup_leader_schedule();

    let mut group = c.benchmark_group("leader_schedule_sysvar/derive");
    group
        .sample_size(10)
        .measurement_time(Duration::from_secs(10));

    group.bench_function("compact", |b| {
        b.iter(|| black_box(derive_compact(&schedule)))
    });

    group.bench_function("crude", |b| b.iter(|| black_box(derive_crude(&schedule))));

    group.bench_function("upcoming", |b| {
        b.iter(|| {
            for window in 0..num_windows(&schedule) {
                black_box(derive_upcoming(&schedule, window));
            }
        })
    });

    group.finish();
}

// Profiles derivation and actual serialization of the sysvar data.
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
            let compact = derive_compact(&schedule);
            serialize_compact(&compact, &mut compact_buffer);
            black_box(&compact_buffer);
        })
    });

    group.bench_function("crude", |b| {
        b.iter(|| {
            let crude = derive_crude(&schedule);
            serialize_crude(&crude, &mut crude_buffer);
            black_box(&crude_buffer);
        })
    });

    group.bench_function("upcoming", |b| {
        b.iter(|| {
            for window in 0..num_windows(&schedule) {
                let upcoming = derive_upcoming(&schedule, window);
                serialize_upcoming(&upcoming, &mut upcoming_buffer);
                black_box(&upcoming_buffer);
            }
        })
    });

    group.finish();
}

/// Full sysvar update path: derive, serialize, update account.
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
            let compact = derive_compact(&schedule);
            let mut data = vec![0u8; COMPACT_LEN];
            serialize_compact(&compact, &mut data);
            store_sysvar(&bank, &compact_id, Arc::new(data));
            bank.finish_accounts_lt_hash_updates();
        })
    });

    group.bench_function("crude", |b| {
        let bank = setup_bank();
        b.iter(|| {
            let crude = derive_crude(&schedule);
            let mut data = vec![0u8; CRUDE_LEN];
            serialize_crude(&crude, &mut data);
            store_sysvar(&bank, &crude_id, Arc::new(data));
            bank.finish_accounts_lt_hash_updates();
        })
    });

    group.bench_function("upcoming", |b| {
        let bank = setup_bank();
        b.iter(|| {
            for window in 0..num_windows(&schedule) {
                let upcoming = derive_upcoming(&schedule, window);
                let mut data = vec![0u8; UPCOMING_LEN];
                serialize_upcoming(&upcoming, &mut data);
                store_sysvar(&bank, &upcoming_id, Arc::new(data));
            }
            bank.finish_accounts_lt_hash_updates();
        })
    });

    group.finish();
}

criterion_group!(benches, bench_derive, bench_serialize, bench_bank_update);
criterion_main!(benches);
