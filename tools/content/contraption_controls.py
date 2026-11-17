"""The Contraption Controls: `create:contraption_controls` (module `contraption_controls`).

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.CONTRAPTION_CONTROLS` (`SharedProperties.stone`, `MapColor.PODZOL`,
`axeOrPickaxe`, a `horizontalBlock` of `block/contraption_controls/block`), `ControlsBlock` (`facing`, the way the
panel faces: the opposite of the player's, the same sneaking; also `open`, `virtual` and `waterlogged`, not here),
`AllShapes.CONTRAPTION_CONTROLS` and `CONTRAPTION_CONTROLS_COLLISION` (`forHorizontal(NORTH)`),
`AllPartialModels.CONTRAPTION_CONTROLS_BUTTON` and `_INDICATOR` (`indicator_0` to `indicator_7`),
`AllItemTags.CONTRAPTION_CONTROLLED`, `recipe/crafting/kinetics/contraption_controls`.

Only the block as it stands in the world is here (the click, the redstone, the filter, the button and
the dial); what it switches, the actors of a moving contraption, is not ported yet, so the tag below names
only what this port has. The behaviour is `native/api/src/machines/contraption_controls.rs`.
"""
from gen_content import ANDESITE, HORIZONTAL_FACING_STATE, block

PODZOL = 34  # MapColor.PODZOL


def _turn(box, steps):
    """A box (blocks) written for a block facing north, turned `steps` quarter turns clockwise seen
    from above (north to east): `forHorizontal(NORTH)`."""
    x0, y0, z0, x1, y1, z1 = box
    for _ in range(steps):
        x0, z0, x1, z1 = 1 - z1, x0, 1 - z0, x1
    return [x0, y0, z0, x1, y1, z1]


def _by_facing(boxes):
    texels = [[v / 16 for v in box] for box in boxes]
    return {"property": "facing", "values": {
        facing: [_turn(box, steps) for box in texels]
        for facing, steps in (("north", 0), ("east", 1), ("south", 2), ("west", 3))}}


# `AllShapes.CONTRAPTION_CONTROLS` and `CONTRAPTION_CONTROLS_COLLISION`, in texels.
SHAPE = _by_facing([[0, 0, 6, 2, 16, 16], [14, 0, 6, 16, 16, 16], [0, 0, 14, 16, 16, 16],
                    [0, 0, 6, 16, 12, 16], [0, 0, 4, 16, 2, 16]])
COLLISION = _by_facing([[0, 0, 6, 2, 16, 16], [14, 0, 6, 16, 16, 16], [0, 0, 14, 16, 16, 16],
                        [0, 0, 7, 16, 12, 16]])

CONTROLS = block(HORIZONTAL_FACING_STATE, {"facing": "north"}, ANDESITE, PODZOL, SHAPE, render_shape="MODEL")
CONTROLS["collision_shapes"] = COLLISION

BLOCKS = {"contraption_controls": CONTROLS}

# `ContraptionControlsRenderer`: the button, and the dial at each of its eight steps.
MODELS = ["block/contraption_controls/button"] + [
    f"block/contraption_controls/indicator_{i}" for i in range(8)]

NAMES = {
    "block.create.contraption_controls": "Contraption Controls",
    # `sendStatus` and the filter's label (`create.contraption.controls.*`, `create.contraptions.contoller.target`).
    "create.contraption.controls.actor_toggle.on": "On",
    "create.contraption.controls.actor_toggle.off": "Off",
    "create.contraption.controls.all_actor_toggle": "All Actors: %1$s",
    "create.contraption.controls.specific_actor_toggle": "%1$s Actors: %2$s",
    "create.contraptions.contoller.target": "Targeted Component",
}

RECIPES = {
    # `crafting/kinetics/contraption_controls`.
    "crafting/kinetics/contraption_controls": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["B", "C", "I"],
        "key": {"B": "#minecraft:buttons", "C": "create:andesite_casing", "I": "create:electron_tube"},
        "result": {"id": "create:contraption_controls", "count": 1},
    },
}

# What the block's filter takes (`CreateRegistrateTags`: the bell, campfires, dispenser and dropper; and
# every block registered with `.tag(CONTRAPTION_CONTROLLED)`: the drill, saw, deployer and the two
# funnels of those this port has; the sliding doors are tagged in `palettes.py`).
TAGS = {
    "create:item/contraption_controlled": [
        "minecraft:bell", "minecraft:campfire", "minecraft:soul_campfire", "minecraft:dispenser",
        "minecraft:dropper", "create:mechanical_drill", "create:mechanical_saw", "create:deployer",
        "create:andesite_funnel", "create:brass_funnel",
    ],
}

TAB = ["contraption_controls"]
