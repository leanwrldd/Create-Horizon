"""The Mechanical Mixer: `create:mechanical_mixer`.

Create: `AllBlocks.MECHANICAL_MIXER` (`SharedProperties.stone`, `noOcclusion`, `MapColor.STONE`,
`axeOrPickaxe`, `CStress.setImpact(4.0)`), `MechanicalMixerBlock.getShape`,
`recipe/crafting/kinetics/mechanical_mixer`.
"""

from gen_content import ANDESITE, block

STONE = 11  # `MapColor.STONE`

# `AllShapes.CASING_14PX.get(Direction.DOWN)`, what players meet (`MechanicalMixerBlock.getShape`);
# other entities get the hollow `MECHANICAL_PROCESSOR_SHAPE`, not modelled here. The casing is in
# the chunk mesh (`create:block/mechanical_mixer/block`); the block entity draws the cog, the pole
# and the head.
BLOCKS = {
    "mechanical_mixer": block({}, {}, ANDESITE, STONE, {"default": [[0, 2 / 16, 0, 1, 1, 1]]},
                             render_shape="MODEL"),
}
NAMES = {"block.create.mechanical_mixer": "Mechanical Mixer"}
TAB = ["mechanical_mixer"]
# `AllPartialModels.MECHANICAL_MIXER_POLE`, `MECHANICAL_MIXER_HEAD`, `SHAFTLESS_COGWHEEL`.
MODELS = ["block/mechanical_mixer/pole", "block/mechanical_mixer/head", "block/cogwheel_shaftless"]

RECIPES = {
    # `recipe/crafting/kinetics/mechanical_mixer`.
    "mechanical_mixer": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["S", "C", "I"],
        "key": {"S": "create:cogwheel", "C": "create:andesite_casing", "I": "create:whisk"},
        "result": {"id": "create:mechanical_mixer"},
    },
}
