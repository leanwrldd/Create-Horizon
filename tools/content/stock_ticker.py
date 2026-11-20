"""The Stock Ticker: `create:stock_ticker`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.STOCK_TICKER` (`SharedProperties.softMetal`, `SoundType.GLASS`, `axeOrPickaxe`,
a `LogisticallyLinkedBlockItem`, blockstate `horizontalBlock` with `block/stock_ticker`), `StockTickerBlock` (`facing`),
`AllShapes.STOCK_TICKER` (`shape(1, 0, 1, 15, 4, 15).add(2, 0, 2, 14, 16, 14)`),
`data/create/recipe/crafting/logistics/stock_ticker`. The `_clear` recipe isn't declared (no frequency in the item). The
behaviour is `mod/src/machines/stock_ticker.rs` and `logistics.rs`.
"""
from gen_content import HORIZONTAL_FACING_STATE, block

# `SharedProperties.softMetal`, with `SoundType.GLASS`; no map colour in `AllBlocks`.
SOFT_METAL = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True}

BLOCKS = {
    "stock_ticker": block(
        HORIZONTAL_FACING_STATE, {"facing": "north"}, SOFT_METAL, 0,
        {"default": [[1 / 16, 0, 1 / 16, 15 / 16, 4 / 16, 15 / 16], [2 / 16, 0, 2 / 16, 14 / 16, 1, 14 / 16]]},
        sounds="minecraft:glass", render_shape="MODEL"),
}

NAMES = {"block.create.stock_ticker": "Stock Ticker"}
TAB = ["stock_ticker"]

RECIPES = {
    "crafting/logistics/stock_ticker": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["C", "A", "B"],
        "key": {"A": "create:stock_link", "B": "#c:ingots/gold", "C": "#c:glass_blocks"},
        "result": {"id": "create:stock_ticker", "count": 1},
    },
}
