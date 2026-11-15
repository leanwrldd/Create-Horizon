"""The Pulse Repeater, Pulse Extender, Pulse Timer, Powered Latch and Powered Toggle Latch:
`create:pulse_repeater`, `pulse_extender`, `pulse_timer`, `powered_latch`, `powered_toggle_latch`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.PULSE_REPEATER`, `PULSE_EXTENDER`, `PULSE_TIMER` (all
`BrassDiodeBlock`, `initialProperties(() -> Blocks.REPEATER)`, tag `SAFE_NBT`: `powered`,
`powering`, `facing` (horizontal), `inverted`), `POWERED_LATCH` (`PoweredLatchBlock`: `powered`,
`powering`, `powered_side`, `facing`), `POWERED_TOGGLE_LATCH` (`ToggleLatchBlock`: `powered`,
`powering`, `facing`), the vanilla `DiodeBlock` shape (`box(0, 0, 0, 16, 2, 16)`),
`data/create/recipe/crafting/logistics/`. The blocks are in the chunk mesh from Create's blockstate
files; the behaviour is `native/api/src/machines/diodes.rs`.
"""
from gen_content import block

# `Blocks.REPEATER`: `instabreak`, `SoundType.STONE` (named by a block), no map colour.
REPEATER = {"hardness": 0.0, "explosion_resistance": 0.0, "requires_tool": False,
            "sounds": "minecraft:stone"}

# `DiodeBlock.SHAPE`.
SHAPE = {"default": [[0, 0, 0, 1, 2 / 16, 1]]}

FACING = {"facing": {"type": "enum", "values": ["north", "south", "west", "east"]}}
BOOLEAN = {"type": "boolean"}


def diode(*extra):
    states = {**FACING, "powered": BOOLEAN, "powering": BOOLEAN, **{name: BOOLEAN for name in extra}}
    default = {"facing": "north", "powered": "false", "powering": "false",
               **{name: "false" for name in extra}}
    return block(states, default, REPEATER, 0, SHAPE, render_shape="MODEL")


BLOCKS = {
    "pulse_repeater": diode("inverted"),
    "pulse_extender": diode("inverted"),
    "pulse_timer": diode("inverted"),
    "powered_latch": diode("powered_side"),
    "powered_toggle_latch": diode(),
}

NAMES = {
    "block.create.pulse_repeater": "Pulse Repeater",
    "block.create.pulse_extender": "Pulse Extender",
    "block.create.pulse_timer": "Pulse Timer",
    "block.create.powered_latch": "Powered Latch",
    "block.create.powered_toggle_latch": "Powered Toggle Latch",
}

_KEY = {"C": "#c:plates/brass", "S": "#c:stones", "T": "minecraft:redstone_torch"}

RECIPES = {
    # `crafting/logistics/pulse_repeater`.
    "crafting/logistics/pulse_repeater": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["RCT", "SSS"],
        "key": {**_KEY, "R": "#c:dusts/redstone"},
        "result": {"id": "create:pulse_repeater", "count": 1},
    },
    # `crafting/logistics/pulse_extender`.
    "crafting/logistics/pulse_extender": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["  T", "RCT", "SSS"],
        "key": {**_KEY, "R": "#c:dusts/redstone"},
        "result": {"id": "create:pulse_extender", "count": 1},
    },
    # `crafting/logistics/pulse_timer`: an Amethyst Shard where the others have Redstone.
    "crafting/logistics/pulse_timer": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["RCT", "SSS"],
        "key": {**_KEY, "R": "minecraft:amethyst_shard"},
        "result": {"id": "create:pulse_timer", "count": 1},
    },
    # `crafting/logistics/powered_latch`.
    "crafting/logistics/powered_latch": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": [" T ", "RCR", "SSS"],
        "key": {"C": "minecraft:lever", "R": "#c:dusts/redstone", "S": "#c:stones",
                "T": "minecraft:redstone_torch"},
        "result": {"id": "create:powered_latch", "count": 1},
    },
    # `crafting/logistics/powered_toggle_latch`.
    "crafting/logistics/powered_toggle_latch": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": [" T ", " C ", "SSS"],
        "key": {"C": "minecraft:lever", "S": "#c:stones", "T": "minecraft:redstone_torch"},
        "result": {"id": "create:powered_toggle_latch", "count": 1},
    },
}

# NeoForge's redstone dust tag, which the recipes name.
TAGS = {
    "c:item/dusts/redstone": ["minecraft:redstone"],
    "create:block/safe_nbt": ["create:pulse_repeater", "create:pulse_extender", "create:pulse_timer"],
}

TAB = ["pulse_repeater", "pulse_extender", "pulse_timer", "powered_latch", "powered_toggle_latch"]
