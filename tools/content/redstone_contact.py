"""The Redstone Contact: `create:redstone_contact` (module `redstone_contact`).

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.REDSTONE_CONTACT` (`SharedProperties.stone`, `MapColor.COLOR_GRAY`,
`axeOrPickaxe`, `directionalBlock(forPowered)`, item tagged `create:contraption_controlled`),
`RedstoneContactBlock` (`facing`, `powered`), `ContactMovementBehaviour`,
`recipe/crafting/logistics/redstone_contact` (makes two). The behaviour is
`native/api/src/machines/redstone_contact.rs` (and `actor_more.rs` on a structure).
"""

from gen_content import ANDESITE, FACING_STATE, GRAY, block

BLOCKS = {
    "redstone_contact": block({**FACING_STATE, "powered": {"type": "boolean"}},
                              {"facing": "up", "powered": "false"}, ANDESITE, GRAY, render_shape="MODEL"),
}

TAB = ["redstone_contact"]

NAMES = {"block.create.redstone_contact": "Redstone Contact"}

TAGS = {"create:item/contraption_controlled": ["create:redstone_contact"]}

RECIPES = {
    # `recipe/crafting/logistics/redstone_contact`, in Horizon's 26.3 form.
    "crafting/logistics/redstone_contact": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": [" S ", "CWC", "CCC"],
        "key": {"C": "minecraft:cobblestone", "S": "#c:plates/iron", "W": "#c:dusts/redstone"},
        "result": {"id": "create:redstone_contact", "count": 2},
    },
}
