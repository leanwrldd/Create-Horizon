"""Generate the Horizon declarations and item definitions for Create."""

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent / "content"
ASSETS = ROOT / "assets" / "create"
DATA = ROOT / "data" / "create"

AXES = ["x", "y", "z"]
FACINGS = ["north", "south", "east", "west", "up", "down"]


def _tables(value):
    """The new loader (launcher 0.2.70+) names a shape or light table's properties as a list: `{"property": p,
    "values": v}` becomes `{"properties": [p], "values": v}` (reference/content-mods: `shapes`, `state_light`)."""
    if isinstance(value, dict):
        for key in ("shapes", "collision_shapes", "state_light"):
            table = value.get(key)
            if isinstance(table, dict) and "property" in table:
                value[key] = {"properties": [table["property"]],
                              **{k: v for k, v in table.items() if k != "property"}}
        for inner in value.values():
            _tables(inner)
    elif isinstance(value, list):
        for inner in value:
            _tables(inner)
    return value


def write(path: Path, value) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    if "horizon" in path.parts and "block" in path.parts:
        _tables(value)
    if "horizon" in path.parts and "item" in path.parts and isinstance(value, dict) and "place_block" in value:
        # the new item schema places a block with `block` (the old loader's `place_block`)
        value["block"] = value.pop("place_block")
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8", newline="\n")


# --- shapes, in blocks, written for the y axis and turned for the others -----------------

def turn(box, axis):
    """A y-axis box [x0, y0, z0, x1, y1, z1] turned to lie along `axis`."""
    x0, y0, z0, x1, y1, z1 = box
    if axis == "y":
        return box
    if axis == "x":
        return [y0, x0, z0, y1, x1, z1]
    return [x0, z0, y0, x1, z1, y1]


def axis_shapes(boxes):
    return {"property": "axis", "values": {a: [turn(b, a) for b in boxes] for a in AXES}}


SHAFT = [0.375, 0, 0.375, 0.625, 1, 0.625]
SHAPES = {
    "shaft": [SHAFT],
    "cogwheel": [SHAFT, [0.125, 0.375, 0.125, 0.875, 0.625, 0.875]],
    "large_cogwheel": [SHAFT, [0, 0.375, 0, 1, 0.625, 1]],
}


def motor_shape(facing):
    """The motor's body: 10 texels square, 14 long, its back against the face behind it."""
    lo, hi = 0.1875, 0.8125
    along = {"up": (1, 1), "down": (1, -1), "south": (2, 1), "north": (2, -1),
             "east": (0, 1), "west": (0, -1)}[facing]
    box = [lo, lo, lo, hi, hi, hi]
    axis, sign = along
    box[axis], box[axis + 3] = (0, 0.875) if sign > 0 else (0.125, 1)
    return box


def facing_shapes(boxes):
    """Boxes written for facing up, turned so up points along each facing, as Catnip's
    `VoxelShaper.forDirectional` does (every box here is square across, so a turn only moves
    its height onto the facing's axis)."""
    def turned(box, facing):
        x0, y0, z0, x1, y1, z1 = box
        axis, sign = {"up": (1, 1), "down": (1, -1), "south": (2, 1), "north": (2, -1),
                      "east": (0, 1), "west": (0, -1)}[facing]
        lo, hi = (y0, y1) if sign > 0 else (1 - y1, 1 - y0)
        out = [x0, x0, x0, x1, x1, x1]
        out[axis], out[axis + 3] = lo, hi
        return out
    return {"property": "facing", "values": {f: [turned(b, f) for b in boxes] for f in FACINGS}}


# Create's `AllShapes.CRANK`: shape(5, 0, 5, 11, 6, 11).add(1, 3, 1, 15, 8, 15), in texels.
CRANK = [[5 / 16, 0, 5 / 16, 11 / 16, 6 / 16, 11 / 16],
         [1 / 16, 3 / 16, 1 / 16, 15 / 16, 8 / 16, 15 / 16]]

