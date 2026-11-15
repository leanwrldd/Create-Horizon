"""Create's fluids: `create:honey` and `create:chocolate` (flowing, with a bucket and a liquid
block each), and the names of `create:tea` and `create:potion` (tank-only: Horizon's tanks take
any id outside `minecraft`, and Create's two are virtual fluids that never flow).

Create: `AllFluids` (`HONEY`: viscosity 2000, density 1400, `levelDecreasePerBlock(2)`,
`tickRate(25)`, `slopeFindDistance(3)`, `explosionResistance(100)`; `CHOCOLATE`: viscosity
1500, the same otherwise), `AllBlocks`' liquid blocks `honey` and `chocolate`
(`MapColor.TERRACOTTA_YELLOW` and `TERRACOTTA_BROWN`), the buckets, `AllFluidTags`, the tags
`c:honey` and `c:chocolate` of the datagen, lang `fluid.create.*`.

Horizon's fluid takes still and flowing sprites; Create's flowing ones aren't reachable from the
jar import (it follows model textures), so both name the still sprite that `block/honey` and
`block/chocolate` (the liquid blocks' particle models) already bring in.
"""

# `YELLOW_TERRACOTTA` and `BROWN_TERRACOTTA` of materials.py, `MapColor.TERRACOTTA_*`.
FLUIDS = {
    "honey": {"viscosity": 2000},
    "chocolate": {"viscosity": 1500},
}


def _fluid(name, viscosity):
    return {
        "template": "minecraft:water",
        "viscosity": viscosity,
        "density": 1400,
        "level_decrease_per_block": 2,
        "tick_rate": 25,
        "slope_find_distance": 3,
        "explosion_resistance": 100.0,
        "bucket": f"create:{name}_bucket",
        "block": f"create:{name}",
        "still": f"create:fluid/{name}_still",
        "flowing": f"create:fluid/{name}_still",
    }


FILES = {f"data/create/horizon/fluid/{name}.json": _fluid(name, **values)
         for name, values in FLUIDS.items()}
# The liquid block copies the fluid's template and holds it at its `level`; it has no item.
BLOCKS = {name: {"template": "minecraft:water", "item": False} for name in FLUIDS}
LOOT = {name: None for name in FLUIDS}
MINEABLE = {name: [] for name in FLUIDS}
MODELS = [f"block/{name}" for name in FLUIDS]  # the particle model that names the still sprite
ITEMS = {f"{name}_bucket": {"template": "minecraft:water_bucket"} for name in FLUIDS}
TAB = [f"{name}_bucket" for name in FLUIDS]
NAMES = {
    "block.create.honey": "Honey",
    "block.create.chocolate": "Chocolate",
    "item.create.honey_bucket": "Honey Bucket",
    "item.create.chocolate_bucket": "Chocolate Bucket",
    "fluid.create.honey": "Honey",
    "fluid.create.chocolate": "Chocolate",
    "fluid.create.tea": "Builder's Tea",
    "fluid.create.potion": "Potion",
}
# `Tags.Fluids.HONEY` and `AllFluidTags.CHOCOLATE`: the source and the flowing fluid.
TAGS = {
    "c:fluid/honey": ["create:honey", "create:flowing_honey"],
    "c:fluid/chocolate": ["create:chocolate", "create:flowing_chocolate"],
    "c:item/buckets/honey": ["create:honey_bucket"],
}
