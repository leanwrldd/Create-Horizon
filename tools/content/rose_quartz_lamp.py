"""The Rose Quartz Lamp: `create:rose_quartz_lamp`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.ROSE_QUARTZ_LAMP`
(`initialProperties(() -> Blocks.REDSTONE_LAMP)`: `strength(0.3F)`, `SoundType.GLASS`;
`MapColor.TERRACOTTA_PINK`; `lightLevel(s -> POWERING ? 15 : 0)`; `pickaxeOnly`), `RoseQuartzLampBlock`
(`powered`, `powering`, `activate`), the blockstate file in Create's jar
(`block/rose_quartz_lamp[_powered]` by `powering`),
`data/create/recipe/crafting/kinetics/rose_quartz_lamp.json`. The behaviour is
`native/api/src/machines/rose_quartz_lamp.rs`.
"""

# `MapColor.TERRACOTTA_PINK`.
TERRACOTTA_PINK = 42

_bool = {"type": "boolean"}

BLOCKS = {
    "rose_quartz_lamp": {
        # A full block that occludes, as the redstone lamp does; its numbers are written out.
        "template": "minecraft:stone",
        "item": True,
        "properties": {"hardness": 0.3, "explosion_resistance": 0.3, "requires_tool": False,
                       "map_color": TERRACOTTA_PINK, "sounds": "minecraft:glass"},
        "state_properties": {"activate": _bool, "powered": _bool, "powering": _bool},
        "default_state": {"activate": "false", "powered": "false", "powering": "false"},
        # `lightLevel(s -> s.getValue(POWERING) ? 15 : 0)`.
        "state_light": {"property": "powering", "values": {"false": 0, "true": 15}},
    },
}

PICKAXE_ONLY = {"rose_quartz_lamp"}

NAMES = {"block.create.rose_quartz_lamp": "Rose Quartz Lamp"}

RECIPES = {
    # `crafting/kinetics/rose_quartz_lamp`.
    "crafting/kinetics/rose_quartz_lamp": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:polished_rose_quartz", "#c:dusts/redstone", "#c:ingots/zinc"],
        "result": {"id": "create:rose_quartz_lamp", "count": 1},
    },
}

# NeoForge's redstone dust tag, which the recipe names.
TAGS = {"c:item/dusts/redstone": ["minecraft:redstone"]}

TAB = ["rose_quartz_lamp"]
