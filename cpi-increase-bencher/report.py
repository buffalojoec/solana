#!/usr/bin/env python3
"""Derive baseline comparisons and direct mapping savings from a bench run.

Reads the JSON written by `examples/resources.rs`, and picks up criterion's
timings if `make wall-clock` has been run.
"""

import json
import os
import sys

KIB = 1024
MIB = KIB * KIB


def load_timings(directory):
    """Mean wall clock, keyed the way criterion names its directories."""
    timings = {}
    if not os.path.isdir(directory):
        return timings
    for name in os.listdir(directory):
        estimates = os.path.join(directory, name, "new", "estimates.json")
        if os.path.exists(estimates):
            with open(estimates) as handle:
                timings[name] = json.load(handle)["mean"]["point_estimate"]
    return timings


def size(value):
    if abs(value) >= MIB:
        return f"{value / MIB:.2f} MiB"
    if abs(value) >= KIB:
        return f"{value / KIB:.1f} KiB"
    return f"{value:.0f} B"


def table(label, note, sizes, rows):
    print(f"\n{label}")
    print(f"  {'':>12} " + "".join(f"{size(s):>22}" for s in sizes))
    for name, cells in rows:
        print(f"  {name:>12} " + "".join(f"{c:>22}" for c in cells))
    if note:
        print(f"  {note}")


def main():
    with open(sys.argv[1]) as handle:
        rows = json.load(handle)
    timings = load_timings(sys.argv[2]) if len(sys.argv) > 2 else {}

    for row in rows:
        suffix = "dm_on" if row["direct_mapping"] else "dm_off"
        kib = row["account_data_len"] // KIB
        row["ns"] = timings.get(f"{row['scenario']}_{kib}KiB_{suffix}")

    sizes = sorted({row["account_data_len"] for row in rows})
    scenarios = list(dict.fromkeys(row["scenario"] for row in rows))
    keyed = {
        (row["account_data_len"], row["scenario"], row["direct_mapping"]): row
        for row in rows
    }

    print("\nDirect mapping savings")
    print(
        f"  {'accounts':>9} {'scenario':<10} {'copies avoided':>15} "
        f"{'peak saved':>12} {'CU delta':>9}"
    )
    for account_len in sizes:
        for scenario in scenarios:
            on = keyed.get((account_len, scenario, True))
            off = keyed.get((account_len, scenario, False))
            if not on or not off:
                continue
            saved = off["peak_mapped_bytes"] - on["peak_mapped_bytes"]
            share = saved / off["peak_mapped_bytes"] * 100
            print(
                f"  {size(account_len):>9} {scenario:<10} "
                f"{size(off['copied_bytes']):>15} "
                f"{size(saved) + f' ({share:.0f}%)':>12} "
                f"{off['compute_units'] - on['compute_units']:>+9}"
            )

    depths = sorted({row["depth"] for row in rows})

    def at(account_len, depth):
        return keyed[(account_len, scenario_at(rows, depth), True)]

    def elapsed(account_len, depth):
        scenario = scenario_at(rows, depth)
        on = keyed[(account_len, scenario, True)]["ns"]
        off = keyed[(account_len, scenario, False)]["ns"]
        return f"{on / 1e6:.2f} / {off / 1e6:.2f} ms" if on and off else "-"

    if any(row["ns"] for row in rows):
        table(
            "Wall clock time, direct mapping on / off",
            "Missing cells are account sizes the wall clock bench does not run.",
            sizes,
            [(f"{depth} deep", [elapsed(a, depth) for a in sizes]) for depth in depths],
        )

    # `64x0` makes no CPI at all, so the shallowest nesting is the baseline
    # every deeper scenario is measured against.
    base, deeper = depths[1], depths[2:]
    base_name = scenario_at(rows, base)

    for label, field, fmt in (
        ("Compute units charged", "compute_units", lambda v: f"{v:+.0f}"),
        ("Peak host memory mapped", "peak_mapped_bytes", lambda v: f"{size(v)}"),
    ):
        table(
            f"{label}, versus the {base}-deep baseline ({base_name}), at 64 frames",
            None,
            sizes,
            [
                (
                    f"{base} (base)",
                    [fmt(at(a, base)[field]).lstrip("+") for a in sizes],
                )
            ]
            + [
                (
                    f"{depth} deep",
                    [
                        f"{fmt(at(a, depth)[field] - at(a, base)[field])} "
                        f"({(at(a, depth)[field] / at(a, base)[field] - 1) * 100:+.0f}%)"
                        for a in sizes
                    ],
                )
                for depth in deeper
            ],
        )

    spans = [(base, depth) for depth in deeper] + list(zip(deeper, deeper[1:]))
    table(
        "Peak host memory mapped per compute unit charged",
        "Each span buys more mapped memory per CU than the one before it.",
        sizes,
        [
            (
                f"{was} -> {now}",
                [
                    f"{(at(a, now)['peak_mapped_bytes'] - at(a, was)['peak_mapped_bytes']) / (at(a, now)['compute_units'] - at(a, was)['compute_units']):.1f} B/CU"
                    for a in sizes
                ],
            )
            for was, now in spans
        ],
    )
    print()


def scenario_at(rows, depth):
    return next(row["scenario"] for row in rows if row["depth"] == depth)


if __name__ == "__main__":
    main()
