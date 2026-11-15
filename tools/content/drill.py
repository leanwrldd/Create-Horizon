"""The Mechanical Drill: `create:mechanical_drill`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.MECHANICAL_DRILL` (`SharedProperties.stone`,
`MapColor.PODZOL`, axe or pickaxe, `CStress.setImpact(4.0)`), `DrillBlock` (the `facing` state;
`waterlogged` is left out, as elsewhere), `AllShapes.CASING_12PX`, `recipe/crafting/kinetics/
mechanical_drill`, `data/create/damage_type/mechanical_drill`, `CreateDamageSources.drill`.
"""

from gen_content import ANDESITE, FACING_STATE, PODZOL, block, facing_shapes

BLOCKS = {
    # `AllShapes.CASING_12PX = shape(0, 0, 0, 16, 12, 16).forDirectional()`: the casing is 12 high
    # at the back and the head sticks out of the open front. The casing is in the chunk mesh
    # (`create:block/mechanical_drill/block`); the block entity draws the turning head.
    "mechanical_drill": block(FACING_STATE, {"facing": "north"}, ANDESITE, PODZOL,
                              facing_shapes([[0, 0, 0, 1, 12 / 16, 1]]), render_shape="MODEL"),
}
# `axeOrPickaxe()`: the default tags (both tools).

# What the block entity draws besides the block's model (`AllPartialModels.DRILL_HEAD`).
MODELS = ["block/mechanical_drill/head"]

TAB = ["mechanical_drill"]

NAMES = {
    "block.create.mechanical_drill": "Mechanical Drill",
    "death.attack.create.mechanical_drill": "%1$s was impaled by a Mechanical Drill",
    "death.attack.create.mechanical_drill.player": "%1$s was thrown in front of a Drill by %2$s",
}

RECIPES = {
    # `recipe/crafting/kinetics/mechanical_drill`, in Horizon's 26.3 form.
    "crafting/kinetics/mechanical_drill": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": [" A ", "AIA", " C "],
        "key": {"A": "create:andesite_alloy", "C": "create:andesite_casing",
                "I": "#c:ingots/iron"},
        "result": {"id": "create:mechanical_drill", "count": 1},
    },
}

# `data/create/damage_type/mechanical_drill.json` (`CreateDamageSources.drill`).
FILES = {
    "data/create/damage_type/mechanical_drill.json": {
        "exhaustion": 0.0, "message_id": "create.mechanical_drill",
        "scaling": "when_caused_by_living_non_player"},
}
