# Benchmarks

```sh
AGAVE_BENCH_PROGRAMS_DIR=<dir> cargo bench -p solana-program-runtime-bench
```

## Baseline — `033a81af7a`

`ProgramCacheEntry::new` (load, verify, JIT) against `ProgramCacheEntry::reload`
(the same, minus verification). Sizes in KiB, times are criterion mean
estimates.

`noop_aligned` (2 KiB, embedded): `new` 18 us, `reload` 20 us. Too small for
verification to register.

### v0

| Program | Size | `new` | `reload` | Saved |
| --- | ---: | ---: | ---: | ---: |
| `D9ek6qwZgvbksJLzXeG9jaNFJgdp68A3iC5yLynieJQp` | 31 | 175 us | 154 us | 12% |
| `MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr` | 73 | 382 us | 332 us | 13% |
| `ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL` | 103 | 561 us | 462 us | 18% |
| `TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA` | 106 | 565 us | 464 us | 18% |
| `D67re8wUwwZ12ni1fMbzaqwfcG3atiRrMMvptEZmENGs` | 200 | 1.03 ms | 830 us | 20% |
| `LGDSXVcDx4Ynw7UXavGEe5nwzyUZZ5d3sLkwYk26LUf` | 450 | 2.35 ms | 1.96 ms | 17% |
| `darkr3FB87qAZmgLwKov6Hk9Yiah5UT4rUYu8Zhthw1` | 800 | 3.97 ms | 3.55 ms | 11% |
| `TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb` | 1350 | 3.36 ms | 2.75 ms | 18% |
| `4MangoMjqJ2firMokCjjGgoK8d4MXcrgL7XJaL3w6fVg` | 3502 | 18.90 ms | 15.85 ms | 16% |
| `FLASH6Lo6h3iasJKWDs2F8TkW2UKf3s15C8PMGuVfgBn` | 6893 | 48.47 ms | 43.19 ms | 11% |
| `UMBRAD2ishebJTcgCLkTkNUx1v3GyoAgpTRPeWoLykh` | 7059 | 58.38 ms | 56.94 ms | 2% |

### v3

| Program | Size | `new` | `reload` | Saved |
| --- | ---: | ---: | ---: | ---: |
| `FmGfWtigbVnYrqryq5z5GdCzCc3XcFa36h86P26qncr1` | 29 | 123 us | 93 us | 24% |
| `53o2tVBfNXj4DgmDKjUWPC9Hszw6zYNG11CY66irhU74` | 81 | 401 us | 317 us | 21% |
| `3XjiiaQhwpu1NccV4dVGc9LqbmKGqfJNCbSj3KnXyCSR` | 113 | 470 us | 414 us | 12% |
| `5zqNuvXY7yLtM1KsjxwFgFNxR56Kfzrg3auFdV7viEcP` | 198 | 860 us | 710 us | 17% |
| `vuHFdYXjv9ePz6CRGXyQRzRfRLX3yyT8zG5hGiUpwF6` | 342 | 1.45 ms | 1.19 ms | 18% |
| `45s36RbsPudmfu82YhE7WXDWzcyJvppfxKYUgCXM6sB5` | 676 | 2.83 ms | 2.41 ms | 15% |
| `FYaHz8zsZzZJetMmU1uxwfzkU8aryPoWyFsSbm69D44G` | 1167 | 4.99 ms | 4.19 ms | 16% |
| `LendVMybdnkGL9yX9VFJamrtCSzL3izpUoB9JDhSU6M` | 1186 | 5.05 ms | 4.18 ms | 17% |
| `CQwWoJENUtKmwCMqnyGbEYkg41oxdat23kkNdJLvY7v9` | 3346 | 11.72 ms | 8.99 ms | 23% |

## sbpf `speed-up-elf-parsing`

Entries below walk https://github.com/buffalojoec/sbpf, one commit at a time.
Each was measured on the same machine with the same corpus, unpinned, one run
per commit. Δ is against the previous entry, `vs baseline` against the table
above.

