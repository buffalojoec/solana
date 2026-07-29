#!/usr/bin/env python3
"""Plot criterion benchmark groups against each other on a shared axis.

Criterion draws one chart per group, so comparing knobs with different units
means flipping between charts and reconciling their y-scales by eye. This reads
the JSON criterion already wrote and emits a single log-log chart with every
group on it, each knob's input expressed as a percentage of its own maximum.
That makes the question "which input actually costs us anything" a glance
instead of an exercise.

No measurements are taken here, so the chart can never disagree with the
per-group reports it is derived from.

This assumes each group is one input swept toward a meaningful ceiling, which
is what makes "percentage of its own maximum" a fair axis to share. Groups that
are not sweeps — one benchmark per algorithm, say — have no maximum to
normalise against, and the comparison will not mean anything.

Requires matplotlib:

    python3 -m venv .venv && .venv/bin/pip install matplotlib
    .venv/bin/python scripts/criterion-plot.py

Usage:
    scripts/criterion-plot.py [CRITERION_DIR] [-o OUTPUT.svg]
"""

import argparse
import json
import pathlib
import sys

try:
    import matplotlib
except ModuleNotFoundError:
    sys.exit(
        "matplotlib is required.\n"
        "  python3 -m venv .venv && .venv/bin/pip install matplotlib\n"
        "  .venv/bin/python scripts/criterion-plot.py"
    )

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402

# First three categorical slots, validated for all-pairs separation in both
# modes. Light mode's aqua sits under 3:1 on the surface, so every series is
# also direct-labeled rather than identified by color alone.
THEMES = {
    "light": {
        "series": ["#2a78d6", "#eb6834", "#1baf7a"],
        "surface": "#fcfcfb",
        "ink": "#0b0b0b",
        "muted": "#52514e",
        "grid": "#d9d8d4",
    },
    "dark": {
        "series": ["#3987e5", "#d95926", "#199e70"],
        "surface": "#1a1a19",
        "ink": "#ffffff",
        "muted": "#c3c2b7",
        "grid": "#34332f",
    },
}


def collect(criterion_dir):
    """Map each group to its (input value, mean nanoseconds) points."""
    groups = {}
    for benchmark_path in sorted(criterion_dir.glob("*/*/*/new/benchmark.json")):
        estimates_path = benchmark_path.with_name("estimates.json")
        if not estimates_path.exists():
            continue
        benchmark = json.loads(benchmark_path.read_text())
        estimates = json.loads(estimates_path.read_text())
        try:
            value = float(benchmark["value_str"])
        except (KeyError, ValueError):
            continue
        groups.setdefault(benchmark["group_id"], []).append(
            (value, estimates["mean"]["point_estimate"])
        )
    for points in groups.values():
        points.sort()
    return groups


def shared_prefix(names):
    """Leading name tokens every group has in common.

    Groups in one chart are usually siblings from a single bench target, so a
    shared prefix identifies the chart rather than any line on it. Dropping it
    keeps the direct labels short enough to fit.
    """
    if len(names) < 2:
        return 0
    tokens = [name.split("_") for name in names]
    matched = 0
    # Stop one short of the shortest name, so no group is left without a label.
    for index in range(min(len(token_list) for token_list in tokens) - 1):
        if len({token_list[index] for token_list in tokens}) > 1:
            break
        matched += 1
    return matched


def to_series(groups):
    """Rescale each group's inputs to a percentage of that group's maximum.

    A log axis cannot place zero, so the zero-input control is dropped from the
    chart. It stays in the table, where it reads as the fixed cost every knob
    starts from.
    """
    strip = shared_prefix(sorted(groups))
    series = []
    for name, points in sorted(groups.items()):
        largest = max(value for value, _ in points)
        if largest <= 0:
            continue
        plotted = [
            (value / largest * 100.0, mean_ns / 1000.0)
            for value, mean_ns in points
            if value > 0
        ]
        if plotted:
            series.append(("_".join(name.split("_")[strip:]), plotted))
    return series


