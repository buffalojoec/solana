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