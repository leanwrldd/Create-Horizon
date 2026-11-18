"""Make mod/Cargo.lock agree with the game's own lock.

`horizon-launcher native-lock mod` resolves the NEWEST registry versions, and the launcher then refuses the build
("game or pinned package proc-macro2 moved or changed its dependency graph") when one of them replaces a version the
game is built with (here: unicode-ident, syn, crc32fast, flate2). This rewrites each package of the registry lock that
the game also has to the game's version, checksum and dependencies, copied from the checkout's Cargo.lock.

    python tools/native_lock.py   (runs native-lock, then this)
    python tools/pin_lock.py [path to the Horizon checkout's Cargo.lock]
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def blocks(text):
    """[(name, version, block text)] of a lock file's [[package]] tables."""
    out = []
    for block in re.split(r"(?m)^\[\[package\]\]\n", text)[1:]:
        name = re.search(r'name = "([^"]+)"', block).group(1)
        version = re.search(r'version = "([^"]+)"', block).group(1)
        out.append((name, version, block))
    return out


def main():
    if len(sys.argv) > 1:
        game_path = Path(sys.argv[1])
    else:
        game_path = None
        for parent in ROOT.parents:
            if (parent / "horizon-latest" / "Cargo.lock").is_file():
                game_path = parent / "horizon-latest" / "Cargo.lock"
                break
    game = {}
    for name, version, block in blocks(game_path.read_text(encoding="utf-8")):
        game.setdefault(name, []).append((version, block))
    lock_path = ROOT / "mod" / "Cargo.lock"
    text = lock_path.read_text(encoding="utf-8")
    head = text[:text.index("[[package]]")]
    present = {name for name, _, _ in blocks(text)}
    out = []
    changed = []
    for name, version, block in blocks(text):
        if name in game:
            major = version.split(".")[0]
            same = [(v, b) for v, b in game[name] if v.split(".")[0] == major]
            if same:
                new_version, new_block = same[0]
                if new_version != version:
                    changed.append(f"{name} {version} -> {new_version}")
                block = new_block
                # The game's package may depend on crates this mod's graph doesn't have (optional features): keep
                # only the dependencies that are in the lock, by bare name (the lock has one version of each).
                def keep(match):
                    deps = [d for d in re.findall(r'"([^"]+)"', match.group(1)) if d.split()[0] in present]
                    lines = "".join(f' "{d.split()[0]}",\n' for d in deps)
                    return "dependencies = [\n" + lines + "]" if deps else ""
                block = re.sub(r"dependencies = \[\n(.*?)\]\n?", lambda m: (keep(m) + "\n") if keep(m) else "", block, flags=re.S)
        out.append("[[package]]\n" + block.rstrip("\n") + "\n")
    lock_path.write_text(head + "\n".join(out), encoding="utf-8", newline="\n")
    print("pinned:", ", ".join(changed) or "nothing to change")


if __name__ == "__main__":
    main()
