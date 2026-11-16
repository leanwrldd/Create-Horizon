"""The Smart Observer: `create:content_observer`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.SMART_OBSERVER` (`SharedProperties.stone`: andesite's
1.5 and 6.0; `MapColor.TERRACOTTA_BROWN`, `noOcclusion`, not a redstone conductor, `axeOrPickaxe`),
`SmartObserverBlock` (`DirectedDirectionalBlock`: `target` and `facing`; `POWERED`),
`SmartObserverGenerator` (the blockstate file in Create's jar picks `block/content_observer/block`
or `block_powered`, turned by the target and the facing), and
`data/create/recipe/crafting/logistics/content_observer.json`. The block sits in the chunk mesh
from Create's blockstate file. The behaviour is `native/api/src/machines/content_observer.rs`
(known differences at its top).
"""
from gen_content import ANDESITE, block

# `MapColor.TERRACOTTA_BROWN`.
TERRACOTTA_BROWN = 48

# `DirectedDirectionalBlock.TARGET` (`AttachFace`, in its order: floor, wall, ceiling) and
# `HorizontalDirectionalBlock.FACING`; the constructors' default states.
STATES = {
    "facing": {"type": "enum", "values": ["north", "south", "west", "east"]},
    "powered": {"type": "boolean"},
    "target": {"type": "enum", "values": ["wall", "floor", "ceiling"]},
}
DEFAULT = {"facing": "north", "powered": "false", "target": "wall"}

BLOCKS = {
    "content_observer": block(STATES, DEFAULT, ANDESITE, TERRACOTTA_BROWN, render_shape="MODEL"),
}

NAMES = {"block.create.content_observer": "Smart Observer"}
TAB = ["content_observer"]

RECIPES = {
    # `recipe/crafting/logistics/content_observer`: an Electron Tube over a Brass Casing over an
    # Observer.
    "crafting/logistics/content_observer": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["R", "B", "I"],
        "key": {"B": "create:brass_casing", "I": "minecraft:observer", "R": "create:electron_tube"},
        "result": {"id": "create:content_observer"},
    },
}
