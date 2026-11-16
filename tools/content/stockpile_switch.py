"""The Threshold Switch: `create:stockpile_switch`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.THRESHOLD_SWITCH` (`SharedProperties.stone`: andesite's
1.5 and 6.0; `MapColor.TERRACOTTA_BROWN`, `noOcclusion`, not a redstone conductor, `axeOrPickaxe`),
`ThresholdSwitchBlock` (`DirectedDirectionalBlock`: `target` and `facing`; `LEVEL`, 0 to 5),
`ThresholdSwitchGenerator` (the blockstate file in Create's jar picks
`block/threshold_switch/block_<target>_<level>`, turned by the facing), its item model
(`threshold_switch/block_wall`) and `data/create/recipe/crafting/logistics/stockpile_switch.json`.
The behaviour is `native/api/src/machines/stockpile_switch.rs` (known differences at its top).
"""
from gen_content import ANDESITE, block

# `MapColor.TERRACOTTA_BROWN`.
TERRACOTTA_BROWN = 48

# `DirectedDirectionalBlock.TARGET` and `FACING`, and `ThresholdSwitchBlock.LEVEL`, an integer
# property 0 to 5 (declared as an enum of those numbers); the constructor's default state.
STATES = {
    "facing": {"type": "enum", "values": ["north", "south", "west", "east"]},
    "level": {"type": "enum", "values": ["0", "1", "2", "3", "4", "5"]},
    "target": {"type": "enum", "values": ["wall", "floor", "ceiling"]},
}
DEFAULT = {"facing": "north", "level": "0", "target": "wall"}

BLOCKS = {
    "stockpile_switch": block(STATES, DEFAULT, ANDESITE, TERRACOTTA_BROWN, render_shape="MODEL"),
}

NAMES = {"block.create.stockpile_switch": "Threshold Switch"}
TAB = ["stockpile_switch"]

RECIPES = {
    # `recipe/crafting/logistics/stockpile_switch`: an Electron Tube over a Brass Casing over a
    # Comparator.
    "crafting/logistics/stockpile_switch": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["R", "B", "I"],
        "key": {"B": "create:brass_casing", "I": "minecraft:comparator", "R": "create:electron_tube"},
        "result": {"id": "create:stockpile_switch"},
    },
}
