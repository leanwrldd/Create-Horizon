"""Sloped, vertical and sideways belts (Belts III): the models the belt's block entity draws on
each slope, read from the player's own Create jar when the package is built. The behaviour is
`native/api/src/belt.rs` and `machines/belt_carry.rs`; the block's shapes for every slope are
`native/kinetics/src/rules/belt_shapes.rs`, written to `_belt_shapes_gen.json` and read by
`gen_content.py`.
"""

# `BeltRenderer.getBeltPartial`: a diagonal belt has one model per part and no bottom half.
# `BeltGenerator.getModel`: the casing is `belt_casing/<slope>_<part>`; a vertical casing is the
# sideways model, turned. The flat ones are `belts_ii.py`'s and `build_content.py`'s.
MODELS = [
    *(f"block/belt/diagonal_{part}" for part in ("start", "middle", "end")),
    *(f"block/belt_casing/{kind}_{part}"
      for kind in ("diagonal", "sideways") for part in ("start", "middle", "pulley", "end")),
]
