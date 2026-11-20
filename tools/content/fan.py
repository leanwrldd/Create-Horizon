"""Declarations for the Encased Fan and the Nozzle (module `fan`): the blocks, their names, loot,
crafting, the tags the fan reads, Create's splashing and haunting recipes, vanilla's smelting,
smoking and blasting recipes copied for smoking and blasting, and the damage types of lava-heated
and smoky air.

Values from Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.ENCASED_FAN` and `NOZZLE`,
`data/create/recipe/crafting/kinetics/{encased_fan,nozzle}.json`, `data/create/tags/` and
`data/create/damage_type/` (Create's datagen and data files).
"""
import importlib.util
from pathlib import Path

from gen_content import ANDESITE, FACING_STATE, PODZOL, block

# `MapColor.COLOR_LIGHT_GRAY`.
LIGHT_GRAY = 22


def _turn(box, facing):
    """`box` written for a block facing up, turned as Create's blockstate turns the model for
    `facing` (`blockstates/nozzle.json`: down is x 180, north x 90, east x 90 y 90, south x 90
    y 180, west x 90 y 270) and as Catnip's `VoxelShaper.forDirectional` turns the shape."""
    x_turn, y_turn = {"up": (0, 0), "down": (180, 0), "north": (90, 0), "east": (90, 90),
                      "south": (90, 180), "west": (90, 270)}[facing]

    def point(p):
        x, y, z = (v - 0.5 for v in p)
        for _ in range(x_turn // 90):
            y, z = z, -y          # x 90: up becomes north
        for _ in range(y_turn // 90):
            x, z = -z, x          # y 90: north becomes east (clockwise from above)
        return [x + 0.5, y + 0.5, z + 0.5]

    a, b = point(box[:3]), point(box[3:])
    return [round(min(a[i], b[i]), 6) for i in range(3)] + [round(max(a[i], b[i]), 6) for i in range(3)]


# `AllShapes.NOZZLE`: shape(2, 0, 2, 14, 14, 14).add(1, 13, 1, 15, 15, 15).erase(3, 13, 3, 13, 15, 13),
# in texels, facing up. The erased hole leaves the body 13 high with a frame round its top.
_NOZZLE = [[2, 0, 2, 14, 13, 14],
           [1, 13, 1, 15, 15, 3], [1, 13, 13, 15, 15, 15],
           [1, 13, 3, 3, 15, 13], [13, 13, 3, 15, 15, 13]]
_NOZZLE = [[v / 16 for v in box] for box in _NOZZLE]
NOZZLE_SHAPES = {"property": "facing", "values": {
    f: [_turn(box, f) for box in _NOZZLE]
    for f in ("north", "south", "east", "west", "up", "down")}}

BLOCKS = {
    # `AllBlocks.ENCASED_FAN`: stone, PODZOL; no shape of its own, so a full block. The casing
    # sits in the chunk mesh; the block entity draws the shaft and the propeller.
    "encased_fan": block(FACING_STATE, {"facing": "north"}, ANDESITE, PODZOL, render_shape="MODEL"),
    # `AllBlocks.NOZZLE`: stone, COLOR_LIGHT_GRAY, `AllShapes.NOZZLE`. All model.
    "nozzle": block(FACING_STATE, {"facing": "north"}, ANDESITE, LIGHT_GRAY, NOZZLE_SHAPES,
                    render_shape="MODEL"),
}

# What the fan's block entity draws besides the block's model (`AllPartialModels.ENCASED_FAN_INNER`).
MODELS = ["block/encased_fan/propeller"]

TAB = ["encased_fan", "nozzle"]

NAMES = {
    "block.create.encased_fan": "Encased Fan",
    "block.create.nozzle": "Nozzle",
    "death.attack.create.fan_fire": "%1$s got smoked by an Encased Fan",
    "death.attack.create.fan_fire.player": "%1$s was thrown into a smoker by %2$s",
    "death.attack.create.fan_lava": "%1$s was incinerated by an Encased Fan",
    "death.attack.create.fan_lava.player": "%1$s was thrown into a smelter by %2$s",
}

# `CreateStandardRecipeGen` (`crafting/kinetics/encased_fan`, `nozzle`) in Horizon's 26.3 form.
# The Andesite Casing and the Propeller are declared by other modules.
RECIPES = {
    "crafting/kinetics/encased_fan": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["S", "A", "P"],
        "key": {"S": "create:shaft", "A": "create:andesite_casing", "P": "create:propeller"},
        "result": {"id": "create:encased_fan", "count": 1},
    },
    "crafting/kinetics/nozzle": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": [" S ", " C ", "SSS"],
        "key": {"S": "create:andesite_alloy", "C": "#minecraft:wool"},
        "result": {"id": "create:nozzle", "count": 1},
    },
}


