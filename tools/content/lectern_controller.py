"""The Lectern Controller: `create:lectern_controller`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.LECTERN_CONTROLLER` (`LecternControllerBlock`,
`initialProperties(() -> Blocks.LECTERN)`, `axeOnly`, `horizontalBlock` blockstate on vanilla's
`block/lectern`, `lt.dropOther(block, Blocks.LECTERN)`). It has no item: a Linked Controller used on a lectern
with no book replaces it, and it drops the lectern (and the controller, from its block entity). The shape is
vanilla's lectern's, baked from the prepared game data by `_lectern_shapes_gen.py`. The behaviour is
`native/api/src/machines/lectern_controller.rs`; the block entity draws the controller on the lectern.

The block keeps the lectern's three properties (`facing`, `has_book`, `powered`) as Create's blockstate file has them.
"""
import json
from pathlib import Path

from gen_content import block, WOOD_SOUND

WOOD = 13  # `MapColor.WOOD`
# `Blocks.LECTERN` (Horizon's prepared data): `strength(2.5f)`, wooden sound.
LECTERN = {"hardness": 2.5, "explosion_resistance": 2.5, "requires_tool": False, "sounds": WOOD_SOUND}

_SHAPES = json.loads((Path(__file__).with_name("_lectern_shapes_gen.json")).read_text(encoding="utf-8"))

_lectern_controller = block(
    {"facing": {"type": "enum", "values": ["north", "south", "west", "east"]},
     "has_book": {"type": "boolean"},
     "powered": {"type": "boolean"}},
    {"facing": "north", "has_book": "true", "powered": "false"},
    LECTERN, WOOD,
    {"property": "facing", "values": {f: _SHAPES[f] for f in ("north", "south", "west", "east")}},
    render_shape="MODEL")
_lectern_controller["item"] = False  # a controller placed on a lectern makes it

BLOCKS = {"lectern_controller": _lectern_controller}

# `axeOnly()`: mined with an axe only.
MINEABLE = {"lectern_controller": ["axe"]}

# `lt.dropOther(block, Blocks.LECTERN)`.
LOOT = {
    "lectern_controller": [
        {"rolls": 1, "entries": [{"type": "minecraft:item", "name": "minecraft:lectern"}],
         "condition": {"type": "minecraft:survives_explosion"}},
    ],
}

NAMES = {"block.create.lectern_controller": "Lectern Controller",
         "create.subtitle.controller_put": "Controller thumps",
         "create.subtitle.controller_take": "Lectern empties"}
