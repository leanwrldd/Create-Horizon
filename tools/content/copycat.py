"""The Copycats (module `copycat`): `create:copycat_step`, `create:copycat_panel`, and the two blocks
that only exist to be a model, `create:copycat_base` and `create:copycat_bars`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.COPYCAT_BASE`, `COPYCAT_STEP`, `COPYCAT_PANEL`,
`COPYCAT_BARS` (`SharedProperties.softMetal` = gold block, `BuilderTransformers.copycat`: no
occlusion, `MapColor.NONE`, axe or pickaxe; the base `MapColor.GLOW_LICHEN`, pickaxe only, tag
`create:fan_transparent`), `CopycatStepBlock` (`half`, `facing`), `CopycatPanelBlock` (`facing`),
`AllShapes.STEP_BOTTOM`, `STEP_TOP`, `CASING_3PX`, and the datagen recipes
`copycat_step_from_ingots_zinc_stonecutting` and `copycat_panel_from_ingots_zinc_stonecutting`.
Only the step and the panel have items (the base and the bars are models: `block.item()` is absent
for them in `AllBlocks`). Create's blockstates for the step and panel are empty models: the block
entity draws the material (`machines/copycat.rs`). The base model, the item models and the bars model
come from the player's jar.

Known differences from Create: waterlogging and the placement assists (`PoleHelper`, the panel's
`PlacementHelper`) are left out; see `rules/copycat.rs` for how the material is drawn.
"""

from gen_content import FACING_STATE, HORIZONTAL_FACING_STATE, block, facing_shapes

SOFT_METAL = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
              "sounds": "minecraft:gold_block"}  # SharedProperties.softMetal
GLOW_LICHEN, NONE = 61, 0  # MapColor ids

HORIZONTAL = ["north", "south", "west", "east"]


def step_box(facing, top):
    """`rules::copycat::step_box`: `shape(0, 0, 8, 16, 8, 16).forHorizontal(SOUTH)`."""
    y0, y1 = (0.5, 1.0) if top else (0.0, 0.5)
    return {"south": [0, y0, 0.5, 1, y1, 1], "north": [0, y0, 0, 1, y1, 0.5],
            "east": [0.5, y0, 0, 1, y1, 1], "west": [0, y0, 0, 0.5, y1, 1]}[facing]


_step_shapes = {"properties": ["half", "facing"], "values": {
    f"{half},{facing}": [step_box(facing, half == "top")]
    for half in ("bottom", "top") for facing in HORIZONTAL}}
# `CASING_3PX.forDirectional()`: a 3/16 slab behind the facing.
_panel_shapes = facing_shapes([[0, 0, 0, 1, 3 / 16, 1]])


def _copycat(states, default, shapes):
    value = block(states, default, SOFT_METAL, NONE, shapes)  # drawn by its block entity
    value["properties"]["occludes"] = False
    return value


_base = block(None, None, SOFT_METAL, GLOW_LICHEN, render_shape="MODEL")
_base["item"] = False
_bars = block(FACING_STATE, {"facing": "north"},
              {"hardness": 0.0, "explosion_resistance": 0.0, "requires_tool": False,
               "sounds": "minecraft:iron_block"}, NONE, render_shape="MODEL")
_bars["item"] = False
_bars["properties"]["occludes"] = False

BLOCKS = {
    "copycat_base": _base,
    "copycat_step": _copycat(
        {"half": {"type": "enum", "values": ["bottom", "top"]}, **HORIZONTAL_FACING_STATE},
        {"half": "bottom", "facing": "south"}, _step_shapes),
    "copycat_panel": _copycat(FACING_STATE, {"facing": "up"}, _panel_shapes),
    "copycat_bars": _bars,
}
PICKAXE_ONLY = {"copycat_base"}
MINEABLE = {"copycat_bars": []}  # in no tool tag

NAMES = {
    "block.create.copycat_base": "Copycat Base",
    "block.create.copycat_step": "Copycat Step",
    "block.create.copycat_panel": "Copycat Panel",
    "block.create.copycat_bars": "Copycat Bars",
}

TAGS = {"create:block/fan_transparent": ["create:copycat_base"]}

RECIPES = {
    f"{name}_from_ingots_zinc_stonecutting": {
        "type": "minecraft:stonecutting", "ingredient": "#c:ingots/zinc",
        "result": {"id": f"create:{name}", "count": 4},
    }
    for name in ("copycat_step", "copycat_panel")
}

PALETTES_TAB = ["copycat_step", "copycat_panel"]