def _fan_recipes():
    spec = importlib.util.spec_from_file_location(
        "create_content_fan_recipes_table", Path(__file__).with_name("fan_recipes.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.FAN_RECIPES


# Every `create:splashing` and `create:haunting` recipe of Create's datagen that doesn't need
# another mod, exactly as written (`tools/content/fan_recipes_gen.py` made the table).
# The three `splashing/industrial_iron_*` recipes are declared, identically, by the palettes
# module (it owns the weathered iron blocks), so the table's copies are left out here.
RECIPES.update({key: value for key, value in _fan_recipes().items()
                if not key.startswith("splashing/industrial_iron")})

def _cooking():
    spec = importlib.util.spec_from_file_location(
        "create_content_fan_cooking_table", Path(__file__).with_name("_fan_cooking.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


_COOKING = _cooking()

# `RuntimeDataGenerator.washingRecipes`: Create writes a splashing recipe from each concrete
# powder to its concrete at load time, so its datagen output has none. Vanilla's 16 colours.
_COLOURS = ("white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray",
            "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black")
RECIPES.update({
    f"splashing/runtime_generated/compat/minecraft/{c}_concrete_powder_to_{c}_concrete": {
        "type": "create:splashing",
        "ingredients": [{"item": f"minecraft:{c}_concrete_powder"}],
        "results": [{"id": f"minecraft:{c}_concrete"}],
    }
    for c in _COLOURS
})

TAGS = {
    # `data/create/tags/block/fan_transparent.json`: what the air goes through. Ids nobody
    # declares yet are dropped by the game when it reads the tag.
    "create:block/fan_transparent": [
        "create:blaze_burner", "create:lit_blaze_burner", "create:sail_frame",
        "create:andesite_bars", "create:brass_bars", "create:copper_bars", "create:copycat_base",
        "minecraft:iron_bars", "minecraft:mangrove_roots", "minecraft:copper_grate",
        "minecraft:exposed_copper_grate", "minecraft:weathered_copper_grate",
        "minecraft:oxidized_copper_grate", "minecraft:waxed_copper_grate",
        "minecraft:waxed_exposed_copper_grate", "minecraft:waxed_weathered_copper_grate",
        "minecraft:waxed_oxidized_copper_grate",
        "#minecraft:campfires", "#minecraft:fences", "#minecraft:leaves",
    ],
    # `data/create/tags/block/fan_processing_catalysts/`. Splashing has no block of its own:
    # water is a fluid (the glue knows water and lava as Create's fluid tags list them).
    "create:block/fan_processing_catalysts/splashing": [],
    "create:block/fan_processing_catalysts/smoking": [
        "create:blaze_burner", "create:lit_blaze_burner", "minecraft:fire", "minecraft:campfire",
    ],
    "create:block/fan_processing_catalysts/blasting": ["create:blaze_burner"],
    "create:block/fan_processing_catalysts/haunting": [
        "create:lit_blaze_burner", "minecraft:soul_fire", "minecraft:soul_campfire",
    ],
    # What `BlastingType.canProcess` spares: Create 1.21.1's `DataComponents.FIRE_RESISTANT` became
    # `minecraft:damage_resistant` against `#minecraft:is_fire`, baked from the game's items
    # (`_fan_cooking_gen.py`).
    "create:item/fan_fire_resistant": _COOKING.FIRE_RESISTANT,
    # `AllBlocks.NOZZLE`: `.tag(AllBlockTags.BRITTLE.tag)`.
    "create:block/brittle": ["create:nozzle"],
    # The convention tags the recipes above name (NeoForge's `c:` tags, with vanilla's members).
    "c:item/gems/lapis": ["minecraft:lapis_lazuli"],
    "c:item/cobblestones": ["minecraft:cobblestone", "minecraft:mossy_cobblestone",
                            "minecraft:cobbled_deepslate"],
    "c:item/flours/wheat": ["create:wheat_flour"],
    "c:item/glass_blocks": ["minecraft:glass", "minecraft:tinted_glass"] + [
        f"minecraft:{colour}_stained_glass" for colour in (
            "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray",
            "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black")],
    "c:item/glass_panes": ["minecraft:glass_pane"] + [
        f"minecraft:{colour}_stained_glass_pane" for colour in (
            "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray",
            "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black")],
}

# `data/create/damage_type/fan_lava.json`, `fan_fire.json` (`CreateDamageSources`).
FILES = {
    "data/create/damage_type/fan_lava.json": {
        "effects": "burning", "exhaustion": 0.0, "message_id": "create.fan_lava",
        "scaling": "when_caused_by_living_non_player"},
    "data/create/damage_type/fan_fire.json": {
        "effects": "burning", "exhaustion": 0.0, "message_id": "create.fan_fire",
        "scaling": "when_caused_by_living_non_player"},
}
