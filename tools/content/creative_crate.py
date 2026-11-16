"""The Creative Crate: `create:creative_crate`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.CREATIVE_CRATE` (`BuilderTransformers.crate("creative")`:
`SharedProperties.stone`, `axeOrPickaxe`, its item `Rarity.EPIC`; `MapColor.COLOR_PURPLE`),
`CreativeCrateBlock` (`WrenchableDirectionalBlock`: `facing`), `AllShapes.CRATE_BLOCK_SHAPE`
(`cuboid(1, 0, 1, 15, 14, 15)`), the blockstate Create's datagen writes (the one model,
`block/crate/creative/single`, for every facing). Create has no recipe for it. The behaviour is
`native/api/src/machines/creative_crate.rs` (known differences at its top).
"""
from gen_content import ANDESITE, FACINGS, PURPLE, block

BLOCKS = {
    "creative_crate": block(
        {"facing": {"type": "enum", "values": FACINGS}}, {"facing": "north"},
        ANDESITE, PURPLE,
        {"default": [[1 / 16, 0, 1 / 16, 15 / 16, 14 / 16, 15 / 16]]},
        render_shape="MODEL"),
}
# `.properties(p -> type.equals("creative") ? p.rarity(Rarity.EPIC) : p)` on the item.
BLOCKS["creative_crate"]["components"] = {"minecraft:rarity": "epic"}

NAMES = {"block.create.creative_crate": "Creative Crate"}
TAB = ["creative_crate"]
