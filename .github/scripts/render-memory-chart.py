#!/usr/bin/env python3
"""Renders memory.json (one entry per release) as a line chart SVG.

usage: render-memory-chart.py memory.json memory.svg
"""
import json
import sys
from html import escape

SERIES = [
    ("idle", "Tray idle", "--s1"),
    ("main_open", "Main window open", "--s2"),
]
MIB = 1024 * 1024
W, H = 800, 380
LEFT, RIGHT, TOP, BOTTOM = 56, 190, 64, 56


def load(path):
    with open(path, encoding="utf-8") as handle:
        data = json.load(handle)
    return data.get("runs", [])


def value(run, key):
    stage = (run.get("stages") or {}).get(key) or {}
    raw = stage.get("private_ws")
    return None if raw is None else raw / MIB


def nice_max(top):
    for step in (10, 20, 25, 50, 100, 200, 250, 500):
        if top <= step:
            return step
    return (int(top / 100) + 1) * 100


def render(runs):
    plot_w = W - LEFT - RIGHT
    plot_h = H - TOP - BOTTOM
    values = [v for run in runs for key, _, _ in SERIES if (v := value(run, key)) is not None]
    y_max = nice_max(max(values) * 1.1 if values else 10)
    ticks = [y_max * i / 4 for i in range(5)]

    def x_at(index):
        if len(runs) == 1:
            return LEFT + plot_w / 2
        return LEFT + plot_w * index / (len(runs) - 1)

    def y_at(v):
        return TOP + plot_h * (1 - v / y_max)

    out = []
    add = out.append
    add(f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}" role="img" aria-labelledby="t d">')
    add("<title id=\"t\">Snapline memory use by release</title>")
    latest = runs[-1] if runs else None
    summary = "No measurements yet."
    if latest:
        parts = []
        for key, label, _ in SERIES:
            v = value(latest, key)
            if v is not None:
                parts.append(f"{label.lower()} {v:.1f} MB")
        summary = f"Version {latest['version']}: " + ", ".join(parts) + "."
    add(f'<desc id="d">Private working set per release. {escape(summary)}</desc>')
    add("""<style>
svg{--bg:#fcfcfb;--ink:#0b0b0b;--ink2:#52514e;--grid:#e6e5e1;--s1:#2a78d6;--s2:#eb6834;--s3:#1baf7a;font-family:-apple-system,"Segoe UI",Helvetica,Arial,sans-serif}
@media (prefers-color-scheme:dark){svg{--bg:#1a1a19;--ink:#f0efec;--ink2:#c3c2b7;--grid:#383835;--s1:#3987e5;--s2:#d95926;--s3:#199e70}}
.bg{fill:var(--bg)}.ink{fill:var(--ink)}.ink2{fill:var(--ink2)}.grid{stroke:var(--grid);stroke-width:1}
.line{fill:none;stroke-width:2;stroke-linejoin:round;stroke-linecap:round}
.dot{stroke:var(--bg);stroke-width:2}
</style>""")
    add(f'<rect class="bg" width="{W}" height="{H}" rx="8"/>')
    add(f'<text class="ink" x="{LEFT}" y="28" font-size="16" font-weight="600">Memory use by release</text>')
    add(f'<text class="ink2" x="{LEFT}" y="47" font-size="12">Private working set in MB, measured in CI on a Windows runner</text>')

    for tick in ticks:
        y = y_at(tick)
        add(f'<line class="grid" x1="{LEFT}" y1="{y:.1f}" x2="{LEFT + plot_w}" y2="{y:.1f}"/>')
        add(f'<text class="ink2" x="{LEFT - 8}" y="{y + 4:.1f}" font-size="11" text-anchor="end">{tick:g}</text>')

    step = max(1, -(-len(runs) // 10))
    for index, run in enumerate(runs):
        if index % step and index != len(runs) - 1:
            continue
        add(f'<text class="ink2" x="{x_at(index):.1f}" y="{TOP + plot_h + 20}" font-size="11" text-anchor="middle">{escape(str(run["version"]))}</text>')

    legend_y = TOP + 8
    for key, label, colour in SERIES:
        points = [(x_at(i), y_at(v), v, run) for i, run in enumerate(runs) if (v := value(run, key)) is not None]
        if len(points) > 1:
            path = " ".join(f"{'M' if n == 0 else 'L'}{x:.1f},{y:.1f}" for n, (x, y, _, _) in enumerate(points))
            add(f'<path class="line" d="{path}" style="stroke:var({colour})"/>')
        for x, y, v, run in points:
            add(f'<circle class="dot" cx="{x:.1f}" cy="{y:.1f}" r="4.5" style="fill:var({colour})"><title>{escape(label)}, {escape(str(run["version"]))}: {v:.1f} MB</title></circle>')
        if points:
            x, y, v, _ = points[-1]
            add(f'<text class="ink" x="{x + 10:.1f}" y="{y + 4:.1f}" font-size="12">{v:.1f} MB</text>')
        add(f'<rect x="{W - RIGHT + 24}" y="{legend_y - 9}" width="10" height="10" rx="2" style="fill:var({colour})"/>')
        add(f'<text class="ink2" x="{W - RIGHT + 40}" y="{legend_y}" font-size="11">{escape(label)}</text>')
        legend_y += 20

    add("</svg>")
    return "\n".join(out) + "\n"


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    runs = sorted(load(sys.argv[1]), key=lambda run: run.get("date", ""))
    with open(sys.argv[2], "w", encoding="utf-8") as handle:
        handle.write(render(runs))


if __name__ == "__main__":
    main()
