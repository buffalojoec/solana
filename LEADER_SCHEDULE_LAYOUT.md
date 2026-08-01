### Leader Schedule: Compact Layout

```
# Header
 0..8    epoch             u64
 8..16   first_slot        u64
16..20   num_leaders       u32   // N (1,300)
20..24   num_windows       u32   // W (108,000)
24..25   slots_per_window  u8    // 4
25..26   index_width       u8    // 2
26..32   _padding          [u8; 6]

# Leader table
[Pubkey; N]

# Window index
[u16; W]
```

Account size: 251.6 KiB
Lookup: `leader(slot) = table[index[(slot - first_slot) / slots_per_window]]`

### Leader Schedule: Crude Layout

```
# Header
 0..8    epoch             u64
 8..16   first_slot        u64
16..20   num_windows       u32   // W (108,000)
20..21   slots_per_window  u8    // 4
21..24   _padding          [u8; 3]

# Simple leader list
[(u64, Pubkey); W]
```

Account size: 4.12 MiB
Lookup: `leader(slot) = list[(slot - first_slot) / slots_per_window].1`

### Upcoming Leader

```
 0..8    current_slot      u64
 8..40   current_leader    Pubkey
 40..48  next_slot         u64
 48..80  next_leader       Pubkey
```

Account size: 80 bytes

### SDK Helper

`sol_get_sysvar` reads a slice at an offset, so a program resolves one leader
without loading the account. Three calls against the compact layout: header,
window index, then the table entry.

```rust
use {solana_address::Address, solana_program_error::ProgramError, solana_sysvar::get_sysvar};

pub fn leader_at_slot(slot: u64) -> Result<Address, ProgramError> {
    let mut header = [0u8; 32];
    get_sysvar(&mut header, &LEADER_SCHEDULE_ID, 0, 32)?;

    let first_slot = u64::from_le_bytes(header[8..16].try_into().unwrap());
    let num_leaders = u64::from(u32::from_le_bytes(header[16..20].try_into().unwrap()));
    let slots_per_window = u64::from(header[24]);

    let window = slot
        .checked_sub(first_slot)
        .ok_or(ProgramError::InvalidArgument)?
        / slots_per_window;

    let mut index = [0u8; 2];
    get_sysvar(&mut index, &LEADER_SCHEDULE_ID, 32 + 32 * num_leaders + 2 * window, 2)?;

    let mut leader = [0u8; 32];
    let entry = u64::from(u16::from_le_bytes(index));
    get_sysvar(&mut leader, &LEADER_SCHEDULE_ID, 32 + 32 * entry, 32)?;

    Ok(Address::from(leader))
}
```

Per call: `sysvar_base_cost + 32/cpi_bytes_per_unit + max(length/cpi_bytes_per_unit,
mem_op_base_cost)` = `100 + 0 + 10` = **110 CU**, flat for any read under 2,500 bytes.

| Layout | Calls | Bytes read | Cost |
| --- | --- | --- | --- |
| Compact | 3 | 66 | 330 CU |
| Crude | 2 | 64 | 220 CU |
| Upcoming | 1 | 80 | 110 CU |

Upcoming is cheapest but only answers for the current and next window; the other
two resolve any slot in the epoch.