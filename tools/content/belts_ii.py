"""Belt casings: the models drawn over a cased belt block (`BeltBlock.CASING`,
`BeltBlockEntity.setCasingType`), read from the player's own Create jar when the package is built.
The behaviour is `native/api/src/machines/belts_ii.rs`."""

# `blockstates/belt.json`'s `casing=true` models for a horizontal belt, one per part.
MODELS = [f"block/belt_casing/horizontal_{part}" for part in ("start", "middle", "pulley", "end")]

# The sprites a belt is drawn shifted onto (`AllSpriteShifts`: `BELT` and `BELT_DIAGONAL` scroll
# over `belt_scroll` and `belt_diagonal_scroll`, `DYED_*` over `belt/<colour>_scroll` and
# `belt/<colour>_diagonal_scroll`, and `ANDESIDE_BELT_CASING` swaps the brass casing sprite for
# `belt/andesite_belt_casing`). No model of Create's names them, so the game would not load
# them: this model does, one hidden face each, and is never drawn (`belt.rs` `SPRITES`).
# `DyeColor.getSerializedName`.
DYES = ("white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray",
        "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black")
SPRITES = ["block/belt_scroll", "block/belt_diagonal_scroll",
           *(f"block/belt/{dye}_scroll" for dye in DYES),
           *(f"block/belt/{dye}_diagonal_scroll" for dye in DYES),
           "block/belt/andesite_belt_casing"]

FILES = {
    "assets/create/models/block/belt_scroll_sprites.json": {
        "textures": {f"s{i}": f"create:{sprite}" for i, sprite in enumerate(SPRITES)},
        "elements": [
            {"from": [0, 0, 0], "to": [1, 1, 1],
             "faces": {"down": {"uv": [0, 0, 16, 16], "texture": f"#s{i}"}}}
            for i in range(len(SPRITES))
        ],
    },
}
