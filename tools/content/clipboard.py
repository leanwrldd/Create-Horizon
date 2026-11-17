"""The Clipboard (module `clipboard`): `create:clipboard`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.CLIPBOARD` (`ClipboardBlock`, a `FaceAttachedHorizontalDirectionalBlock`
with `face`, `facing`, `written` and `waterlogged`; `SharedProperties.wooden`, `axeOrPickaxe`, `forceSolidOn`;
the item is a `ClipboardBlockItem`), `AllShapes.CLIPBOARD_FLOOR/CEILING/WALL` (`forHorizontal(SOUTH)`), the
blockstate (`block/clipboard/block_empty` and `block_written`), and the recipes `crafting/appliances/clipboard`
and `clipboard_clear`. Create's loot table drops nothing and the block hands the item over itself; here the
block drops itself and the module adds the pages to the drop. Behaviour:
`native/api/src/machines/clipboard.rs`.
"""
from gen_content import HORIZONTAL_FACING_STATE, WOODEN, block

# `MapColor` for `SharedProperties.wooden`'s default is the wood's; the clipboard sets none.
_PODZOL = 34

_STATES = {
    "face": {"type": "enum", "values": ["floor", "wall", "ceiling"]},
    **HORIZONTAL_FACING_STATE,
    "written": {"type": "boolean"},
}

_S = 1 / 16
# Each shape is for `facing south` in `AllShapes`; the others turn the box about the block's centre.
_FLOOR = {"south": [3 * _S, 0, 1 * _S, 13 * _S, 1 * _S, 15 * _S],
          "north": [3 * _S, 0, 1 * _S, 13 * _S, 1 * _S, 15 * _S],
          "east": [1 * _S, 0, 3 * _S, 15 * _S, 1 * _S, 13 * _S],
          "west": [1 * _S, 0, 3 * _S, 15 * _S, 1 * _S, 13 * _S]}
_CEILING = {f: [b[0], 15 * _S, b[2], b[3], 1, b[5]] for f, b in _FLOOR.items()}
# The wall shape `shape(3, 1, 0, 13, 15, 1)` for south hangs on the north side of the cell.
_WALL = {"south": [3 * _S, 1 * _S, 0, 13 * _S, 15 * _S, 1 * _S],
         "north": [3 * _S, 1 * _S, 15 * _S, 13 * _S, 15 * _S, 1],
         "east": [0, 1 * _S, 3 * _S, 1 * _S, 15 * _S, 13 * _S],
         "west": [15 * _S, 1 * _S, 3 * _S, 1, 15 * _S, 13 * _S]}

_SHAPES = {"properties": ["face", "facing"], "values": {}}
for _face, _boxes in (("floor", _FLOOR), ("ceiling", _CEILING), ("wall", _WALL)):
    for _facing, _box in _boxes.items():
        _SHAPES["values"][f"{_face},{_facing}"] = [_box]

BLOCKS = {
    "clipboard": block(_STATES, {"face": "wall", "facing": "north", "written": "false"}, WOODEN, _PODZOL,
                       _SHAPES, render_shape="MODEL"),
}
BLOCKS["clipboard"]["properties"]["occludes"] = False

NAMES = {"block.create.clipboard": "Clipboard"}

RECIPES = {
    # `crafting/appliances/clipboard`.
    "crafting/appliances/clipboard": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["A", "P", "G"],
        "key": {"A": "create:andesite_alloy", "G": "#minecraft:planks", "P": "minecraft:paper"},
        "result": {"id": "create:clipboard", "count": 1},
    },
    # `crafting/appliances/clipboard_clear`: a clipboard alone makes a clean one.
    "crafting/appliances/clipboard_clear": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:clipboard"],
        "result": {"id": "create:clipboard", "count": 1},
    },
}

TAB = ["clipboard"]
