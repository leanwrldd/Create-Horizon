"""The Redstone Requester: `create:redstone_requester`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.REDSTONE_REQUESTER` (`SharedProperties.stone`, `SoundType.NETHERITE_BLOCK`,
`noOcclusion`, `pickaxeOnly`, a `RedstoneRequesterBlockItem`, blockstate `horizontalAxisBlock` with `forPowered`),
`RedstoneRequesterBlock` (`axis` of `HORIZONTAL_AXIS`, `powered`; no shape of its own, so the full cube),
`data/create/recipe/crafting/logistics/redstone_requester`. The `_clear` recipe isn't declared (no frequency in the item). The
behaviour is `mod/src/machines/redstone_requester.rs` and `logistics.rs`.
"""
from gen_content import block

# `SharedProperties.stone`: a stone block's hardness and blast resistance, a tool needed.
STONE = {"hardness": 1.5, "explosion_resistance": 6.0, "requires_tool": True}

BLOCKS = {
    "redstone_requester": block(
        {"axis": {"type": "enum", "values": ["x", "z"]}, "powered": {"type": "boolean"}},
        {"axis": "x", "powered": "false"}, STONE, 0, sounds="minecraft:netherite_block", render_shape="MODEL"),
}
PICKAXE_ONLY = {"redstone_requester"}

NAMES = {"block.create.redstone_requester": "Redstone Requester"}
TAB = ["redstone_requester"]

RECIPES = {
    "crafting/logistics/redstone_requester": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["C", "A", "B"],
        "key": {"A": "create:stock_link", "B": "#c:ingots/iron", "C": "#c:dusts/redstone"},
        "result": {"id": "create:redstone_requester", "count": 1},
    },
}