# What each block copies in Create (`AllBlocks`, `SharedProperties`), with vanilla's numbers for
# those blocks as Horizon's prepared game data has them.
ANDESITE = {"hardness": 1.5, "explosion_resistance": 6.0, "requires_tool": True,
            "sounds": "minecraft:andesite"}  # SharedProperties.stone
WOODEN = {"hardness": 2.0, "explosion_resistance": 2.0, "requires_tool": False,
          "sounds": "minecraft:stripped_spruce_wood"}  # SharedProperties.wooden
WOOD_SOUND = "minecraft:oak_planks"  # SoundType.WOOD
# MapColor ids.
METAL, DIRT, GRAY, PURPLE, PODZOL = 6, 10, 21, 24, 34


def block(extra_states, default, base, map_color, shapes=None, sounds=None,
          render_shape="ENTITYBLOCK_ANIMATED", template="minecraft:glass"):
    properties = dict(base, map_color=map_color, render_shape=render_shape)
    if sounds:
        properties["sounds"] = sounds
    value = {
        "template": template,
        "item": True,
        "properties": properties,
    }
    if extra_states:
        value["state_properties"] = extra_states
        value["default_state"] = default
    if shapes:
        value["shapes"] = shapes
        value["collision_shapes"] = shapes
    return value


AXIS_STATE = {"axis": {"type": "enum", "values": AXES}}
FACING_STATE = {"facing": {"type": "enum", "values": FACINGS}}
HORIZONTAL_FACING_STATE = {"facing": {"type": "enum", "values": ["north", "south", "west", "east"]}}
# `BeltBlock`: HORIZONTAL_FACING, SLOPE, PART and CASING (waterlogging left out, as elsewhere).
BELT_STATES = {
    **HORIZONTAL_FACING_STATE,
    "slope": {"type": "enum",
              "values": ["horizontal", "upward", "downward", "vertical", "sideways"]},
    "part": {"type": "enum", "values": ["start", "middle", "end", "pulley"]},
    "casing": {"type": "boolean"},
}
BELT_DEFAULT = {"facing": "north", "slope": "horizontal", "part": "start", "casing": "false"}
# `AllBlocks.BELT`: `strength(0.8f)`, `SoundType.WOOL`, no tool needed.
BELT = {"hardness": 0.8, "explosion_resistance": 0.8, "requires_tool": False,
        "sounds": "minecraft:white_wool"}
# `BeltShapes` for every state: the outline and the collision (the shape under
# `AllShapes.BELT_COLLISION_MASK`), in pixels, made by the rules (`rules/belt_shapes.rs`, which
# writes `tools/content/_belt_shapes_gen.json`) and turned to blocks here.
def belt_shapes(which):
    table = json.loads((Path(__file__).resolve().parent / "content" / "_belt_shapes_gen.json")
                       .read_text(encoding="utf-8"))
    return {"properties": ["facing", "slope", "part", "casing"], "values": {
        key: [[v / 16 for v in box] for box in value[which]] for key, value in table.items()}}


