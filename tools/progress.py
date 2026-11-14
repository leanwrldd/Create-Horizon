"""Measure how much of Create is ported and draw it.

Every `BlockEntry`, `ItemEntry`, dyed list and block set field in Create's `AllBlocks.java` and
`AllItems.java` counts as one feature, grouped by the section comment it sits under (Kinetics,
Fluids, Logistics...). A feature counts as done once `content/` declares its id as a block or item.

    python tools/progress.py --snapshot <create-java-src>   # refresh the feature list from Create
    python tools/progress.py                                # redraw progress.svg and the README table

The feature list is stored in `progress/create_features.json` so redrawing needs no Create checkout.
"""
from __future__ import annotations

import argparse
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SNAPSHOT = ROOT / "progress" / "create_features.json"
SVG = ROOT / "progress" / "progress.svg"
README = ROOT / "README.md"
DECLS = ROOT / "content" / "data" / "create" / "horizon"

ORDER = ["Kinetics", "Materials", "Logistics", "Fluids", "Contraptions", "Schematics",
         "Curiosities", "Palettes", "Shortcuts"]

FIELD = re.compile(r"^\s*public static final (\w+)(?:<.*?>)?\s+(\w+)")
SECTION = re.compile(r"^\s*// ([A-Z][a-z]+)\s*$")
QUOTED = re.compile(r'"([a-z0-9_]+)"')
KINDS = {"BlockEntry", "ItemEntry", "DyedBlockList", "CopperBlockSet"}


def scan(path: Path, first_section: str) -> list[dict]:
    lines = path.read_text(encoding="utf-8").splitlines()
    section = first_section
    out = []
    for i, line in enumerate(lines):
        if "PALETTES_CREATIVE_TAB" in line:
            section = "Palettes"
        m = SECTION.match(line)
        if m and m.group(1) != "Load":
            section = m.group(1) if section != "Palettes" else section
        m = FIELD.match(line)
        if not m or m.group(1) not in KINDS:
            continue
        # The id is the first quoted name in the next few lines of the declaration.
        window = " ".join(lines[i:i + 6]).split(";")[0]
        q = QUOTED.search(window)
        ident = q.group(1) if q and m.group(1) in ("BlockEntry", "ItemEntry") else m.group(2).lower()
        out.append({"id": ident, "field": m.group(2), "section": section})
    return out


def snapshot(src: Path) -> None:
    pkg = src / "src" / "main" / "java" / "com" / "simibubi" / "create"
    feats = scan(pkg / "AllBlocks.java", "Schematics") + scan(pkg / "AllItems.java", "Materials")
    seen, unique = set(), []
    for f in feats:
        if f["id"] not in seen:
            seen.add(f["id"])
            unique.append(f)
    SNAPSHOT.parent.mkdir(exist_ok=True)
    SNAPSHOT.write_text(json.dumps({"source": "Create mc1.21.1/dev (AllBlocks.java, AllItems.java)",
                                    "features": unique}, indent=1) + "\n", encoding="utf-8")
    print(f"{len(unique)} features written to {SNAPSHOT.relative_to(ROOT)}")


def declared() -> set[str]:
    ids = set()
    for kind in ("block", "item"):
        d = DECLS / kind
        if d.is_dir():
            ids.update(p.stem for p in d.glob("*.json"))
    return ids


def tally() -> list[tuple[str, int, int]]:
    feats = json.loads(SNAPSHOT.read_text(encoding="utf-8"))["features"]
    done = declared()
    rows = []
    for sec in ORDER + sorted({f["section"] for f in feats} - set(ORDER)):
        group = [f for f in feats if f["section"] == sec]
        if group:
            rows.append((sec, sum(f["id"] in done for f in group), len(group)))
    return rows


def esc(s: str) -> str:
    return s.replace("&", "&amp;").replace("<", "&lt;")


def draw(rows: list[tuple[str, int, int]]) -> None:
    w, label_w, bar_w, row_h, top = 640, 120, 400, 26, 92
    total_done = sum(r[1] for r in rows)
    total = sum(r[2] for r in rows)
    pct = 100 * total_done / total
    h = top + row_h * len(rows) + 16
    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" '
        'font-family="-apple-system,Segoe UI,Helvetica,Arial,sans-serif">',
        "<style>.t{fill:#24292f}.m{fill:#57606a}.track{fill:#d0d7de}.fill{fill:#c98a2b}"
        "@media (prefers-color-scheme:dark){.t{fill:#e6edf3}.m{fill:#8d96a0}.track{fill:#30363d}"
        ".fill{fill:#e3a64a}}</style>",
        f'<text class="t" x="16" y="30" font-size="18" font-weight="600">Create ported to Horizon: '
        f'{pct:.1f}%</text>',
        f'<text class="m" x="16" y="50" font-size="12">{total_done} of {total} Create blocks and items '
        'declared</text>',
        f'<rect class="track" x="16" y="60" width="{w - 32}" height="12" rx="6"/>',
        f'<rect class="fill" x="16" y="60" width="{max(12, (w - 32) * total_done / total):.1f}" '
        'height="12" rx="6"/>',
    ]
    for i, (sec, d, n) in enumerate(rows):
        y = top + i * row_h
        fw = bar_w * d / n
        parts.append(f'<text class="t" x="16" y="{y + 13}" font-size="13">{esc(sec)}</text>')
        parts.append(f'<rect class="track" x="{label_w}" y="{y + 3}" width="{bar_w}" height="12" rx="3"/>')
        if d:
            parts.append(f'<rect class="fill" x="{label_w}" y="{y + 3}" width="{max(3, fw):.1f}" '
                         'height="12" rx="3"/>')
        parts.append(f'<text class="m" x="{label_w + bar_w + 10}" y="{y + 13}" font-size="12">'
                     f'{d} / {n}  ({100 * d / n:.0f}%)</text>')
    parts.append("</svg>")
    SVG.parent.mkdir(exist_ok=True)
    SVG.write_text("\n".join(parts) + "\n", encoding="utf-8")


def update_readme(rows: list[tuple[str, int, int]]) -> None:
    start, end = "<!-- progress:start -->", "<!-- progress:end -->"
    text = README.read_text(encoding="utf-8")
    if start not in text:
        return
    table = ["| Area | Done | Total | |", "|---|---:|---:|---:|"]
    table += [f"| {s} | {d} | {n} | {100 * d / n:.0f}% |" for s, d, n in rows]
    d, n = sum(r[1] for r in rows), sum(r[2] for r in rows)
    table.append(f"| **All** | **{d}** | **{n}** | **{100 * d / n:.1f}%** |")
    body = "\n".join(table)
    pre, rest = text.split(start, 1)
    post = rest.split(end, 1)[1]
    README.write_text(f"{pre}{start}\n{body}\n{end}{post}", encoding="utf-8")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--snapshot", type=Path, metavar="CREATE_SRC",
                    help="rebuild the feature list from a Create source checkout first")
    args = ap.parse_args()
    if args.snapshot:
        snapshot(args.snapshot)
    rows = tally()
    draw(rows)
    update_readme(rows)
    for s, d, n in rows:
        print(f"{s:14} {d:4} / {n}")


if __name__ == "__main__":
    main()