Two things to read these against. `Executable::load` is only 10-19% of a
`ProgramCacheEntry::new` -- JIT is roughly two thirds -- so a halving of the
parse path is worth about -5% end to end. And repeat runs of one unchanged
build on this machine move by +-10%. Every entry below is therefore at or under
the noise floor, and no single entry is individually conclusive.

Where a commit touches only one parser, the other acts as a control: v3 does not
go through `relocate`, and `reload` does not verify. Drift moves both together,
so a gap between treatment and control carries more than either number alone.

| Commit | v0 `new` | v0 `reload` | v3 `new` | v3 `reload` |
| --- | ---: | ---: | ---: | ---: |
| `dc2d733` skip full instruction decode in the call relocation pass | +1% | +3% | +9% | +10% |
| `bc9f2d4` index the function registry by hash instead of by order | -2% | -4% | +2% | +3% |
| `fa8961e` register each call target once instead of once per call site | -3% | -4% | -2% | -6% |
| `b72623a` let the caller hand over the ELF buffer | +3% | +2% | -0% | +0% |
| `27cf5ec` verifier: decode instructions from fixed size slots | -4% | +6% | +1% | +4% |
| **cumulative vs baseline** | **-7%** | **-3%** | **+6%** | **+13%** |

Medians over the corpus. None needed an Agave code change; the workspace
compiles clean at every commit.

`dc2d733`, `bc9f2d4` and `fa8961e` are the v0 relocation work, which sbpf
measures at -47%, -29% and -25% on v0 `load` for a cumulative -75%. Agave sees
v0 land at -7% while v3 drifts to +6%, a gap of roughly thirteen points in the
direction the change predicts, spread over three commits that are each too small
to separate from drift on their own.

`b72623a` adds `load_owned` alongside `load` rather than changing it, so nothing
in Agave calls it and the flat result is expected. Adopting it means handing
`ProgramCacheEntry::new` an `AlignedMemory` the caller already owns, which the
account data path does not currently produce. sbpf measures the new entry point
at -99% on v3 `load`, and v3 `load` is a memcpy, so the ceiling here is the
memcpy -- a few hundred microseconds on the largest v3 program.

`27cf5ec` is the only entry whose control is `reload` rather than v3, since
`reload` skips the verifier entirely. v0 `new` -4% against v0 `reload` +6% is
the widest treatment-control gap of the five, and it is the one entry that
should move both parsers: sbpf measures -7% v0 and -9% v3 on `verify`.

### Cumulative — `27cf5ec` vs baseline

| Program | Size | `new` | Baseline | Δ |
| --- | ---: | ---: | ---: | ---: |
| `D9ek6qwZgvbksJLzXeG9jaNFJgdp68A3iC5yLynieJQp` | 31 | 168 us | 175 us | -4% |
| `MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr` | 73 | 343 us | 382 us | -10% |
| `ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL` | 103 | 504 us | 561 us | -10% |
| `TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA` | 106 | 565 us | 565 us | -0% |
| `D67re8wUwwZ12ni1fMbzaqwfcG3atiRrMMvptEZmENGs` | 200 | 968 us | 1.03 ms | -6% |
| `LGDSXVcDx4Ynw7UXavGEe5nwzyUZZ5d3sLkwYk26LUf` | 450 | 2.14 ms | 2.35 ms | -9% |
| `darkr3FB87qAZmgLwKov6Hk9Yiah5UT4rUYu8Zhthw1` | 800 | 3.80 ms | 3.97 ms | -4% |
| `TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb` | 1350 | 2.97 ms | 3.36 ms | -12% |
| `4MangoMjqJ2firMokCjjGgoK8d4MXcrgL7XJaL3w6fVg` | 3502 | 16.67 ms | 18.90 ms | -12% |
| `FLASH6Lo6h3iasJKWDs2F8TkW2UKf3s15C8PMGuVfgBn` | 6893 | 45.31 ms | 48.47 ms | -7% |
| `UMBRAD2ishebJTcgCLkTkNUx1v3GyoAgpTRPeWoLykh` | 7059 | 57.98 ms | 58.38 ms | -1% |

v3 over the same span moved +3%..+14%, which bounds what the v0 column above can
be trusted to mean.
