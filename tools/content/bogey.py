"""The bogeys and the Train Casing: `create:small_bogey`, `create:large_bogey`, `create:railway_casing`
(module `bogey`).

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.SMALL_BOGEY` and `LARGE_BOGEY` (`StandardBogeyBlock`;
`BuilderTransformers.bogey()`: `SharedProperties.softMetal`, `SoundType.NETHERITE_BLOCK`, `noOcclusion`,
`pickaxeOnly`, the model `block/track/bogey/top` turned by `axis`, a loot table that drops a Train Casing, no item
of their own), `MapColor.PODZOL`; `AbstractBogeyBlock` (`axis`: `HORIZONTAL_AXIS`, `waterlogged` left out as
elsewhere); `AllBlocks.RAILWAY_CASING` (`BuilderTransformers.layeredCasing`: `SharedProperties.stone`,
`axeOrPickaxe`, tags `AllBlockTags.CASING` and `AllItemTags.CASING`, `MapColor.TERRACOTTA_CYAN`,
`SoundType.NETHERITE_BLOCK`), `recipe/item_application/railway_casing` (Create's datagen file).

A bogey is put on the track by right-clicking the track in a station's assembly area with a Train Casing
(`StationBlockEntity.trackClicked`, `machines/track_station.rs`); the standing bogey is the anchor block of
`block/track/bogey/top`, the wheels `BogeyBlockEntityRenderer` draws are not drawn yet. The bogeys' block entity
(`StandardBogeyBlockEntity`: the style and its data) isn't declared: only the standard style exists.
"""

from gen_content import ANDESITE, PODZOL, block

TERRACOTTA_CYAN = 45  # `MapColor.TERRACOTTA_CYAN`
# `SharedProperties.softMetal`: a gold block (3.0 / 6.0), with `SoundType.NETHERITE_BLOCK`.
SOFT_METAL = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
              "sounds": "minecraft:netherite_block"}

_STATES = {"axis": {"type": "enum", "values": ["x", "z"]}}


def _bogey():
    value = block(_STATES, {"axis": "z"}, SOFT_METAL, PODZOL, render_shape="MODEL")
    value["item"] = False  # `BuilderTransformers.bogey()` registers no item
    value["properties"]["occludes"] = False  # `noOcclusion`
    return value


def _casing():
    return block(None, None, dict(ANDESITE, sounds="minecraft:netherite_block"), TERRACOTTA_CYAN,
                 render_shape="MODEL", template="minecraft:andesite")


BLOCKS = {
    "small_bogey": _bogey(),
    "large_bogey": _bogey(),
    "railway_casing": _casing(),
}

# `.loot((p, l) -> p.dropOther(l, AllBlocks.RAILWAY_CASING.get()))`.
_DROP_CASING = [{
    "rolls": 1,
    "entries": [{"type": "minecraft:item", "name": "create:railway_casing"}],
    "condition": {"type": "minecraft:survives_explosion"},
}]
LOOT = {"small_bogey": _DROP_CASING, "large_bogey": _DROP_CASING}

TAB = ["railway_casing"]

# `AllPartialModels` of the standard bogeys' frame, wheels and the large bogey's drive, which `train_draw.rs` asks the game to
# load as standalone models (`carriage_pose.rs` has the ids). Create's `.obj` files come with their `.mtl`: `build_content.py`
# copies both. Without this entry the game logs "standalone model ... not loaded" and the wheels aren't drawn.
MODELS = [f"block/track/bogey/{name}" for name in (
    "bogey_frame", "bogey_wheel", "bogey_drive", "bogey_drive_wheel", "bogey_drive_wheel_pin", "bogey_drive_piston",
    "bogey_drive_belt")]

NAMES = {
    "block.create.small_bogey": "Small Bogey",
    "block.create.large_bogey": "Large Bogey",
    "block.create.railway_casing": "Train Casing",
    "create.bogey.style.invalid": "Unnamed style",
    "create.bogey.style.no_other_sizes": "No other sizes",
    "create.bogey.style.standard": "Standard",
    "create.bogey.style.updated_style": "Updated style",
    "create.bogey.style.updated_style_and_size": "Updated style and size",
}

TAGS = {
    # `BuilderTransformers.layeredCasing`: `AllBlockTags.CASING` and `AllItemTags.CASING`.
    "create:block/casing": ["create:railway_casing"],
    "create:item/casing": ["create:railway_casing"],
}

# The standing bogeys are `pickaxeOnly()`; the casing is `axeOrPickaxe()` (both tools, the default).
PICKAXE_ONLY = {"small_bogey", "large_bogey"}

RECIPES = {
    # `data/create/recipe/item_application/railway_casing.json`.
    "item_application/railway_casing": {
        "type": "create:item_application",
        "ingredients": [{"item": "create:brass_casing"}, {"tag": "c:plates/obsidian"}],
        "results": [{"id": "create:railway_casing"}],
    },
}
