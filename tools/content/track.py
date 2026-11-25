"""The Train Track: `create:track` (module `track`).

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.TRACK` (`TrackMaterial.ANDESITE::createBlock`, so a `TrackBlock`;
`SharedProperties.stone` then `mapColor(METAL)`, `strength(0.8F)`, `SoundType.METAL`, `noOcclusion`,
`forceSolidOn`, `pickaxeOnly`, tags `create:tracks`, `create:girdable_tracks` and `c:relocation_not_supported`,
`TrackBlockItem`, item tag `create:tracks`, a generated item model), `TrackBlock` (`shape`: `TrackShape`, `turn`
(the `HAS_BE` property: the block has a block entity for its curves), waterlogging left out as elsewhere),
`recipe/sequenced_assembly/track` (Create's datagen file), `data/create/tags/*/tracks`, `girdable_tracks`, `sleepers`.

The straight track is the chunk model (Create's blockstate, imported from the player's jar: `TrackBlockStateGenerator`);
the curves are drawn by the block entity (`TrackRenderer`, `machines/track_draw.rs`).

`create:fake_track` (`FakeTrackBlock`, the marker `TrackBlockEntity.manageFakeTracksAlong` puts above a curve so a map
colours its column) is not declared: nothing places it yet.
"""

from gen_content import ANDESITE, METAL, block

# `TrackShape`'s constants, in declaration order: the values of the `shape` property.
SHAPES = ["none", "zo", "xo", "pd", "nd", "an", "as", "ae", "aw", "tn", "ts", "te", "tw",
          "cr_o", "cr_d", "cr_pdx", "cr_pdz", "cr_ndx", "cr_ndz"]
STATES = {
    "shape": {"type": "enum", "values": SHAPES},
    # `TrackBlock.HAS_BE`, named `turn`.
    "turn": {"type": "boolean"},
}
DEFAULT = {"shape": "zo", "turn": "false"}

# `SharedProperties.stone` with `.strength(0.8F)` (hardness and explosion resistance both 0.8), `SoundType.METAL`.
STONE_STRENGTH = dict(ANDESITE, hardness=0.8, explosion_resistance=0.8, sounds="minecraft:iron_block")


def _ascending(direction):
    """`TrackVoxelShapes.ascending()` (written for south, rising toward +z) clipped to the block, for the
    direction it rises toward: the low step and the six that climb two pixels at a time, as boxes in blocks."""
    steps = [(0, 0)] + [((i + 1) * 2, (i + 1) * 2) for i in range(6)]
    boxes = []
    for u, y in steps:
        u0, u1, y0, y1 = u / 16, (u + 4) / 16, y / 16, (y + 4) / 16
        u1, y1 = min(u1, 1.0), min(y1, 1.0)
        if direction == "south":
            boxes.append([0, y0, u0, 1, y1, u1])
        elif direction == "north":
            boxes.append([0, y0, 1 - u1, 1, y1, 1 - u0])
        elif direction == "east":
            boxes.append([u0, y0, 0, u1, y1, 1])
        else:
            boxes.append([1 - u1, y0, 0, 1 - u0, y1, 1])
    return boxes


# `AllShapes.TRACK_FALLBACK` and every other shape, clipped to the block, are 4 pixels high over the whole block (the
# orthogonal, diagonal, long and crossing shapes all cover the block's own cell; only the ascending ones are stepped).
_FLAT = [[0, 0, 0, 1, 4 / 16, 1]]
OUTLINE = {"property": "shape", "values": {
    **{name: _FLAT for name in SHAPES},
    "ae": _ascending("east"), "aw": _ascending("west"), "an": _ascending("north"), "as": _ascending("south"),
}}
# `getCollisionShape`: nothing for the ascending shapes, `AllShapes.TRACK_COLLISION` (2 pixels) otherwise.
COLLISION = {"property": "shape", "values": {
    **{name: [[0, 0, 0, 1, 2 / 16, 1]] for name in SHAPES},
    "ae": [], "aw": [], "an": [], "as": [],
}}

