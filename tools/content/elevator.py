"""The Elevator Pulley and the Elevator Contact (module `elevator`).

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.ELEVATOR_PULLEY` (`SharedProperties.softMetal`,
`MapColor.TERRACOTTA_BROWN`, axe or pickaxe, `CStress.setImpact(4.0)`, `HorizontalKineticBlock`:
`facing`; `AllShapes.ELEVATOR_PULLEY`), `ELEVATOR_CONTACT` (`softMetal`, `MapColor.TERRACOTTA_YELLOW`,
axe or pickaxe, `ElevatorContactBlock`: `facing`, `powered`, `calling`, `powering`) and
`recipe/crafting/kinetics/elevator_pulley`. Differences from Create: the contact has no
Redstone Contact to be made from, so it is only in the creative tab and drops itself; it doesn't
light up while it powers (a block's light here is one number); waterlogging is left out. The
behaviour is `native/api/src/machines/elevator.rs`.
"""

from gen_content import HORIZONTAL_FACING_STATE, FACING_STATE, block

TERRACOTTA_BROWN, TERRACOTTA_YELLOW = 48, 40

# `SharedProperties.softMetal` (a gold block).
SOFT_METAL = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
              "sounds": "minecraft:gold_block"}

# `AllShapes.ELEVATOR_PULLEY`, written facing east: two plates and a core. The north and south
# shapes have x and z swapped.
_EAST = [[0, 0, 0, 1, 1, 2 / 16], [0, 0, 14 / 16, 1, 1, 1], [2 / 16, 0, 2 / 16, 14 / 16, 14 / 16, 14 / 16]]
_NORTH = [[b[2], b[1], b[0], b[5], b[4], b[3]] for b in _EAST]
PULLEY_SHAPES = {"property": "facing",
                 "values": {"east": _EAST, "west": _EAST, "north": _NORTH, "south": _NORTH}}

PULLEY_STATES = HORIZONTAL_FACING_STATE
CONTACT_STATES = {
    **FACING_STATE,
    "powered": {"type": "boolean"},
    "calling": {"type": "boolean"},
    "powering": {"type": "boolean"},
}
CONTACT_DEFAULT = {"facing": "south", "powered": "false", "calling": "false", "powering": "false"}

BLOCKS = {
    "elevator_pulley": block(PULLEY_STATES, {"facing": "north"}, SOFT_METAL, TERRACOTTA_BROWN,
                             PULLEY_SHAPES, render_shape="MODEL"),
    "elevator_contact": block(CONTACT_STATES, CONTACT_DEFAULT, SOFT_METAL, TERRACOTTA_YELLOW,
                              render_shape="MODEL"),
}

# What the block entity draws besides the block's model (`AllPartialModels.ELEVATOR_COIL`,
# `ELEVATOR_BELT`, `ELEVATOR_BELT_HALF`, `ELEVATOR_MAGNET`).
MODELS = ["block/elevator_pulley/rope_coil", "block/elevator_pulley/rope",
          "block/elevator_pulley/rope_half", "block/elevator_pulley/pulley_magnet"]

TAB = ["elevator_pulley", "elevator_contact"]

NAMES = {
    "block.create.elevator_pulley": "Elevator Pulley",
    "block.create.elevator_contact": "Elevator Contact",
}

RECIPES = {
    # `crafting/kinetics/elevator_pulley`, in Horizon's 26.3 form.
    "crafting/kinetics/elevator_pulley": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["B", "C", "I"],
        "key": {"B": "create:brass_casing", "C": "minecraft:dried_kelp_block",
                "I": "#c:plates/iron"},
        "result": {"id": "create:elevator_pulley", "count": 1},
    },
}
