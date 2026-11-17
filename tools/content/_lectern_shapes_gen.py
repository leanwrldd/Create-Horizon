"""One-off generator for the Lectern Controller's shapes (not a plugin: the leading underscore keeps
tools/gen_content.py from loading it). Writes tools/content/_lectern_shapes_gen.json.

    python tools/content/_lectern_shapes_gen.py [prepared game data dir]

`LecternControllerBlock` copies `Blocks.LECTERN` (`initialProperties(() -> Blocks.LECTERN)`) and extends
`LecternBlock`, so its shape is vanilla's `LecternBlock.getShape`. Horizon can't be asked for a vanilla
block's shape at generation time, so it is read from the prepared game data (`facts.json` names each
state and its properties, `gameplay-facts.json` the outline, same order) and baked, as the Copycats'
facts are. The outline and the collision are the same here (the lectern's real collision has a thin top
plate where the outline has the sloped boxes; the outline is a fair stand-in).
"""

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
DEFAULT_DATA = Path.home() / "AppData/Roaming/horizon/.horizon-cache/mod-content/5cce03fe38433e65"
OUT = Path(__file__).with_name("_lectern_shapes_gen.json")


def main(data):
    facts = json.loads((data / "facts.json").read_text(encoding="utf-8"))["states"]
    gameplay = json.loads((data / "gameplay-facts.json").read_text(encoding="utf-8"))["states"]
    assert len(facts) == len(gameplay), (len(facts), len(gameplay))
    shapes = {}
    for fact, state in zip(facts, gameplay):
        if fact[0] != "minecraft:lectern":
            continue
        facing = fact[4]["properties"]["facing"]
        boxes = [[round(v, 6) for v in box] for box in state["outline"]]
        assert shapes.setdefault(facing, boxes) == boxes, f"the lectern's {facing} shape depends on other properties"
    OUT.write_text(json.dumps(shapes, indent=1, sort_keys=True) + "\n", encoding="utf-8")
    print(f"wrote {OUT} ({', '.join(f'{k}: {len(v)} boxes' for k, v in sorted(shapes.items()))})")


if __name__ == "__main__":
    main(Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_DATA)