def render(series, title, theme_name, output):
    theme = THEMES[theme_name]
    figure, axes = plt.subplots(figsize=(9.6, 5.4), dpi=140)
    figure.patch.set_facecolor(theme["surface"])
    axes.set_facecolor(theme["surface"])

    for index, (name, points) in enumerate(series):
        xs = [x for x, _ in points]
        ys = [y for _, y in points]
        color = theme["series"][index]
        axes.plot(xs, ys, color=color, linewidth=2, zorder=3, solid_capstyle="round")
        # A surface-colored ring keeps overlapping marks legible where the flat
        # series sit on top of each other.
        axes.plot(
            xs,
            ys,
            "o",
            color=color,
            markersize=6,
            markeredgecolor=theme["surface"],
            markeredgewidth=2,
            zorder=4,
        )

    axes.set_xscale("log")
    axes.set_yscale("log")
    axes.set_xlabel("input, as % of that knob's maximum", color=theme["muted"], fontsize=11)
    axes.set_ylabel("mean time (µs)", color=theme["muted"], fontsize=11)
    axes.set_title(title, color=theme["ink"], fontsize=15, fontweight="600", loc="left", pad=18)
    axes.annotate(
        "Each input as a share of its own protocol maximum · log–log",
        xy=(0, 1.02),
        xycoords="axes fraction",
        color=theme["muted"],
        fontsize=10.5,
    )

    axes.grid(True, which="major", color=theme["grid"], linewidth=1, zorder=0)
    axes.grid(True, which="minor", color=theme["grid"], linewidth=0.5, alpha=0.5, zorder=0)
    axes.tick_params(colors=theme["muted"], labelsize=10)
    for spine in ("top", "right"):
        axes.spines[spine].set_visible(False)
    for spine in ("left", "bottom"):
        axes.spines[spine].set_color(theme["grid"])
    axes.xaxis.set_major_formatter(matplotlib.ticker.FuncFormatter(lambda v, _: f"{v:g}%"))
    axes.yaxis.set_major_formatter(matplotlib.ticker.FuncFormatter(lambda v, _: f"{v:g}"))

    # Anchor the bottom on a decade so the flat series have a gridline to read
    # against rather than floating above an unlabelled axis.
    axes.set_ylim(bottom=1.0)

    # Leave room on the right for the direct labels.
    figure.subplots_adjust(right=0.76)
    label_series(figure, axes, series, theme)

    figure.savefig(output, facecolor=theme["surface"])
    plt.close(figure)


def label_series(figure, axes, series, theme):
    """Direct-label each line, nudging labels apart where lines end together.

    Flat series finish within a few pixels of each other, so without this the
    labels overprint.
    """
    minimum_gap = 15.0 * figure.dpi / 72.0
    figure.canvas.draw()
    ends = sorted(
        (
            (axes.transData.transform((points[-1][0], points[-1][1])), index, name)
            for index, (name, points) in enumerate(series)
        ),
        key=lambda end: end[0][1],
        reverse=True,
    )
    previous = None
    for (_, display_y), index, name in ends:
        target = display_y
        if previous is not None and previous - target < minimum_gap:
            target = previous - minimum_gap
        previous = target
        name_points = series[index][1]
        axes.annotate(
            name,
            (name_points[-1][0], name_points[-1][1]),
            textcoords="offset points",
            xytext=(10, (target - display_y) * 72.0 / figure.dpi),
            va="center",
            color=theme["series"][index],
            fontsize=11,
            fontweight="600",
            annotation_clip=False,
            zorder=5,
        )


def table(groups):
    lines = []
    for name, points in sorted(groups.items()):
        largest = max(value for value, _ in points)
        lines.append(f"\n  {name}")
        lines.append(f"    {'input':>12}  {'% of max':>9}  {'mean':>12}")
        for value, mean_ns in points:
            share = f"{value / largest * 100:.2f}%" if largest else "-"
            lines.append(
                f"    {value:>12,.0f}  {share:>9}  {mean_ns / 1000:>9.3f} us"
            )
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("criterion_dir", nargs="?", default="target/criterion")
    parser.add_argument("-o", "--output")
    parser.add_argument("-t", "--title")
    args = parser.parse_args()

    criterion_dir = pathlib.Path(args.criterion_dir)
    if not criterion_dir.is_dir():
        sys.exit(f"no criterion output at {criterion_dir} — run cargo bench first")

    groups = collect(criterion_dir)
    if not groups:
        sys.exit(f"no numeric-parameter benchmarks under {criterion_dir}")

    series = to_series(groups)
    slots = len(THEMES["light"]["series"])
    if len(series) > slots:
        sys.exit(
            f"{len(series)} groups exceeds the {slots}-slot palette; "
            "fold groups together or facet"
        )

    strip = shared_prefix(sorted(groups))
    subject = "_".join(sorted(groups)[0].split("_")[:strip]) if strip else "benchmark"
    title = args.title or f"{subject} cost by input"

    print(table(groups))
    base = pathlib.Path(args.output) if args.output else criterion_dir / "comparison.svg"
    for theme_name in THEMES:
        suffix = "" if theme_name == "light" else "-dark"
        output = base.with_name(f"{base.stem}{suffix}{base.suffix}")
        render(series, title, theme_name, output)
        print(f"\n  wrote {output}", end="")
    print()


if __name__ == "__main__":
    main()
