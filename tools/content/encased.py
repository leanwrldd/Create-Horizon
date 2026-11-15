"""Declarations for the encased shafts and encased cogwheels (module `encased`).

Values from Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.ANDESITE_ENCASED_SHAFT` and the five
like it, `BuilderTransformers.encasedBase` (stone properties, no occlusion, no stress impact, drops
the plain block), `EncasedCogwheelBlock` (`top_shaft`, `bottom_shaft`). The casing sits in the
chunk mesh (Create's blockstate files); the block entity draws the shaft and the cogwheel.
"""
from gen_content import ANDESITE, AXIS_STATE, PODZOL, block

# `MapColor.TERRACOTTA_BROWN`, the brass casing's colour (`AllBlocks.BRASS_ENCASED_SHAFT`).
TERRACOTTA_BROWN = 48

COG_STATES = {**AXIS_STATE, "top_shaft": {"type": "boolean"}, "bottom_shaft": {"type": "boolean"}}
COG_DEFAULT = {"axis": "y", "top_shaft": "false", "bottom_shaft": "false"}


def shaft(color):
    return block(AXIS_STATE, {"axis": "y"}, ANDESITE, color, render_shape="MODEL")


def cog(color):
    return block(COG_STATES, COG_DEFAULT, ANDESITE, color, render_shape="MODEL")


BLOCKS = {
    "andesite_encased_shaft": shaft(PODZOL),
    "brass_encased_shaft": shaft(TERRACOTTA_BROWN),
    "andesite_encased_cogwheel": cog(PODZOL),
    "brass_encased_cogwheel": cog(TERRACOTTA_BROWN),
    "andesite_encased_large_cogwheel": cog(PODZOL),
    "brass_encased_large_cogwheel": cog(TERRACOTTA_BROWN),
}

# What `EncasedCogRenderer` draws turning (`AllPartialModels.SHAFTLESS_COGWHEEL` and
# `SHAFTLESS_LARGE_COGWHEEL`); the shaft halves are the core's.
MODELS = ["block/cogwheel_shaftless", "block/large_cogwheel_shaftless"]

NAMES = {
    "block.create.andesite_encased_shaft": "Andesite Encased Shaft",
    "block.create.brass_encased_shaft": "Brass Encased Shaft",
    "block.create.andesite_encased_cogwheel": "Andesite Encased Cogwheel",
    "block.create.brass_encased_cogwheel": "Brass Encased Cogwheel",
    "block.create.andesite_encased_large_cogwheel": "Andesite Encased Large Cogwheel",
    "block.create.brass_encased_large_cogwheel": "Brass Encased Large Cogwheel",
}


def drops(item):
    # `encasedBase`: `p.dropOther(lb, drop.get())`, the plain block, not the casing.
    return [{
        "rolls": 1,
        "entries": [{"type": "minecraft:item", "name": item}],
        "condition": {"type": "minecraft:survives_explosion"},
    }]


LOOT = {
    "andesite_encased_shaft": drops("create:shaft"),
    "brass_encased_shaft": drops("create:shaft"),
    "andesite_encased_cogwheel": drops("create:cogwheel"),
    "brass_encased_cogwheel": drops("create:cogwheel"),
    "andesite_encased_large_cogwheel": drops("create:large_cogwheel"),
    "brass_encased_large_cogwheel": drops("create:large_cogwheel"),
}

TAB = list(BLOCKS)
