"""The Cuckoo Clock and the Mysterious Cuckoo Clock (module `cuckoo_clock`).

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.CUCKOO_CLOCK` and `MYSTERIOUS_CUCKOO_CLOCK`
(`SharedProperties.wooden`, `MapColor.TERRACOTTA_YELLOW`, `axeOrPickaxe`,
`BuilderTransformers.cuckooClock`: horizontal block with `CStress.setImpact(1)`), `AllShapes.CUCKOO_CLOCK`
(shape(1, 0, 1, 15, 19, 15)), `AllPartialModels.CUCKOO_*`, the recipes under
`recipe/crafting/kinetics/`. The mysterious clock is left out of the creative tab
(`AllCreativeModeTabs.makeExclusionPredicate`) and is named "Cuckoo Clock" too.
"""

from gen_content import HORIZONTAL_FACING_STATE, WOODEN, block

TERRACOTTA_YELLOW = 40  # MapColor.TERRACOTTA_YELLOW

# `shape(1, 0, 1, 15, 19, 15)` is 19 texels high; a declared shape stops at the block's top.
SHAPE = {"property": "facing", "values": {
    f: [[1 / 16, 0, 1 / 16, 15 / 16, 1, 15 / 16]] for f in ("north", "south", "east", "west")}}


def _clock():
    # The clock's body is in the chunk mesh (`block/cuckoo_clock/block`); the block entity draws the
    # shaft, the hands, the doors and the bird.
    return block(HORIZONTAL_FACING_STATE, {"facing": "north"}, WOODEN, TERRACOTTA_YELLOW, SHAPE,
                 render_shape="MODEL")


BLOCKS = {"cuckoo_clock": _clock(), "mysterious_cuckoo_clock": _clock()}

NAMES = {
    "block.create.cuckoo_clock": "Cuckoo Clock",
    "block.create.mysterious_cuckoo_clock": "Cuckoo Clock",
}

# The partial models the block entity draws (`AllPartialModels.CUCKOO_HOUR_HAND` ...).
MODELS = [f"create:block/cuckoo_clock/{name}"
          for name in ("hour_hand", "minute_hand", "left_door", "right_door", "pig", "creeper")]

RECIPES = {
    # `crafting/kinetics/cuckoo_clock`.
    "cuckoo_clock": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["S", "C", "A"],
        "key": {"S": "#minecraft:planks", "C": "create:andesite_casing", "A": "minecraft:clock"},
        "result": {"id": "create:cuckoo_clock"},
    },
    # `crafting/kinetics/mysterious_cuckoo_clock`.
    "mysterious_cuckoo_clock": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": [" C ", "CBC", " C "],
        "key": {"C": "#c:gunpowders", "B": "create:cuckoo_clock"},
        "result": {"id": "create:mysterious_cuckoo_clock"},
    },
}

# NeoForge's `c:gunpowders` (`Tags.Items.GUNPOWDERS`).
TAGS = {"c:item/gunpowders": ["minecraft:gunpowder"]}

TAB = ["cuckoo_clock"]