BLOCKS = {
    "track": block(STATES, DEFAULT, STONE_STRENGTH, METAL, OUTLINE, render_shape="MODEL"),
}
BLOCKS["track"]["collision_shapes"] = COLLISION
BLOCKS["track"]["properties"]["occludes"] = False  # `noOcclusion`

# What the block entity draws besides the block's model (`AllPartialModels.TRACK_TIE`, `TRACK_SEGMENT_LEFT`,
# `TRACK_SEGMENT_RIGHT`, the `TrackModelHolder.DEFAULT`).
MODELS = [
    "block/track/tie", "block/track/segment_left", "block/track/segment_right",
    # The markers an edge point shows on its track (`AllPartialModels.TRACK_*_OVERLAY`; `machines/train_overlay.rs`). The assembling
    # one is only named so its texture is imported: the spots are drawn as boxes in the colour of their state.
    "block/track_overlay/station", "block/track_overlay/signal", "block/track_overlay/signal_dual", "block/track_overlay/observer",
    "block/track_overlay/assembling",
]


TAB = ["track"]

# `pickaxeOnly()`.
PICKAXE_ONLY = {"track"}

NAMES = {
    "block.create.track": "Train Track",
    # `create.track.*` and `create.portal_track.*` of Create's `en_us.json`: what the item tells the player.
    "create.track.ascending_s_curve": "Cannot create sloped S-Bends",
    "create.track.hold_for_smooth_curve": "Hold %1$s for maximized turn",
    "create.track.junction_start": "Cannot start connection from a Junction",
    "create.track.leave_slope_ascending": "Cannot leave this slope while ascending",
    "create.track.leave_slope_descending": "Cannot leave this slope while descending",
    "create.track.not_enough_pavement": "Not holding enough pavement blocks",
    "create.track.not_enough_tracks": "Not holding enough tracks",
    "create.track.opposing_slopes": "Cannot connect opposing slopes",
    "create.track.original_missing": "Original block removed, Sneak-click to reset",
    "create.track.perpendicular": "Cannot connect perpendicularly",
    "create.track.second_point": "Place track or select a second point",
    "create.track.selection_cleared": "Selection Cleared",
    "create.track.slope_turn": "Cannot enter or leave slope on a turn",
    "create.track.too_far": "Too far away",
    "create.track.too_sharp": "Turn too sharp",
    "create.track.too_steep": "Slope too steep",
    "create.track.turn_90": "Can only turn up to 90 Degrees",
    "create.track.turn_start": "Cannot start connection from a Turn",
    "create.track.valid_connection": "Can Connect ✔",
}

TAGS = {
    "create:block/tracks": ["create:track"],
    "create:block/girdable_tracks": ["create:track"],
    "create:item/tracks": ["create:track"],
    "c:block/relocation_not_supported": ["create:track"],
    # `data/create/tags/item/sleepers`: what the sequenced assembly starts from.
    "create:item/sleepers": ["minecraft:stone_slab", "minecraft:smooth_stone_slab", "minecraft:andesite_slab"],
}

_NUGGETS = [{"tag": "c:nuggets/iron"}, {"tag": "c:nuggets/zinc"}]
_DEPLOY = {"type": "create:deploying",
           "ingredients": [{"item": "create:incomplete_track"}, _NUGGETS],
           "results": [{"id": "create:incomplete_track"}]}
RECIPES = {
    # `data/create/recipe/sequenced_assembly/track.json`, as Create's datagen writes it.
    "sequenced_assembly/track": {
        "type": "create:sequenced_assembly",
        "ingredient": {"tag": "create:sleepers"},
        "results": [{"id": "create:track"}],
        "sequence": [
            _DEPLOY,
            _DEPLOY,
            {"type": "create:pressing",
             "ingredients": [{"item": "create:incomplete_track"}],
             "results": [{"id": "create:incomplete_track"}]},
        ],
        "transitional_item": {"id": "create:incomplete_track"},
    },
}
