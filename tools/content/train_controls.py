"""The Train Controls and the carriage anchor: `create:controls`, `create:carriage_anchor` (module `train_controls`).

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.TRAIN_CONTROLS` (`ControlsBlock`; `SharedProperties.softMetal`,
`MapColor.TERRACOTTA_BROWN`, `SoundType.NETHERITE_BLOCK`, `cutoutMipped`, `pickaxeOnly`; `facing` (the opposite of the
player's, the same sneaking), `open` (set while the block is in a contraption world), `virtual`; `waterlogged` left
out as elsewhere), `AllShapes.CONTROLS` and `CONTROLS_COLLISION` (`forHorizontal(NORTH)`),
`AllPartialModels.TRAIN_CONTROLS_COVER` and `TRAIN_CONTROLS_LEVER`, `recipe/crafting/kinetics/controls` (Create's
datagen file). The behaviour is `machines/train_controls.rs` and `machines/trains.rs`.

`create:carriage_anchor` is ours: Horizon has no entity type a mod can draw, so each carriage of a train is drawn by
a block entity in an invisible block that follows it (`machines/train_draw.rs`). It has no shape, no item, and its
model is empty.
"""
from gen_content import ANDESITE, HORIZONTAL_FACING_STATE, block

TERRACOTTA_BROWN = 48  # `MapColor.TERRACOTTA_BROWN`
# `SharedProperties.softMetal`: a gold block (3.0 / 6.0), with `SoundType.NETHERITE_BLOCK`.
SOFT_METAL = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
              "sounds": "minecraft:netherite_block"}


def _turn(box, steps):
    """A box (blocks) written for a block facing north, turned `steps` quarter turns clockwise seen from above."""
    x0, y0, z0, x1, y1, z1 = box
    for _ in range(steps):
        x0, z0, x1, z1 = 1 - z1, x0, 1 - z0, x1
    return [x0, y0, z0, x1, y1, z1]


def _by_facing(boxes):
    texels = [[v / 16 for v in box] for box in boxes]
    return {"property": "facing", "values": {
        facing: [_turn(box, steps) for box in texels]
        for facing, steps in (("north", 0), ("east", 1), ("south", 2), ("west", 3))}}


# `AllShapes.CONTROLS` and `CONTROLS_COLLISION`, in texels.
SHAPE = _by_facing([[0, 0, 6, 16, 16, 16], [0, 0, 4, 16, 2, 16]])
COLLISION = _by_facing([[0, 0, 6, 16, 16, 16]])

_STATES = {**HORIZONTAL_FACING_STATE, "open": {"type": "boolean"}, "virtual": {"type": "boolean"}}
CONTROLS = block(_STATES, {"facing": "north", "open": "false", "virtual": "false"}, SOFT_METAL, TERRACOTTA_BROWN,
                 SHAPE, render_shape="MODEL")
CONTROLS["collision_shapes"] = COLLISION

# The anchor: nothing to see, hit or stand on; it only holds the carriage's block entity.
_ANCHOR = block(None, None, {"hardness": 0.0, "explosion_resistance": 0.0, "requires_tool": False,
                             "sounds": "minecraft:glass"}, 0, {"default": []}, render_shape="ENTITYBLOCK_ANIMATED")
_ANCHOR["collision_shapes"] = {"default": []}
_ANCHOR["item"] = False
_ANCHOR["properties"]["occludes"] = False

BLOCKS = {"controls": CONTROLS, "carriage_anchor": _ANCHOR}

BLOCKSTATES = {"carriage_anchor": {"variants": {"": {"model": "create:block/carriage_anchor"}}}}

FILES = {
    "assets/create/models/block/carriage_anchor.json": {
        "textures": {"particle": "minecraft:block/glass"},
        "elements": [],
    },
}

# `ControlsRenderer`: the cover and the lever.
MODELS = ["block/controls/train/cover", "block/controls/train/lever"]

TAB = ["controls"]
PICKAXE_ONLY = {"controls"}

NAMES = {
    "block.create.controls": "Train Controls",
    "block.create.carriage_anchor": "Carriage Anchor",
    "create.contraption.controls.approach_station": "Hold %1$s to approach %2$s",
    "create.contraption.controls.start_controlling": "Now controlling: %1$s",
    "create.contraption.controls.stop_controlling": "Stopped controlling contraption",
    "create.train.arrived_at": "Arrived at %1$s",
    "create.train.departing_from": "Departing from %1$s",
}

RECIPES = {
    # `crafting/kinetics/controls`.
    "crafting/kinetics/controls": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["B", "C", "I"],
        "key": {"B": "minecraft:lever", "C": "create:railway_casing", "I": "create:precision_mechanism"},
        "result": {"id": "create:controls", "count": 1},
    },
}
