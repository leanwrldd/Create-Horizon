"""The Train Station: `create:track_station` (module `track_station`).

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.TRACK_STATION` (`StationBlock`; `SharedProperties.softMetal`,
`MapColor.PODZOL`, `SoundType.NETHERITE_BLOCK`, `pickaxeOnly`, a `TrackTargetingBlockItem` of
`EdgePointType.STATION` with a custom item model), `StationBlock` (`assembling`; `waterlogged` left out as
elsewhere; `getShape` is `AllShapes.STATION`), `AllShapes.STATION = shape(0, 0, 0, 16, 2, 16).add(1, 0, 1, 15, 13,
15)`, `recipe/crafting/kinetics/track_station` (Create's datagen file).

The block's model is Create's blockstate from the player's jar (`create:block/track_station/block`); the flag the
block entity raises (`StationRenderer`) is not drawn yet (see `machines/track_station.rs`). The display sources
(`STATION_SUMMARY`, `TRAIN_STATUS`) are not declared.
"""

from gen_content import PODZOL, block

# `SharedProperties.softMetal`: a gold block (3.0 / 6.0), with `SoundType.NETHERITE_BLOCK`.
SOFT_METAL = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
              "sounds": "minecraft:netherite_block"}

# `AllShapes.STATION`.
_STATION = [[0, 0, 0, 1, 2 / 16, 1], [1 / 16, 0, 1 / 16, 15 / 16, 13 / 16, 15 / 16]]

BLOCKS = {
    "track_station": block({"assembling": {"type": "boolean"}}, {"assembling": "false"}, SOFT_METAL, PODZOL,
                           _STATION, render_shape="MODEL"),
}

TAB = ["track_station"]

NAMES = {
    "block.create.track_station": "Train Station",
    "create.track_target.clear": "Cleared track selection",
    "create.track_target.invalid": "Cannot target this track here",
    "create.track_target.missing": "Right-click the targeted train track first",
    "create.track_target.no_junctions": "Targeted track cannot be an intersection",
    "create.track_target.occupied": "Targeted track is occupied",
    "create.track_target.set": "Targeted track selected",
    "create.track_target.success": "Successfully bound to targeted track",
    "create.track_target.too_far": "Targeted track is too far from here",
    "create.train_assembly.bogey_created": "Bogey created. Click again to cycle type",
    "create.train_assembly.bogeys_too_close": "Bogeys %1$s and %2$s are too close to each other",
    "create.train_assembly.frontmost_bogey_at_station": "Frontmost Bogey must be at Station Marker",
    "create.train_assembly.no_bogeys": "No Bogeys Found",
    "create.train_assembly.no_controls":
        "Attach at least one forward-facing Train Controls block (Are you missing Super Glue?)",
    "create.train_assembly.not_connected_in_order": "Bogeys are not connected in order",
    "create.train_assembly.nothing_attached": "No structure attached to Bogey %1$s",
    "create.train_assembly.requires_casing": "Use Train Casing to create bogeys on tracks",
    "create.train_assembly.single_bogey_carriage": "This Bogey type cannot support a carriage on its own",
    "create.train_assembly.too_many_bogeys": "Too many Bogeys attached: %1$s",
    "create.train.unnamed": "Unnamed Train",
}

RECIPES = {
    # `data/create/recipe/crafting/kinetics/track_station.json`.
    "crafting/kinetics/track_station": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:railway_casing", "minecraft:compass"],
        "result": {"id": "create:track_station", "count": 2},
    },
}

PICKAXE_ONLY = {"track_station"}