BELT_SHAPES = belt_shapes("outline")
BLOCKS = {
    "shaft": block(AXIS_STATE, {"axis": "y"}, ANDESITE, METAL, axis_shapes(SHAPES["shaft"])),
    "cogwheel": block(AXIS_STATE, {"axis": "y"}, ANDESITE, DIRT,
                      axis_shapes(SHAPES["cogwheel"]), sounds=WOOD_SOUND),
    "large_cogwheel": block(AXIS_STATE, {"axis": "y"}, ANDESITE, DIRT,
                            axis_shapes(SHAPES["large_cogwheel"]), sounds=WOOD_SOUND),
    "gearbox": block(AXIS_STATE, {"axis": "y"}, ANDESITE, PODZOL),
    "creative_motor": block(
        FACING_STATE, {"facing": "north"}, ANDESITE, PURPLE,
        {"property": "facing", "values": {f: [motor_shape(f)] for f in FACINGS}},
    ),
    "hand_crank": block(FACING_STATE, {"facing": "up"}, WOODEN, PODZOL, facing_shapes(CRANK)),
    # Create gives the wheel no shape of its own: a full block.
    "water_wheel": block(AXIS_STATE, {"axis": "z"}, WOODEN, DIRT),
    # `AllShapes.CASING_13PX` facing up. Its model sits in the chunk mesh; the block entity
    # draws only the stack on top. No waterlogging yet, as for the other blocks.
    "depot": block({}, {}, ANDESITE, GRAY, {"default": [[0, 0, 0, 1, 13 / 16, 1]]},
                   render_shape="MODEL"),
    # `AllShapes.CASING_14PX` facing down, what players meet (`MechanicalPressBlock.getShape`);
    # other entities get the hollow `MECHANICAL_PROCESSOR_SHAPE`, not modelled here. The casing
    # sits in the chunk mesh; the block entity draws the shaft and the moving head.
    "mechanical_press": block(HORIZONTAL_FACING_STATE, {"facing": "north"}, ANDESITE, PODZOL,
                              {"default": [[0, 2 / 16, 0, 1, 1, 1]]}, render_shape="MODEL"),
    "belt": block(BELT_STATES, BELT_DEFAULT, BELT, GRAY, BELT_SHAPES),
}
BLOCKS["belt"]["collision_shapes"] = belt_shapes("collision")
# The belt has no item of its own: the Belt Connector places it (`BeltConnectorItem`).
BLOCKS["belt"]["item"] = False
# `pickaxeOnly()` in Create; every other block here is `axeOrPickaxe()`.
PICKAXE_ONLY = {"shaft", "creative_motor"}
ITEMS = {
    "andesite_alloy": {"template": "minecraft:iron_ingot", "max_stack_size": 64},
    "iron_sheet": {"template": "minecraft:iron_ingot", "max_stack_size": 64},
    "belt_connector": {"template": "minecraft:iron_ingot", "max_stack_size": 64},
    "wrench": {
        "template": "minecraft:iron_ingot",
        "max_stack_size": 1,
        # Horizon checks the tool before rolling drops for a player's break.
        "components": {"minecraft:tool": {
            "rules": [{
                "blocks": [f"create:{name}" for name, block_data in BLOCKS.items()
                           if block_data["properties"]["requires_tool"]],
                "correct_for_drops": True,
            }],
            "damage_per_block": 0,
        }},
    },
}
NAMES = {
    "itemGroup.create.palettes": "Create's Building Blocks",
    "block.create.shaft": "Shaft",
    "block.create.cogwheel": "Cogwheel",
    "block.create.large_cogwheel": "Large Cogwheel",
    "block.create.gearbox": "Gearbox",
    "block.create.creative_motor": "Creative Motor",
    "block.create.hand_crank": "Hand Crank",
    "block.create.water_wheel": "Water Wheel",
    "block.create.depot": "Depot",
    "block.create.mechanical_press": "Mechanical Press",
    "item.create.andesite_alloy": "Andesite Alloy",
    "item.create.iron_sheet": "Iron Sheet",
    "block.create.belt": "Mechanical Belt",
    "item.create.belt_connector": "Mechanical Belt",
    "item.create.wrench": "Wrench",
    "itemGroup.create.base": "Create",
}


