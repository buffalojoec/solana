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
