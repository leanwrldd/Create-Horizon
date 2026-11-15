"""Belt casings: the models drawn over a cased belt block (`BeltBlock.CASING`,
`BeltBlockEntity.setCasingType`), read from the player's own Create jar when the package is built.
The behaviour is `native/api/src/machines/belts_ii.rs`."""

# `blockstates/belt.json`'s `casing=true` models for a horizontal belt, one per part.
MODELS = [f"block/belt_casing/horizontal_{part}" for part in ("start", "middle", "pulley", "end")]