RECIPES = {
    "andesite_alloy": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["NA", "AN"],
        "key": {"A": "minecraft:andesite", "N": "#c:nuggets/iron"},
        "result": {"id": "create:andesite_alloy"},
    },
    "shaft": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["A", "A"], "key": {"A": "create:andesite_alloy"},
        "result": {"id": "create:shaft", "count": 8},
    },
    "cogwheel": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:shaft", "#minecraft:planks"],
        "result": {"id": "create:cogwheel"},
    },
    "large_cogwheel": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:shaft", "#minecraft:planks", "#minecraft:planks"],
        "result": {"id": "create:large_cogwheel"},
    },
    # `crafting/kinetics/gearbox`. The casing is declared in tools/content/materials.py.
    "gearbox": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": [" C ", "CAC", " C "],
        "key": {"C": "create:cogwheel", "A": "create:andesite_casing"},
        "result": {"id": "create:gearbox"},
    },
    "wrench": {
        "type": "minecraft:crafting_shaped", "category": "equipment",
        "pattern": ["GG", "GC", " S"],
        "key": {"G": "#c:plates/gold", "C": "create:cogwheel", "S": "#c:rods/wooden"},
        "result": {"id": "create:wrench"},
    },
    "hand_crank": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["PPP", "  A"],
        "key": {"P": "#minecraft:planks", "A": "create:andesite_alloy"},
        "result": {"id": "create:hand_crank"},
    },
    "water_wheel": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["PPP", "PSP", "PPP"],
        "key": {"P": "#minecraft:planks", "S": "create:shaft"},
        "result": {"id": "create:water_wheel"},
    },
    # `crafting/kinetics/mechanical_press`.
    "mechanical_press": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["S", "C", "I"],
        "key": {"S": "create:shaft", "C": "create:andesite_casing",
                "I": "#c:storage_blocks/iron"},
        "result": {"id": "create:mechanical_press"},
    },
    # `crafting/kinetics/depot`.
    "depot": {
        "type": "minecraft:crafting_shapeless", "category": "misc",
        "ingredients": ["create:andesite_alloy", "create:andesite_casing"],
        "result": {"id": "create:depot"},
    },
    # `crafting/kinetics/belt_connector`.
    "belt_connector": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["DDD", "DDD"], "key": {"D": "minecraft:dried_kelp"},
        "result": {"id": "create:belt_connector"},
    },
    # Create's `pressing/iron_ingot`, as its datagen writes it. The press reads this type itself
    # (`native/api/src/press.rs`), so it keeps Create's 1.21.1 form.
    "pressing/iron_ingot": {
        "type": "create:pressing",
        "ingredients": [{"tag": "c:ingots/iron"}],
        "results": [{"id": "create:iron_sheet"}],
    },
    # The other sheets (tools/content/materials.py declares them).
    "pressing/gold_ingot": {
        "type": "create:pressing",
        "ingredients": [{"tag": "c:ingots/gold"}],
        "results": [{"id": "create:golden_sheet"}],
    },
    "pressing/copper_ingot": {
        "type": "create:pressing",
        "ingredients": [{"tag": "c:ingots/copper"}],
        "results": [{"id": "create:copper_sheet"}],
    },
    "pressing/brass_ingot": {
        "type": "create:pressing",
        "ingredients": [{"tag": "c:ingots/brass"}],
        "results": [{"id": "create:brass_sheet"}],
    },
    "pressing/cardboard": {
        "type": "create:pressing",
        "ingredients": [{"item": "create:pulp"}],
        "results": [{"id": "create:cardboard"}],
    },
    "pressing/path": {
        "type": "create:pressing",
        "ingredients": [[{"item": "minecraft:dirt"}, {"item": "minecraft:coarse_dirt"},
                         {"item": "minecraft:rooted_dirt"}, {"item": "minecraft:mycelium"},
                         {"item": "minecraft:podzol"}]],
        "results": [{"id": "minecraft:dirt_path"}],
    },
    "pressing/path_from_grass": {
        "type": "create:pressing",
        "ingredients": [{"item": "minecraft:grass_block"}],
        "results": [{"id": "minecraft:dirt_path"}],
    },
    "pressing/sugar_cane": {
        "type": "create:pressing",
        "ingredients": [{"item": "minecraft:sugar_cane"}],
        "results": [{"id": "minecraft:paper"}],
    },
}
# The convention tags those recipes name, with what NeoForge puts in them.
C_ITEM_TAGS = {
    "ingots/iron": ["minecraft:iron_ingot"],
    "ingots/gold": ["minecraft:gold_ingot"],
    "ingots/copper": ["minecraft:copper_ingot"],
    "nuggets/iron": ["minecraft:iron_nugget"],
    "storage_blocks/iron": ["minecraft:iron_block"],
}


