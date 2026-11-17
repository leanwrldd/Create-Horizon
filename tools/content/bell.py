"""The Peculiar Bell and the Haunted Bell (module `bell`): `create:peculiar_bell`, `create:haunted_bell`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.PECULIAR_BELL` and `HAUNTED_BELL` (`BuilderTransformers.bell`:
`SharedProperties.softMetal` (a gold block), `SoundType.ANVIL`, `noOcclusion`, `pickaxeOnly`, tag
`create:brittle`; `MapColor.GOLD` and `MapColor.SAND`; `PeculiarBellBlock`/`HauntedBellBlock` are vanilla's
`BellBlock` through `AbstractBellBlock`: `facing`, `attachment`, `powered`), their blockstates
(`block/peculiar_bell_floor|ceiling|single_wall|double_wall` over `block/bell_base/`), and the recipe
`crafting/curiosities/peculiar_bell`. The haunted bell has no recipe: a peculiar bell over soul fire or a Fan's
haunting (in `fan_recipes.py`) makes one. Declared on vanilla's `minecraft:bell` template, so the ring,
attachment and redstone behaviour are the template's. Behaviour: `native/api/src/machines/bell.rs`.
"""

GOLD = 30  # MapColor.GOLD
SAND = 2  # MapColor.SAND
# `SharedProperties.softMetal` is the gold block; `BuilderTransformers.bell` gives it the anvil's sound.
SOFT_METAL = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
              "sounds": "minecraft:anvil"}


def _bell(colour):
    return {
        "template": "minecraft:bell", "item": True,
        "properties": {**SOFT_METAL, "map_color": colour, "render_shape": "MODEL", "occludes": False},
    }


BLOCKS = {"peculiar_bell": _bell(GOLD), "haunted_bell": _bell(SAND)}
PICKAXE_ONLY = {"peculiar_bell", "haunted_bell"}

NAMES = {
    "block.create.peculiar_bell": "Peculiar Bell",
    "block.create.haunted_bell": "Haunted Bell",
}

RECIPES = {
    # `crafting/curiosities/peculiar_bell`.
    "crafting/curiosities/peculiar_bell": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["I", "P"],
        "key": {"I": "#c:storage_blocks/brass", "P": "#c:plates/brass"},
        "result": {"id": "create:peculiar_bell", "count": 1},
    },
}

# `AllCreativeModeTabs`: both bells (the haunted one is listed there too).
TAB = ["peculiar_bell", "haunted_bell"]
