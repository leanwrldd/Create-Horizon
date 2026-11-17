"""The Extendo Grip (module `extendo_grip`): `create:extendo_grip`.

Create `mc1.21.1/dev` at 4148fd8: `AllItems.EXTENDO_GRIP` (`ExtendoGripItem`: `durability(200)`,
`Rarity.UNCOMMON`, tag `minecraft:durability_enchantable`) and its recipe `mechanical_crafting/extendo_grip`
(a `create:mechanical_crafting` recipe, as the Mechanical Crafter reads them). Behaviour:
`native/api/src/machines/extendo_grip.rs`.
"""

ITEMS = {
    "extendo_grip": {
        "template": "minecraft:iron_ingot", "max_stack_size": 1, "rarity": "uncommon",
        "components": {"minecraft:max_damage": 200},
    },
}

NAMES = {"item.create.extendo_grip": "Extendo Grip"}

RECIPES = {
    # `recipe/mechanical_crafting/extendo_grip`, as Create's datagen writes it.
    "mechanical_crafting/extendo_grip": {
        "type": "create:mechanical_crafting",
        "accept_mirrored": False,
        "category": "misc",
        "key": {"H": {"item": "create:brass_hand"}, "L": {"tag": "c:ingots/brass"},
                "R": {"item": "create:precision_mechanism"}, "S": {"tag": "c:rods/wooden"}},
        "pattern": [" L ", " R ", "SSS", "SSS", " H "],
        "result": {"count": 1, "id": "create:extendo_grip"},
        "show_notification": False,
    },
}

TAB = ["extendo_grip"]
