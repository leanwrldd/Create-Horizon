"""The Sequenced Gearshift: `create:sequenced_gearshift` (module `sequenced_gearshift`).

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.SEQUENCED_GEARSHIFT` (`SharedProperties.stone`,
`MapColor.TERRACOTTA_BROWN`, `axeOrPickaxe`, `noOcclusion`, `CStress.setNoImpact`), `SequencedGearshiftBlock`
(`HorizontalAxisKineticBlock`'s `axis` (x or z), `vertical`, and `state`, an integer 0 to 5: which instruction
runs, read by comparators), `SequencedGearshiftGenerator` (the model is `idle` or `seq_<state>`, turned
90 about x when vertical and about y when the axis is x),
`recipe/crafting/kinetics/sequenced_gearshift`. The casing is in the chunk mesh; the block entity
draws the shafts out of both ends. The behaviour is `native/api/src/machines/sequenced_gearshift.rs`.
"""
from gen_content import ANDESITE, block

TERRACOTTA_BROWN = 48  # MapColor.TERRACOTTA_BROWN

# The state's names are Create's: `HORIZONTAL_AXIS` is the property `axis` (x or z), `STATE` an
# integer property spelt as the digits' strings.
STATES = {
    "axis": {"type": "enum", "values": ["x", "z"]},
    "vertical": {"type": "boolean"},
    "state": {"type": "enum", "values": ["0", "1", "2", "3", "4", "5"]},
}

BLOCKS = {
    "sequenced_gearshift": block(STATES, {"axis": "z", "vertical": "false", "state": "0"}, ANDESITE,
                                 TERRACOTTA_BROWN, render_shape="MODEL"),
}

# The block's own models, as `SequencedGearshiftGenerator` picks them, are in the blockstate.
NAMES = {
    "block.create.sequenced_gearshift": "Sequenced Gearshift",
}

RECIPES = {
    # `crafting/kinetics/sequenced_gearshift`.
    "crafting/kinetics/sequenced_gearshift": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:brass_casing", "create:cogwheel", "create:electron_tube"],
        "result": {"id": "create:sequenced_gearshift", "count": 1},
    },
}

TAB = ["sequenced_gearshift"]