def item_model(name):
    model = {"type": "minecraft:model", "model": f"create:item/{name}"}
    if name == "wrench":
        return {"type": "minecraft:composite", "models": [
            model, {"type": "minecraft:model", "model": "create:item/wrench/gear"},
        ]}
    return model


def loot_pools(name):
    if name != "belt":
        return [{
            "rolls": 1,
            "entries": [{"type": "minecraft:item", "name": f"create:{name}"}],
            "condition": {"type": "minecraft:survives_explosion"},
        }]
    # Create's `blocks/belt` drops a Belt Connector; `BeltBlock.getDrops` adds a Shaft for a
    # part with a pulley. The other blocks of the belt go without drops (`BeltBlock.onRemove`).
    return [{
        "rolls": 1,
        "entries": [{"type": "minecraft:item", "name": "create:belt_connector"}],
        "condition": {"type": "minecraft:survives_explosion"},
    }, {
        "rolls": 1,
        "entries": [{"type": "minecraft:item", "name": "create:shaft"}],
        "condition": {"type": "minecraft:all_of", "terms": [
            {"type": "minecraft:survives_explosion"},
            {"type": "minecraft:inverted", "term": {
                "type": "minecraft:block_state_property", "block": "create:belt",
                "properties": {"part": "middle"}}},
        ]},
    }]


# --- plugins: tools/content/<module>.py --------------------------------------------------------

PLUGINS = Path(__file__).resolve().parent / "content"
BASE_TAB = ["creative_motor", "shaft", "cogwheel", "large_cogwheel", "gearbox", "hand_crank",
            "water_wheel", "depot", "mechanical_press", "belt_connector", "wrench",
            "andesite_alloy", "iron_sheet"]
# What a plugin may define; see tools/content/README.md.
PLUGIN_KEYS = {"BLOCKS", "ITEMS", "NAMES", "RECIPES", "TAGS", "TAB", "PALETTES_TAB",
               "PICKAXE_ONLY", "MINEABLE", "LOOT", "MODELS", "BLOCKSTATES", "ITEM_MODELS", "FILES"}


def others_wip(owner):
    """The module names tools/wip.txt gives to owners other than `owner` (none for no owner)."""
    wip = Path(__file__).resolve().parent / "wip.txt"
    names = set()
    if owner in (None, "all") or not wip.is_file():
        return names
    for line in wip.read_text(encoding="utf-8").splitlines():
        if not line.strip() or line.startswith("#") or ":" not in line:
            continue
        who, _, modules = line.partition(":")
        if who.strip() != owner:
            names.update(modules.split())
    return names


def collect(owner=None):
    """Everything the mod declares: this file's own tables with every plugin's merged in.
    With an `owner`, the plugins other owners are still writing (tools/wip.txt) are left out."""
    import copy
    import importlib.util
    from types import SimpleNamespace

    c = SimpleNamespace(
        blocks=copy.deepcopy(BLOCKS), items=copy.deepcopy(ITEMS), names=dict(NAMES),
        recipes=copy.deepcopy(RECIPES), tab=list(BASE_TAB), palettes_tab=[],
        tags={f"c:item/{name}": list(values) for name, values in C_ITEM_TAGS.items()},
        pickaxe_only=set(PICKAXE_ONLY), mineable={}, loot={}, models=[], blockstates={},
        item_models={}, files={}, base_blocks=set(BLOCKS), source={})
    skip = others_wip(owner)
    for path in sorted(PLUGINS.glob("*.py")) if PLUGINS.is_dir() else []:
        stem = path.stem
        if stem.startswith("_") or any(stem == name or stem.startswith(name + "_") for name in skip):
            continue
        spec = importlib.util.spec_from_file_location(f"create_content_{stem}", path)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)

        def merge(target, key, kind):
            for name, value in getattr(module, key, {}).items():
                if name in target:
                    raise SystemExit(f"tools/content/{stem}.py: {kind} {name} is already declared"
                                     f" by {c.source.get((kind, name), 'gen_content.py')}")
                target[name] = value
                c.source[(kind, name)] = f"tools/content/{stem}.py"

        merge(c.blocks, "BLOCKS", "block")
        merge(c.items, "ITEMS", "item")
        merge(c.names, "NAMES", "name")
        merge(c.recipes, "RECIPES", "recipe")
        merge(c.loot, "LOOT", "loot table")
        merge(c.mineable, "MINEABLE", "mineable entry")
        merge(c.blockstates, "BLOCKSTATES", "blockstate")
        merge(c.item_models, "ITEM_MODELS", "item model")
        merge(c.files, "FILES", "file")
        for tag, values in getattr(module, "TAGS", {}).items():
            have = c.tags.setdefault(tag, [])
            have.extend(value for value in values if value not in have)
        c.tab.extend(name for name in getattr(module, "TAB", []) if name not in c.tab)
        c.palettes_tab.extend(name for name in getattr(module, "PALETTES_TAB", [])
                              if name not in c.palettes_tab)
        c.pickaxe_only.update(getattr(module, "PICKAXE_ONLY", ()))
        c.models.extend(getattr(module, "MODELS", []))
    # The wrench is the right tool for every block of the mod that needs one.
    c.items["wrench"]["components"]["minecraft:tool"]["rules"][0]["blocks"] = [
        f"create:{name}" for name, value in c.blocks.items()
        if value.get("properties", {}).get("requires_tool")]
    return c


