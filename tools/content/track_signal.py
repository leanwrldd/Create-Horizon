"""The Train Signal: `create:track_signal` (module `track_signal`).

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.TRACK_SIGNAL` (`SignalBlock`; `SharedProperties.softMetal`,
`MapColor.PODZOL`, `noOcclusion`, `SoundType.NETHERITE_BLOCK`, `pickaxeOnly`, a `TrackTargetingBlockItem` of
`EdgePointType.SIGNAL` with a custom item model), `SignalBlock` (`type`: `entry_signal` or `cross_signal`,
`powered`; no `getShape`, so a full block; `waterlogged` isn't a property of it),
`recipe/crafting/kinetics/track_signal` (Create's datagen file), `block.create.track_signal` and the lang keys
`SignalBlock.onWrenched` shows.

The block's model is Create's blockstate from the player's jar (`block_entry_signal` / `block_cross_signal`); the
indicator light `SignalVisual` swaps is drawn by the block entity (`machines/track_signal.rs`). The overlay it puts
on the track (`TRACK_SIGNAL_OVERLAY`) is the track's own drawing and not here.
"""

from gen_content import PODZOL, block

# `SharedProperties.softMetal`: a gold block (3.0 / 6.0), with `SoundType.NETHERITE_BLOCK`.
SOFT_METAL = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
              "sounds": "minecraft:netherite_block"}

BLOCKS = {
    "track_signal": block({
        "type": {"type": "enum", "values": ["entry_signal", "cross_signal"]},
        "powered": {"type": "boolean"},
    }, {"type": "entry_signal", "powered": "false"}, SOFT_METAL, PODZOL, render_shape="MODEL"),
}
BLOCKS["track_signal"]["properties"]["occludes"] = False  # `noOcclusion`

# What the block entity draws besides the block's own model (`SignalVisual`: `AllPartialModels.SIGNAL_ON` while the
# light is red, `SIGNAL_OFF` otherwise).
MODELS = ["block/track_signal/indicator_on", "block/track_signal/indicator_off"]

TAB = ["track_signal"]

NAMES = {
    "block.create.track_signal": "Train Signal",
    "create.track_signal.cannot_change_mode": "Unable to switch mode of this Signal",
    "create.track_signal.mode_change.cross_signal": "-> Allow passage if section fully traversable",
    "create.track_signal.mode_change.entry_signal": "-> Allow passage if section unoccupied",
    "create.track_signal.mode_controlled_by_computer": "Signal mode is controlled by computer",
}

RECIPES = {
    # `data/create/recipe/crafting/kinetics/track_signal.json`.
    "crafting/kinetics/track_signal": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:railway_casing", "create:electron_tube"],
        "result": {"id": "create:track_signal", "count": 4},
    },
}

PICKAXE_ONLY = {"track_signal"}
