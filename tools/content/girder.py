"""The Metal Girder and the Metal Girder Encased Shaft (module `girder`): `create:metal_girder` and
`create:metal_girder_encased_shaft`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.METAL_GIRDER` and `METAL_GIRDER_ENCASED_SHAFT`
(`SharedProperties.softMetal` = gold block's 3.0 / 6.0 and tool, `MapColor.COLOR_GRAY`,
`SoundType.NETHERITE_BLOCK`, pickaxe only), `GirderBlock` (`x`, `z`, `top`, `bottom`, `axis`),
`GirderEncasedShaftBlock` (`HORIZONTAL_AXIS`, which is the property `axis` with x and z, `top`,
`bottom`), `AllShapes.GIRDER_*` and `EIGHT_VOXEL_POLE`/`TEN_VOXEL_POLE`/`SIX_VOXEL_POLE`,
the datagen recipe `crafting/kinetics/metal_girder` and the loot tables. Create's blockstates
(multipart on those properties) and models are imported from the player's jar.

The encased shaft has no item in Create (`METAL_GIRDER_ENCASED_SHAFT` has no `.item()`): it is made
by clicking a girder with a Shaft or a shaft with a girder, and drops a girder and a shaft.

Known differences from Create: waterlogging is left out; see `rules/girder.rs` and
`machines/girder.rs` for the rest.
"""

from gen_content import AXES, GRAY, block

SOFT_METAL = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
              "sounds": "minecraft:netherite_block"}  # SharedProperties.softMetal, NETHERITE_BLOCK


def box(x0, y0, z0, x1, y1, z1):
    return [x0 / 16, y0 / 16, z0 / 16, x1 / 16, y1 / 16, z1 / 16]


# `AllShapes`: EIGHT_VOXEL_POLE (4..12, along y), TEN_VOXEL_POLE (3..13), SIX_VOXEL_POLE (5..11),
# GIRDER_BEAM = shape(4, 2, 0, 12, 14, 16).forHorizontalAxis() (a beam along z, turned for x).
POLE = box(4, 0, 4, 12, 16, 12)
TEN_POLE = box(3, 0, 3, 13, 16, 13)
BEAM = {"z": box(4, 2, 0, 12, 14, 16), "x": box(0, 2, 4, 16, 14, 12)}
# `getShape`: GIRDER_CROSS (the ten pole and both beams) when x and z, the beam along the one that
# is on, else the pole.
_girder_shapes = {"properties": ["x", "z"], "values": {
    "false,false": [POLE],
    "true,false": [BEAM["x"]],
    "false,true": [BEAM["z"]],
    "true,true": [TEN_POLE, BEAM["x"], BEAM["z"]],
}}
# `AllShapes.GIRDER_BEAM_SHAFT` = the beam along x plus a six pole along z, `forHorizontalAxis()`:
# for the shaft along z the beam runs along x, for the shaft along x the beam runs along z.
_encased_shapes = {"property": "axis", "values": {
    "z": [BEAM["x"], box(5, 5, 0, 11, 11, 16)],
    "x": [BEAM["z"], box(0, 5, 5, 16, 11, 11)],
}}

_girder = block(
    {"x": {"type": "boolean"}, "z": {"type": "boolean"}, "top": {"type": "boolean"},
     "bottom": {"type": "boolean"}, "axis": {"type": "enum", "values": AXES}},
    {"x": "false", "z": "false", "top": "false", "bottom": "false", "axis": "y"},
    SOFT_METAL, GRAY, _girder_shapes, render_shape="MODEL")
_girder["properties"]["occludes"] = False

_encased = block(
    {"axis": {"type": "enum", "values": ["x", "z"]}, "top": {"type": "boolean"},
     "bottom": {"type": "boolean"}},
    {"axis": "x", "top": "false", "bottom": "false"},
    SOFT_METAL, GRAY, _encased_shapes, render_shape="MODEL")
_encased["properties"]["occludes"] = False
_encased["item"] = False  # no `.item()` in AllBlocks

BLOCKS = {"metal_girder": _girder, "metal_girder_encased_shaft": _encased}
PICKAXE_ONLY = {"metal_girder", "metal_girder_encased_shaft"}

NAMES = {
    "block.create.metal_girder": "Metal Girder",
    "block.create.metal_girder_encased_shaft": "Metal Girder Encased Shaft",
}


def _pool(item):
    # `createSingleItemTable` / `applyExplosionCondition`: one roll, survives_explosion.
    return {"rolls": 1, "entries": [{"type": "minecraft:item", "name": item}],
            "condition": {"type": "minecraft:survives_explosion"}}


# `data/create/loot_table/blocks/metal_girder_encased_shaft.json`: a girder and a shaft.
LOOT = {"metal_girder_encased_shaft": [_pool("create:metal_girder"), _pool("create:shaft")]}

RECIPES = {
    # `CreateStandardRecipeGen.METAL_GIRDER`: `crafting/kinetics/metal_girder`.
    "crafting/kinetics/metal_girder": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "key": {"C": "create:andesite_alloy", "P": "#c:plates/iron"},
        "pattern": ["PPP", "CCC"],
        "result": {"id": "create:metal_girder", "count": 8},
    },
}

# After the palettes' scaffolding in Create's tab; here in file-name order (see the README).
PALETTES_TAB = ["metal_girder"]