def full_id(name):
    return name if ":" in name else f"create:{name}"


def generate(root=ROOT, owner=None):
    """Writes the declarations under `root` and answers what was collected."""
    c = collect(owner)
    assets, data = root / "assets" / "create", root / "data" / "create"
    for name, value in c.blocks.items():
        write(data / "horizon" / "block" / f"{name}.json", value)
        pools = c.loot[name] if name in c.loot else loot_pools(name) if value.get("item", True) \
            or name == "belt" else None
        if pools is not None:
            write(data / "loot_table" / "blocks" / f"{name}.json", {
                "type": "minecraft:block",
                "pools": pools,
                "random_sequence": f"create:blocks/{name}",
            })
    for name, value in c.items.items():
        write(data / "horizon" / "item" / f"{name}.json", value)
    for name in [*(b for b, value in c.blocks.items() if value.get("item", True)), *c.items]:
        write(assets / "items" / f"{name}.json",
              c.item_models.get(name) or {"model": item_model(name)})
    for name, value in c.recipes.items():
        write(data / "recipe" / f"{name}.json", value)
    tabs = [("kinetics", "itemGroup.create.base", "create:cogwheel", c.tab)]
    if c.palettes_tab:
        # `AllCreativeModeTabs.PALETTES_CREATIVE_TAB`: the Ornate Iron Window, once declared.
        icon = "ornate_iron_window" if "ornate_iron_window" in c.blocks else c.palettes_tab[0]
        tabs.append(("palettes", "itemGroup.create.palettes", full_id(icon), c.palettes_tab))
    for file, title, icon, items in tabs:
        write(data / "horizon" / "creative_tab" / f"{file}.json", {
            "title": title, "icon": icon, "items": [full_id(name) for name in items]})
    tags = {key: list(values) for key, values in c.tags.items()}
    for name in c.blocks:
        tools = c.mineable.get(name, ["pickaxe"] if name in c.pickaxe_only else ["pickaxe", "axe"])
        for tool in tools:
            have = tags.setdefault(f"minecraft:block/mineable/{tool}", [])
            if f"create:{name}" not in have:
                have.append(f"create:{name}")
    for key, values in tags.items():
        namespace, _, path = key.partition(":")
        write(root / "data" / namespace / "tags" / f"{path}.json", {"values": values})
    for path, value in c.files.items():
        if isinstance(value, str):
            (root / path).parent.mkdir(parents=True, exist_ok=True)
            (root / path).write_text(value, encoding="utf-8", newline="\n")
        else:
            write(root / path, value)
    write(assets / "lang" / "en_us.json", c.names)
    return c


def main():
    generate()


if __name__ == "__main__":
    main()
