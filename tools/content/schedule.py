"""The Train Schedule: `create:schedule` (module `schedule`).

Create `mc1.21.1/dev` at 4148fd8: `AllItems.SCHEDULE` (`ScheduleItem`, default properties: stacks to 64; the model
`item/schedule`, a flat item), `AllDataComponents.TRAIN_SCHEDULE` (`create:train_schedule`, the item's
`Schedule` as an NBT compound), `recipe/crafting/kinetics/schedule` and `recipe/crafting/appliances/schedule_clear`
(Create's datagen files), the lang keys of `ScheduleItem`, `ScheduleItemEntityInteraction` and the editor's summaries.

The schedule is edited with the `/schedule` command instead of Create's screen (`machines/schedule.rs`).
"""

# `Item::new` with default properties (as `materials.item()`): an iron nugget's template, which has none of an ingot's
# components, and 64 as its stack size.
ITEMS = {"schedule": {"template": "minecraft:iron_nugget", "max_stack_size": 64}}

# The Train Hat a seated conductor wears (`AllPartialModels.TRAIN_HAT`; `machines/train_hat.rs`).
MODELS = ["entity/train_hat"]

TAB = ["schedule"]


NAMES = {
    "item.create.schedule": "Train Schedule",
    "create.schedule.applied_to_train": "Train is now following this Schedule",
    "create.schedule.auto_removed_from_train": "Auto-Schedule discarded",
    "create.schedule.continued": "Schedule resumed",
    "create.schedule.no_stops": "This Schedule does not have any Stops yet",
    "create.schedule.non_controlling_seat": "Conductor needs to sit in front of a Controls block",
    "create.schedule.remove_with_empty_hand": "Remove current Schedule with an Empty Hand",
    "create.schedule.removed_from_train": "Schedule retrieved from Train",
}

RECIPES = {
    # `data/create/recipe/crafting/kinetics/schedule.json`.
    "crafting/kinetics/schedule": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["#c:plates/obsidian", "minecraft:paper"],
        "result": {"id": "create:schedule", "count": 4},
    },
    # `data/create/recipe/crafting/appliances/schedule_clear.json`: a schedule alone gives a clean one.
    "crafting/appliances/schedule_clear": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:schedule"],
        "result": {"id": "create:schedule"},
    },
}
