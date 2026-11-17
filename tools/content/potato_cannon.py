"""The Potato Cannon (module `potato_cannon`): `create:potato_cannon`.

Create `mc1.21.1/dev` at 4148fd8: `AllItems.POTATO_CANNON` (`PotatoCannonItem`, `durability(100)`), the damage
type `data/create/damage_type/potato_cannon.json`, and `recipe/mechanical_crafting/potato_cannon` (as the
Mechanical Crafter reads them). The 24 ammunition types of `data/create/create/potato_projectile/type/` are
in `native/kinetics/src/rules/potato_cannon.rs`; the behaviour is `native/api/src/machines/potato_cannon.rs`.
"""

ITEMS = {
    "potato_cannon": {
        "template": "minecraft:iron_ingot", "max_stack_size": 1,
        "components": {"minecraft:max_damage": 100},
    },
}

NAMES = {
    "item.create.potato_cannon": "Potato Cannon",
    "death.attack.create.potato_cannon": "%1$s was hit by a flying snack",
    "death.attack.create.potato_cannon.player": "%1$s was hit by a flying snack fired by %2$s",
}

RECIPES = {
    # `recipe/mechanical_crafting/potato_cannon`, as Create's datagen writes it.
    "mechanical_crafting/potato_cannon": {
        "type": "create:mechanical_crafting",
        "accept_mirrored": True,
        "category": "misc",
        "key": {"C": {"tag": "c:ingots/copper"}, "L": {"item": "create:andesite_alloy"},
                "R": {"item": "create:precision_mechanism"}, "S": {"item": "create:fluid_pipe"}},
        "pattern": ["LRSSS", "CC "],
        "result": {"count": 1, "id": "create:potato_cannon"},
    },
}

# `data/create/damage_type/potato_cannon.json`.
FILES = {
    "data/create/damage_type/potato_cannon.json": {
        "exhaustion": 0.0, "message_id": "create.potato_cannon", "scaling": "when_caused_by_living_non_player"},
}

TAB = ["potato_cannon"]
