"""Find the OLD definitions of the types named on the command line in the old Horizon source
(C:\...\horizon-src, a reference checkout) and print where each lives. With --write, append them to
mod/src/compat/old_types.rs verbatim (derives kept, doc comments dropped).

    python tools/extract_old_types.py [--write] Name Name ...
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = None
for parent in ROOT.parents:
    if (parent / "horizon-src").is_dir():
        SRC = parent / "horizon-src"
        break
args = [a for a in sys.argv[1:] if a != "--write"]
write = "--write" in sys.argv
files = [p for p in (SRC / "crates").rglob("*.rs")]
texts = {}


def find(name):
    pat = re.compile(r"(?m)^((?:[ ]*#\[[^\n]*\]\n)*)[ ]*pub (struct|enum|type|trait) %s\b[^\n]*" % re.escape(name))
    hits = []
    for p in files:
        t = texts.setdefault(p, p.read_text(encoding="utf-8", errors="ignore"))
        for m in pat.finditer(t):
            start = m.start()
            # take the whole item
            i = m.end()
            line = t[m.start(2) - 4:m.end()]
            if line.rstrip().endswith(";") and "{" not in line:
                end = i
            else:
                depth = 0
                j = t.index("{", m.end() - len(m.group(0)) + len(m.group(1))) if "{" in t[m.end() - 1:m.end() + 200] else i
                end = j
                while end < len(t):
                    if t[end] == "{":
                        depth += 1
                    elif t[end] == "}":
                        depth -= 1
                        if depth == 0:
                            end += 1
                            break
                    end += 1
            hits.append((p, t[start:end]))
    return hits


out = []
for name in args:
    hits = find(name)
    if not hits:
        print(f"{name}: NOT FOUND")
        continue
    p, text = hits[0]
    print(f"{name}: {p.relative_to(SRC)} ({len(hits)} hit(s), {text.count(chr(10))} lines)")
    text = re.sub(r"(?m)^\s*///[^\n]*\n", "", text)
    out.append(text)
if write:
    target = ROOT / "mod" / "src" / "compat" / "old_types.rs"
    target.write_text(("// Copied from the old Horizon source by tools/extract_old_types.py.\nuse super::*;\n\n" if not target.exists() else target.read_text()) + "\n".join(out) + "\n", encoding="utf-8")
