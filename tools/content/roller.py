"""The Mechanical Roller: `create:mechanical_roller` (module `roller`).

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.MECHANICAL_ROLLER` (`SharedProperties.stone`,
`MapColor.COLOR_GRAY`, `noOcclusion`, `axeOrPickaxe`, `horizontalBlockProvider(true)`, cutout layer,
`RollerBlockItem`, item tagged `create:contraption_controlled`, custom item model), `RollerBlock`
(an `AttachedActorBlock`: `facing`, `waterlogged` left out as elsewhere; `getShape` is `Shapes.block()`,
`canSurvive` is always true), `recipe/crafting/kinetics/mechanical_roller`,
`data/create/damage_type/mechanical_roller`, `CreateDamageSources.roller`.

The block does nothing on its own: it works on a moving structure (`machines/roller_actor.rs`,
`rules/roller.rs`). The placed block draws its wheel and frame (`RollerRenderer`,
`AllPartialModels.ROLLER_WHEEL` and `ROLLER_FRAME`); Create's blockstate, the casing model, the item's
`.obj` model, the wheel's `.obj` and the textures are imported from the player's jar.
"""

from gen_content import ANDESITE, GRAY, HORIZONTAL_FACING_STATE, block

BLOCKS = {
    # `RollerBlock.getShape` is `Shapes.block()`: no shape table. The casing is in the chunk mesh
    # (`create:block/mechanical_roller/block`).
    "mechanical_roller": block(HORIZONTAL_FACING_STATE, {"facing": "north"}, ANDESITE, GRAY,
                               render_shape="MODEL"),
}
BLOCKS["mechanical_roller"]["properties"]["occludes"] = False  # `noOcclusion`

# What the block entity draws besides the block's model (`AllPartialModels.ROLLER_WHEEL`, `ROLLER_FRAME`).
MODELS = ["block/mechanical_roller/wheel", "block/mechanical_roller/frame"]

TAB = ["mechanical_roller"]

NAMES = {
    "block.create.mechanical_roller": "Mechanical Roller",
    "death.attack.create.mechanical_roller": "%1$s was flattened by a Mechanical Roller",
    "death.attack.create.mechanical_roller.player": "%1$s was thrown in front of a Mechanical Roller by %2$s",
}

TAGS = {
    "create:item/contraption_controlled": ["create:mechanical_roller"],
}

RECIPES = {
    # `recipe/crafting/kinetics/mechanical_roller`, in Horizon's 26.3 form.
    "crafting/kinetics/mechanical_roller": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["A", "C", "I"],
        "key": {"A": "create:electron_tube", "C": "create:andesite_casing", "I": "create:crushing_wheel"},
        "result": {"id": "create:mechanical_roller", "count": 1},
    },
}

# `data/create/damage_type/mechanical_roller.json` (`CreateDamageSources.roller`).
FILES = {
    "data/create/damage_type/mechanical_roller.json": {
        "exhaustion": 0.0, "message_id": "create.mechanical_roller",
        "scaling": "when_caused_by_living_non_player"},
}
