"""The sixteen Seats (module `seat`): `create:<colour>_seat`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.SEATS` (`DyedBlockList`, `SharedProperties.wooden` =
stripped spruce wood's 2.0 / 2.0 and sound, `mapColor(colour)`, axe only, tags `create:seats`, the two
datagen recipes `crafting/kinetics/<colour>_seat` and `..._from_other_seat`), `SeatBlock`,
`AllShapes.SEAT` (0..8 texels high), `SEAT_COLLISION` (6) and `SEAT_COLLISION_PLAYERS` (3).
Create's blockstates and models are imported from the player's jar.

In Create's creative tab only the red seat shows (`makeVisibilityFunc`: the others are search-only);
Horizon's tab has no search-only entries, so all sixteen are listed, as the valve handles are.
Waterlogging is left out. See `machines/seat.rs` for how sitting works.
"""

from gen_content import WOODEN, block

# `DyeColor`: `(name, MapColor id of getMapColor())`, in ordinal order.
COLOURS = [("white", 8), ("orange", 15), ("magenta", 16), ("light_blue", 17), ("yellow", 18),
           ("lime", 19), ("pink", 20), ("gray", 21), ("light_gray", 22), ("cyan", 23),
           ("purple", 24), ("blue", 25), ("brown", 26), ("green", 27), ("red", 28), ("black", 29)]

# `AllShapes.SEAT`, `SEAT_COLLISION`, `SEAT_COLLISION_PLAYERS`: cuboid(0, 0, 0, 16, h, 16).
_outline = {"default": [[0, 0, 0, 1, 8 / 16, 1]]}
_collision = {"default": [[0, 0, 0, 1, 6 / 16, 1]]}
_player_collision = {"default": [[0, 0, 0, 1, 3 / 16, 1]]}

BLOCKS, NAMES, RECIPES, MINEABLE, TAGS = {}, {}, {}, {}, {"create:block/seats": [], "create:item/seats": []}
PALETTES_TAB = []

for colour, map_color in COLOURS:
    name = f"{colour}_seat"
    seat = block(None, None, WOODEN, map_color, _outline, render_shape="MODEL")
    seat["collision_shapes"] = _collision
    seat["collision_context_shapes"] = {"player": _player_collision}
    seat["properties"]["occludes"] = False
    BLOCKS[name] = seat
    NAMES[f"block.create.{name}"] = f"{colour.replace('_', ' ').title()} Seat"
    MINEABLE[name] = ["axe"]  # `axeOnly()`
    TAGS["create:block/seats"].append(f"create:{name}")
    TAGS["create:item/seats"].append(f"create:{name}")
    TAGS[f"c:item/dyes/{colour}"] = [f"minecraft:{colour}_dye"]
    PALETTES_TAB.append(name)
    # `AllBlocks.SEATS`' recipes: the wool and a wooden slab, and a dye on another seat.
    RECIPES[f"crafting/kinetics/{name}"] = {
        "type": "minecraft:crafting_shapeless", "category": "building",
        "ingredients": [f"minecraft:{colour}_wool", "#minecraft:wooden_slabs"],
        "result": {"id": f"create:{name}"},
    }
    RECIPES[f"crafting/kinetics/{name}_from_other_seat"] = {
        "type": "minecraft:crafting_shapeless", "category": "building",
        "ingredients": [f"#c:dyes/{colour}", "#create:seats"],
        "result": {"id": f"create:{name}"},
    }
