"""The Wooden and Metal Brackets (module `bracket`).

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.WOODEN_BRACKET` (`SoundType.SCAFFOLDING`, `axeOrPickaxe`) and
`METAL_BRACKET` (`SoundType.NETHERITE_BLOCK`, `pickaxeOnly`), `BracketBlock` (`facing`,
`axis_along_first` (Java's `AXIS_ALONG_FIRST_COORDINATE`), `type`: pipe, cog or shaft), `BracketGenerator` (the blockstates), the
recipes under `recipe/crafting/kinetics/` and `AllItemTags.INVALID_FOR_TRACK_PAVING`.

A bracket block is never placed in the world: the item puts it on a shaft, cogwheel or pipe, which
keeps it in its block entity and draws the block's model (`BracketBlockItem`,
`BracketedBlockEntityBehaviour`). The block exists to be that model and the item.
"""

from gen_content import FACING_STATE, block

BRACKET_STATES = {
    **FACING_STATE,
    "type": {"type": "enum", "values": ["pipe", "cog", "shaft"]},
    "axis_along_first": {"type": "boolean"},
}
BRACKET_DEFAULT = {"facing": "north", "type": "pipe", "axis_along_first": "false"}
# `Block.Properties.of()` with no strength: breaks at once. Only the sound is set.
WOODEN_BRACKET = {"hardness": 0.0, "explosion_resistance": 0.0, "requires_tool": False,
                  "sounds": "minecraft:scaffolding"}
METAL_BRACKET = dict(WOODEN_BRACKET, sounds="minecraft:netherite_block")

BLOCKS = {
    "wooden_bracket": block(BRACKET_STATES, BRACKET_DEFAULT, WOODEN_BRACKET, 0, render_shape="MODEL"),
    "metal_bracket": block(BRACKET_STATES, BRACKET_DEFAULT, METAL_BRACKET, 0, render_shape="MODEL"),
}
# `pickaxeOnly()`; the wooden one is `axeOrPickaxe()`, the default.
PICKAXE_ONLY = {"metal_bracket"}

NAMES = {
    "block.create.wooden_bracket": "Wooden Bracket",
    "block.create.metal_bracket": "Metal Bracket",
}

RECIPES = {
    # `crafting/kinetics/wooden_bracket`.
    "wooden_bracket": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["SSS", "PCP"],
        "key": {"S": "#c:rods/wooden", "P": "#minecraft:planks", "C": "create:andesite_alloy"},
        "result": {"id": "create:wooden_bracket", "count": 4},
    },
    # `crafting/kinetics/metal_bracket`.
    "metal_bracket": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["SSS", "PCP"],
        "key": {"S": "#c:nuggets/iron", "P": "#c:ingots/iron", "C": "create:andesite_alloy"},
        "result": {"id": "create:metal_bracket", "count": 4},
    },
}

# `data/create/tags/item/invalid_for_track_paving`.
TAGS = {"create:item/invalid_for_track_paving": ["create:wooden_bracket", "create:metal_bracket"]}

TAB = ["wooden_bracket", "metal_bracket"]
