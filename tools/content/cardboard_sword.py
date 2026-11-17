"""The Cardboard Sword: `create:cardboard_sword`.

Create `mc1.21.1/dev` at 4148fd8: `AllItems.CARDBOARD_SWORD` (`CardboardSwordItem`, `stacksTo(1)`, burn
time 1000, `SwordItem.createAttributes(AllToolMaterials.CARDBOARD, 3, 1)`: the tier is 0 uses, speed 1,
damage bonus 2, so 5 attack damage and +1 attack speed; model `item/cardboard_sword`),
`recipe/crafting/appliances/cardboard_sword`. It never hurts: a hit only knocks back
(`native/api/src/machines/cardboard_sword.rs`).
"""


def burn(ticks):
    """`ItemBuilder.burnTime(ticks)` (as `materials.py` writes it)."""
    return {"minecraft:cooking_fuel": {
        "burn_time": {"type": "minecraft:div", "left": ticks, "right": {
            "type": "minecraft:conditional", "condition": "minecraft:block/fast_cooking",
            "on_false": "minecraft:cooking/normal_burn_time_reduction_factor",
            "on_true": "minecraft:cooking/fast_burn_time_reduction_factor"}},
        "speed_multiplier": "minecraft:cooking/speed_default"}}

ITEMS = {
    "cardboard_sword": {
        "template": "minecraft:iron_sword",
        "max_stack_size": 1,
        "components": {
            # A tier of 0 uses: it doesn't wear.
            "!minecraft:max_damage": {},
            "!minecraft:repairable": {},
            "minecraft:attribute_modifiers": [
                {"type": "minecraft:attack_damage", "id": "minecraft:base_attack_damage", "amount": 5.0,
                 "operation": "add_value", "slot": "mainhand"},
                {"type": "minecraft:attack_speed", "id": "minecraft:base_attack_speed", "amount": 1.0,
                 "operation": "add_value", "slot": "mainhand"},
            ],
            **burn(1000),
        },
    },
}

NAMES = {"item.create.cardboard_sword": "Cardboard Sword"}
TAB = ["cardboard_sword"]

RECIPES = {
    "crafting/appliances/cardboard_sword": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["P", "P", "S"],
        "key": {"P": "create:cardboard", "S": "#c:rods/wooden"},
        "result": {"id": "create:cardboard_sword", "count": 1},
    },
}
