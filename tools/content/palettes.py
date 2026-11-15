"""Create's building blocks (the `palettes` module): declarations only, no behaviour code.

Every family is generated from Create's own enumerations, and every number names its source:
AllBlocks.java (from `PALETTES_CREATIVE_TAB` on), content/decoration/palettes/AllPaletteBlocks.java,
AllPaletteStoneTypes.java, PaletteBlockPattern.java, PaletteBlockPartial.java,
PalettesVariantEntry.java, foundation/block/CopperBlockSet.java, foundation/data/BuilderTransformers.java,
MetalBarsGen.java, WindowGen.java, SharedProperties.java, and the recipes of src/generated/resources.

Known differences from Create (for the README):
- Sliding doors swing. Andesite, brass, copper, train and framed glass doors sit on vanilla's waxed
  copper door (a `DoorBlock` that opens by hand and by redstone), so they hinge like a vanilla door.
  The door and trapdoor open and close sounds are the copper ones, not Create's iron/stone ones.
- Connected textures (framed glass, windows, pillars, layered stone, copper roofs, scaffolding
  frames) are drawn by Create's Java at run time: here every block shows its base texture.
- Copper shingles and tiles never weather and cannot be waxed with honeycomb or scraped with an
  axe: Horizon's weathering and waxing read vanilla block names. The four stages and the waxed
  forms exist as blocks and recipes. The stage blocks sit on the unwaxed vanilla copper templates
  so a native hook can advance them later; until then they stay as placed.
- Metal scaffolding is a plain climbable block with an empty collision box (you walk into it and
  climb with the climbable tag), not vanilla's `ScaffoldingBlock`: Horizon's scaffolding rules count
  only vanilla scaffolding for stability, so a Create scaffold would fall where it meets its own
  kind. It is never "bottom" (always the full model), outline is a full block, and it does not
  have waterlogging, a standable top or Create's stacking item.
- Metal ladders follow vanilla's ladder (Create lets one hang from another ladder above it) and
  have no placement-assist extension.
- Wrench pick-up, the `quark:non_double_door` tag and Create's biome stone veins (the custom
  `create:layered_ore` feature) are not here.
- Windows, glass and ladders use vanilla's numbers copied from Horizon's prepared data.
"""

# --- vanilla's MapColor ids (net.minecraft.world.level.material.MapColor), as Horizon takes them ---
NONE, SAND, DIRT, STONE, WOOD, COLOR_ORANGE, COLOR_GRAY, COLOR_BLUE, COLOR_BROWN = 0, 2, 10, 11, 13, 15, 21, 25, 26
COLOR_RED, QUARTZ, PODZOL, TERRACOTTA_WHITE, TERRACOTTA_YELLOW, TERRACOTTA_PINK = 28, 14, 34, 36, 40, 42
TERRACOTTA_GRAY, TERRACOTTA_LIGHT_GRAY, TERRACOTTA_CYAN, TERRACOTTA_BROWN = 43, 44, 45, 48
COLOR_LIGHT_GRAY, CRIMSON_STEM, WARPED_STEM, WARPED_NYLIUM, DEEPSLATE_COLOR = 22, 53, 56, 55, 59
YELLOW = 18

# `SoundType`s as blocks that have them (a block's `sounds` names a block).
S_COPPER, S_IRON, S_METAL, S_NETHERITE = ("minecraft:copper_block", "minecraft:iron_block",
                                          "minecraft:gold_block", "minecraft:netherite_block")
S_DEEPSLATE, S_STONE = "minecraft:deepslate", "minecraft:stone"

BLOCKS, ITEMS, NAMES, RECIPES, LOOT, TAGS = {}, {}, {}, {}, {}, {}
MINEABLE, BLOCKSTATES = {}, {}
PICKAXE_ONLY = set()
PALETTES_TAB = []


def title(name):
    """Registrate's default English name."""
    return " ".join(word.capitalize() for word in name.split("_"))


def non_plural(name):
    """`Lang.nonPluralId`."""
    return name[:-1] if name.endswith("s") else name


def tag(key, *values):
    have = TAGS.setdefault(key, [])
    have.extend(v for v in values if v not in have)


def tool_props(hardness, resistance, requires_tool, sounds, color):
    return {"hardness": hardness, "explosion_resistance": resistance,
            "requires_tool": requires_tool, "sounds": sounds, "map_color": color}


def add(name, template, props=None, *, pickaxe=True, lang=None, **extra):
    """Declares block `name` on `template`, in Create's registration order (= the tab's order)."""
    decl = {"template": template, "item": True}
    if props:
        decl["properties"] = dict(props)
    decl.update(extra)
    BLOCKS[name] = decl
    NAMES[f"block.create.{name}"] = lang or title(name)
    PALETTES_TAB.append(name)
    if pickaxe:
        PICKAXE_ONLY.add(name)
    else:
        MINEABLE[name] = []
    return name


# --- recipes, in Horizon's 26.3 form ----------------------------------------------------------

def tg(x):
    """An ingredient: an item id, or a `#tag`."""
    return x


def stonecut(result, ingredient, count=1, key=None):
    """Registrate's `stonecutting`: the file is `<result>_from_<ingredient>_stonecutting`."""
    if key is None:
        label = ingredient.removeprefix("#").split(":", 1)[-1].replace("/", "_")
        key = f"{result}_from_{label}_stonecutting"
    RECIPES[key] = {"type": "minecraft:stonecutting", "ingredient": ingredient,
                    "result": {"id": f"create:{result}", "count": count} if count != 1
                    else {"id": f"create:{result}"}}


def shaped(key, result, pattern, ingredients, count=1, category="building", group=None):
    recipe = {"type": "minecraft:crafting_shaped", "category": category}
    if group:
        recipe["group"] = group
    recipe["key"] = ingredients
    recipe["pattern"] = pattern
    recipe["result"] = {"id": result, "count": count} if count != 1 else {"id": result}
    RECIPES[key] = recipe


def shapeless(key, result, ingredients, count=1, category="misc"):
    RECIPES[key] = {"type": "minecraft:crafting_shapeless", "category": category,
                    "ingredients": ingredients,
                    "result": {"id": result, "count": count} if count != 1 else {"id": result}}


# --- loot tables (the forms vanilla's own tables have in Horizon's prepared data) ----------------

def slab_loot(block):
    # `BlockLootSubProvider.createSlabItemTable`.
    return [{"rolls": 1, "entries": [{
        "type": "minecraft:item", "name": f"create:{block}",
        "modifier": [
            {"type": "minecraft:set_count", "count": 2, "condition": {
                "type": "minecraft:match_block", "blocks": f"create:{block}",
                "state": {"type": "double"}}},
            {"type": "minecraft:explosion_decay"}]}]}]


def door_loot(block):
    # `BlockLootSubProvider.createDoorTable` (`SlidingDoorBlock` registers `lr.createDoorTable`).
    return [{"rolls": 1, "condition": {"type": "minecraft:survives_explosion"}, "entries": [{
        "type": "minecraft:item", "name": f"create:{block}",
        "condition": {"type": "minecraft:match_block", "blocks": f"create:{block}",
                      "state": {"half": "lower"}}}]}]


def silk_loot(block):
    # `dropWhenSilkTouch`.
    return [{"rolls": 1, "condition": "minecraft:tool/can_silk_touch",
             "entries": [{"type": "minecraft:item", "name": f"create:{block}"}]}]


# ======================================================================================
# Group 1: AllBlocks.java, the section after `PALETTES_CREATIVE_TAB`
# ======================================================================================

# Create's convention tags that its recipes name, with what NeoForge puts in them.
tag("c:item/ingots/copper", "minecraft:copper_ingot")
tag("c:item/ingots/brass", "create:brass_ingot")
tag("c:item/glass_blocks/colorless", "minecraft:glass")
tag("c:block/glass_blocks/colorless", "minecraft:glass")
tag("c:item/glass_panes/colorless", "minecraft:glass_pane")
tag("c:block/glass_panes/colorless", "minecraft:glass_pane")
tag("c:item/glass_panes", "minecraft:glass_pane")
tag("c:block/glass_panes", "minecraft:glass_pane")
tag("c:item/dyes/black", "minecraft:black_dye")

# BuilderTransformers.ladder: initialProperties(Blocks.LADDER), mapColor, SoundType.COPPER, pickaxeOnly,
# BlockTags.CLIMBABLE; stonecutting x2 from the ingredient. Blocks.LADDER: strength 0.4.
METALS = [  # (name, ingredient, MapColor)
    ("andesite", "create:andesite_alloy", STONE),            # AllBlocks.ANDESITE_LADDER ...
    ("brass", "#c:ingots/brass", TERRACOTTA_YELLOW),         # MapColor.TERRACOTTA_YELLOW
    ("copper", "#c:ingots/copper", COLOR_ORANGE),            # MapColor.COLOR_ORANGE
]
for metal, ingredient, color in METALS:
    add(f"{metal}_ladder", "minecraft:ladder", tool_props(0.4, 0.4, False, S_COPPER, color))
    tag("minecraft:block/climbable", f"create:{metal}_ladder")
    stonecut(f"{metal}_ladder", ingredient, 2)

# MetalBarsGen.createBars: initialProperties(Blocks.IRON_BARS) (5.0 / 6.0, needs a tool),
# SoundType.COPPER, tags create:wrench_pickup and create:fan_transparent, pickaxeOnly, stonecutting x4.
for metal, ingredient, color in METALS:
    add(f"{metal}_bars", "minecraft:iron_bars", tool_props(5.0, 6.0, True, S_COPPER, color))
    tag("create:block/wrench_pickup", f"create:{metal}_bars")
    tag("create:block/fan_transparent", f"create:{metal}_bars")
    stonecut(f"{metal}_bars", ingredient, 4)

# BuilderTransformers.scaffold: initialProperties(Blocks.SCAFFOLDING) (strength 0), SoundType.COPPER,
# BlockTags.CLIMBABLE, pickaxeOnly, stonecutting x2. `AllShapes.SCAFFOLD_FULL`/`MetalScaffoldingBlock`
# lets players pass through; here a plain block on the glass template stands in for it (see header).
for metal, ingredient, color in METALS:
    add(f"{metal}_scaffolding", "minecraft:glass", tool_props(0.0, 0.0, False, S_COPPER, color),
        shapes={"default": [[0, 0, 0, 1, 1, 1]]}, collision_shapes={"default": []})
    tag("minecraft:block/climbable", f"create:{metal}_scaffolding")
    stonecut(f"{metal}_scaffolding", ingredient, 2)

# BuilderTransformers.slidingDoor: initialProperties(Blocks.IRON_DOOR), requiresCorrectToolForDrops,
# strength(3.0F, 6.0F), noOcclusion, pickaxeOnly, tags doors + wooden_doors (villager AI), createDoorTable.
# `SlidingDoorBlock` extends DoorBlock and opens by hand (`BlockSetType` create:stone/train/glass all
# `canOpenByHand`); vanilla's waxed copper door is the DoorBlock template that opens by hand too.
DOOR_COLORS = [("andesite", STONE, "create:andesite_casing"),                # SlidingDoorBlock.stone
               ("brass", TERRACOTTA_YELLOW, "create:brass_casing"),
               ("copper", COLOR_ORANGE, "create:copper_casing"),
               ("train", TERRACOTTA_CYAN, "create:railway_casing"),         # SlidingDoorBlock.metal
               ("framed_glass", NONE, "create:framed_glass")]              # SlidingDoorBlock.glass


def door_blockstate(state):
    """Create's door has a `visible` state (shown by its block entity); vanilla's door has none."""
    return {"variants": {key.removesuffix(",visible=true"): value
                         for key, value in state["variants"].items() if not key.endswith("visible=false")}}


for kind, color, casing in DOOR_COLORS:
    door = f"{kind}_door"
    add(door, "minecraft:waxed_copper_door", tool_props(3.0, 6.0, True, S_IRON, color))
    BLOCKSTATES[door] = door_blockstate
    LOOT[door] = door_loot(door)
    tag("minecraft:block/doors", f"create:{door}")
    tag("minecraft:block/wooden_doors", f"create:{door}")
    tag("minecraft:item/doors", f"create:{door}")
    tag("create:item/contraption_controlled", f"create:{door}")
    shapeless(f"crafting/kinetics/{door}", f"create:{door}", ["#minecraft:wooden_doors", casing])
    if kind == "train":  # TrainTrapdoorBlock.metal comes right after TRAIN_DOOR in AllBlocks
        add("train_trapdoor", "minecraft:waxed_copper_trapdoor",
            tool_props(3.0, 6.0, True, S_METAL, TERRACOTTA_CYAN))  # SharedProperties.softMetal = gold block
        shapeless("crafting/kinetics/train_trapdoor", "create:train_trapdoor",
                  ["#minecraft:wooden_trapdoors", "create:railway_casing"])
    if kind == "framed_glass":  # TrainTrapdoorBlock.glass: softMetal, mapColor NONE, noOcclusion
        add("framed_glass_trapdoor", "minecraft:waxed_copper_trapdoor",
            {**tool_props(3.0, 6.0, True, S_METAL, NONE), "occludes": False})
        shapeless("crafting/kinetics/framed_glass_trapdoor", "create:framed_glass_trapdoor",
                  ["#minecraft:wooden_trapdoors", "create:framed_glass"])
# BuilderTransformers.trapdoor: pickaxeOnly, BlockTags.TRAPDOORS (block and item)
for trapdoor in ("train_trapdoor", "framed_glass_trapdoor"):
    tag("minecraft:block/trapdoors", f"create:{trapdoor}")
    tag("minecraft:item/trapdoors", f"create:{trapdoor}")

# (zinc ore, raw zinc and the storage blocks are declared by the materials module.)

# BuilderTransformers.palettesIronBlock: softMetal (gold block: 3.0 / 6.0), COLOR_GRAY,
# SoundType.NETHERITE_BLOCK, requiresCorrectToolForDrops, pickaxeOnly, create:wrench_pickup,
# stonecutting x2 from c:ingots/iron.
for iron, name in (("industrial_iron_block", "Block of Industrial Iron"),
                   ("weathered_iron_block", "Block of Weathered Iron")):
    add(iron, "minecraft:gold_block", tool_props(3.0, 6.0, True, S_NETHERITE, COLOR_GRAY), lang=name)
    tag("create:block/wrench_pickup", f"create:{iron}")
    stonecut(iron, "#c:ingots/iron", 2)

# AllBlocks.ROSE_QUARTZ_BLOCK: RotatedPillarBlock, Blocks.AMETHYST_BLOCK (1.5 / 1.5), TERRACOTTA_PINK,
# requiresCorrectToolForDrops, SoundType.DEEPSLATE; stonecutting x2 from create:rose_quartz.
add("rose_quartz_block", "minecraft:quartz_pillar",
    tool_props(1.5, 1.5, True, S_DEEPSLATE, TERRACOTTA_PINK), lang="Block of Rose Quartz")
stonecut("rose_quartz_block", "create:rose_quartz", 2)
# ROSE_QUARTZ_TILES / SMALL_ROSE_QUARTZ_TILES: Blocks.DEEPSLATE (3.0 / 6.0), TERRACOTTA_PINK,
# requiresCorrectToolForDrops; stonecutting x2 from create:polished_rose_quartz, and the
# crafting/materials/*_from_conversion recipes turn one into the other.
for tiles, other in (("rose_quartz_tiles", "small_rose_quartz_tiles"),
                     ("small_rose_quartz_tiles", "rose_quartz_tiles")):
    add(tiles, "minecraft:cobbled_deepslate", tool_props(3.0, 6.0, True, S_DEEPSLATE, TERRACOTTA_PINK))
    stonecut(tiles, "create:polished_rose_quartz", 2)
    shapeless(f"crafting/materials/{tiles}_from_conversion", f"create:{tiles}",
              [f"create:{other}"], category="misc")

# CopperBlockSet: for waxed in (False, True): for variant in (block, slab, stairs): for the four
# WeatherStates. Base properties are `Blocks.COPPER_BLOCK` / EXPOSED_COPPER / WEATHERED_COPPER /
# OXIDIZED_COPPER (3.0 / 6.0, needs a tool, SoundType.COPPER); `pickaxeOnly`, `needs_stone_tool`.
STATES = [("", COLOR_ORANGE, "copper_block", "cut_copper"),
          ("exposed_", TERRACOTTA_LIGHT_GRAY, "exposed_copper", "exposed_cut_copper"),
          ("weathered_", WARPED_STEM, "weathered_copper", "weathered_cut_copper"),
          ("oxidized_", WARPED_NYLIUM, "oxidized_copper", "oxidized_cut_copper")]


def copper_set(name):
    """`COPPER_SHINGLES` / `COPPER_TILES`: stonecutting x2 from c:ingots/copper for the plain block."""
    for waxed in (False, True):
        for suffix in ("", "_slab", "_stairs"):
            for prefix, color, block_template, cut_template in STATES:
                full = ("waxed_" if waxed else "") + prefix + name
                if suffix:
                    full = non_plural(full) + suffix
                if suffix == "":
                    template = ("waxed_" if waxed else "") + block_template
                    if waxed and block_template == "copper_block":
                        template = "waxed_copper_block"
                else:
                    template = ("waxed_" if waxed else "") + cut_template + suffix
                add(full, f"minecraft:{template}", tool_props(3.0, 6.0, True, S_COPPER, color))
                tag("minecraft:block/needs_stone_tool", f"create:{full}")
                base = ("waxed_" if waxed else "") + prefix + name  # the block variant of this state
                if suffix == "_slab":
                    LOOT[full] = slab_loot(full)
                    tag("minecraft:block/slabs", f"create:{full}")
                    tag("minecraft:item/slabs", f"create:{full}")
                    shaped(full, f"create:{full}", ["XXX"], {"X": f"create:{base}"}, 6)
                    stonecut(full, f"create:{base}", 2)
                if suffix == "_stairs":
                    tag("minecraft:block/stairs", f"create:{full}")
                    tag("minecraft:item/stairs", f"create:{full}")
                    shaped(full, f"create:{full}", ["X  ", "XX ", "XXX"], {"X": f"create:{base}"}, 4)
                    stonecut(full, f"create:{base}", 1)
                if suffix == "" and not waxed and not prefix:
                    stonecut(full, "#c:ingots/copper", 2)
                if waxed:
                    unwaxed = full.removeprefix("waxed_")
                    shapeless(f"crafting/copper/{full}_from_honeycomb", f"create:{full}",
                              [f"create:{unwaxed}", "minecraft:honeycomb"], category="building")


copper_set("copper_shingles")
copper_set("copper_tiles")

# ======================================================================================
# Group 2: AllPaletteBlocks.java and WindowGen.java
# ======================================================================================

# WindowGen.framedGlass: Blocks.GLASS, silk-touch drop, impermeable, c:glass_blocks/colorless,
# stonecutting from c:glass_blocks/colorless. TILED_GLASS has the same recipe and tags.
GLASSES = ["tiled_glass", "framed_glass", "horizontal_framed_glass", "vertical_framed_glass"]
for glass in GLASSES:
    add(glass, "minecraft:glass", pickaxe=False)
    LOOT[glass] = silk_loot(glass)
    tag("minecraft:block/impermeable", f"create:{glass}")
    tag("c:block/glass_blocks/colorless", f"create:{glass}")
    tag("c:item/glass_blocks/colorless", f"create:{glass}")
    stonecut(glass, "#c:glass_blocks/colorless", 1, key=f"{glass}_from_glass_blocks_colorless_stonecutting")


def pane(name, parent, color, colorless):
    """`WindowGen.glassPane`: Blocks.GLASS_PANE with the parent's map colour, silk-touch drop, 16 from
    the parent, c:glass_panes (and the colorless tag for the colourless ones)."""
    props = {"map_color": color} if color else None
    add(name, "minecraft:glass_pane", props, pickaxe=False)
    LOOT[name] = silk_loot(name)
    shaped(name, f"create:{name}", ["###", "###"], {"#": f"create:{parent}"}, 16)
    tag("c:block/glass_panes", f"create:{name}")
    tag("c:item/glass_panes", f"create:{name}")
    if colorless:
        tag("c:block/glass_panes/colorless", f"create:{name}")
        tag("c:item/glass_panes/colorless", f"create:{name}")
        stonecut(name, "#c:glass_panes/colorless", 1)


pane("tiled_glass_pane", "tiled_glass", NONE, True)
for glass in GLASSES[1:]:
    pane(f"{glass}_pane", glass, NONE, True)

# WindowGen.windowBlock: Blocks.GLASS with the ingredient's map colour, silk-touch drop, impermeable;
# recipe " # " / "#X#" with X c:glass_blocks/colorless, x2.
# (wood, planks map colour from `Blocks.<WOOD>_PLANKS`.defaultMapColor)
WOODS = [("oak", WOOD), ("spruce", PODZOL), ("birch", SAND), ("jungle", DIRT), ("acacia", COLOR_ORANGE),
         ("dark_oak", COLOR_BROWN), ("mangrove", COLOR_RED), ("crimson", CRIMSON_STEM),
         ("warped", WARPED_STEM), ("cherry", TERRACOTTA_WHITE), ("bamboo", YELLOW)]
WINDOWS = [(f"{wood}_window", f"minecraft:{wood}_planks", color) for wood, color in WOODS] + [
    ("ornate_iron_window", "minecraft:iron_nugget", TERRACOTTA_LIGHT_GRAY),
    ("industrial_iron_window", "create:industrial_iron_block", COLOR_GRAY),
    ("weathered_iron_window", "create:weathered_iron_block", TERRACOTTA_LIGHT_GRAY)]
for window, ingredient, color in WINDOWS:
    add(window, "minecraft:glass", {"map_color": color}, pickaxe=False)
    LOOT[window] = silk_loot(window)
    tag("minecraft:block/impermeable", f"create:{window}")
    shaped(window, f"create:{window}", [" # ", "#X#"],
           {"#": ingredient, "X": "#c:glass_blocks/colorless"}, 2)
for window, ingredient, color in WINDOWS:
    pane(f"{window}_pane", window, color, False)

# ======================================================================================
# Group 3: AllPaletteStoneTypes.java, PaletteBlockPattern.java, PaletteBlockPartial.java
# ======================================================================================

# (id, base block's props: hardness, explosion resistance, sounds, MapColor, cube template, family)
# VANILLA_RANGE types copy `Blocks.<X>`; STANDARD_RANGE types copy the block named in
# `paletteStoneBlock(..., propertiesFrom, ...)` and then change `destroyTime` and `mapColor`.
# `family` is the vanilla stairs/slab/wall set used as a template (class only; numbers are explicit).
STONE_TYPES = [
    ("granite", (1.5, 6.0, "minecraft:granite", DIRT), "minecraft:granite", "granite"),
    ("diorite", (1.5, 6.0, "minecraft:diorite", QUARTZ), "minecraft:diorite", "diorite"),
    ("andesite", (1.5, 6.0, "minecraft:andesite", STONE), "minecraft:andesite", "andesite"),
    ("calcite", (0.75, 0.75, "minecraft:calcite", TERRACOTTA_WHITE), "minecraft:calcite", "cobblestone"),
    ("dripstone", (1.5, 1.0, "minecraft:dripstone_block", TERRACOTTA_BROWN),
     "minecraft:dripstone_block", "cobblestone"),
    ("deepslate", (3.0, 6.0, "minecraft:deepslate", DEEPSLATE_COLOR), "minecraft:cobbled_deepslate",
     "cobbled_deepslate"),
    ("tuff", (1.5, 6.0, "minecraft:tuff", TERRACOTTA_GRAY), "minecraft:tuff", "tuff"),
    ("asurine", (1.25, 6.0, "minecraft:deepslate", COLOR_BLUE), "minecraft:cobbled_deepslate",
     "cobbled_deepslate"),
    ("crimsite", (1.25, 6.0, "minecraft:deepslate", COLOR_RED), "minecraft:cobbled_deepslate",
     "cobbled_deepslate"),
    ("limestone", (1.25, 0.8, "minecraft:sandstone", SAND), "minecraft:sandstone", "sandstone"),
    ("ochrum", (1.25, 0.75, "minecraft:calcite", TERRACOTTA_YELLOW), "minecraft:calcite", "cobblestone"),
    ("scoria", (1.5, 6.0, "minecraft:blackstone", COLOR_BROWN), "minecraft:blackstone", "blackstone"),
    ("scorchia", (1.5, 6.0, "minecraft:blackstone", TERRACOTTA_GRAY), "minecraft:blackstone", "blackstone"),
    ("veridium", (1.25, 6.0, "minecraft:tuff", WARPED_NYLIUM), "minecraft:tuff", "tuff"),
]
NATURAL = {"asurine", "crimsite", "ochrum", "veridium"}  # hasNaturalVariants; the other new ones are plain
NEW_TYPES = {"asurine", "crimsite", "limestone", "ochrum", "scoria", "scorchia", "veridium"}
# PaletteBlockPattern: (id, name function, partials). PREFIX/WRAP/SUFFIX naming as `createName`.
PATTERNS = [
    ("cut", lambda v: f"cut_{v}", ("stairs", "slab", "wall")),
    ("polished", lambda v: f"polished_cut_{v}", ("stairs", "unique_slab", "wall")),
    ("bricks", lambda v: f"cut_{v}_bricks", ("stairs", "slab", "wall")),
    ("small_bricks", lambda v: f"small_{v}_bricks", ("stairs", "slab", "wall")),
    ("layered", lambda v: f"layered_{v}", ()),
    ("pillar", lambda v: f"{v}_pillar", ()),
]

for stone, (hardness, resistance, sound, color), cube, family in STONE_TYPES:
    material = f"#create:stone_types/{stone}"     # PalettesVariantEntry: the item tag the recipes read
    props = tool_props(hardness, resistance, True, sound, color)
    members = []                                    # what `stone_types/<id>` holds
    if stone in NEW_TYPES:
        # CreateRegistrate.paletteStoneBlock: pickaxeOnly, dripstone/azalea/moss/lush replaceable tags.
        add(stone, cube, props)
        for replaceable in ("dripstone_replaceable_blocks", "azalea_root_replaceable", "moss_replaceable",
                            "lush_ground_replaceable"):
            tag(f"minecraft:block/{replaceable}", f"create:{stone}")
        members.append(f"create:{stone}")
        base = f"create:{stone}"
    else:
        members.append(cube if cube != "minecraft:cobbled_deepslate" else "minecraft:deepslate")
        base = members[0]
    # PalettesVariantEntry: stonecutting from the material tag to the base block itself; a vanilla
    # base block's recipe is named after that block (`dripstone_block_from_stone_types_dripstone...`).
    base_path = base.split(":", 1)[1]
    stonecut(base_path, material, 1, key=f"{base_path}_from_stone_types_{stone}_stonecutting")
    if stone not in NEW_TYPES:
        RECIPES[f"{base_path}_from_stone_types_{stone}_stonecutting"]["result"] = {"id": base}
    for pattern, namer, partials in PATTERNS:
        block = namer(stone)
        template = "minecraft:quartz_pillar" if pattern == "pillar" else cube
        add(block, template, props)
        members.append(f"create:{block}")
        stonecut(block, material, 1, key=f"{block}_from_stone_types_{stone}_stonecutting")
        if pattern == "layered":
            pass
        for partial in partials:
            kind = "slab" if partial == "unique_slab" else partial
            name = f"{non_plural(block)}_{kind}"
            add(name, f"minecraft:{family}_{kind}", props)
            tag(f"minecraft:block/{kind}s", f"create:{name}")
            tag(f"minecraft:item/{kind}s", f"create:{name}")
            if kind == "stairs":   # PaletteBlockPartial.Stairs
                members.append(f"create:{name}")
                shaped(name, f"create:{name}", ["X  ", "XX ", "XXX"], {"X": f"create:{block}"}, 4, group=name)
                stonecut(name, material, 1, key=f"{name}_from_stone_types_{stone}_stonecutting")
            elif kind == "slab":   # PaletteBlockPartial.Slab: drops two when double, can't be recycled in the tag
                LOOT[name] = slab_loot(name)
                shaped(name, f"create:{name}", ["XXX"], {"X": f"create:{block}"}, 6, group=name)
                stonecut(name, material, 2, key=f"{name}_from_stone_types_{stone}_stonecutting")
                shapeless(f"{name}_recycling", f"create:{block}", [f"create:{name}", f"create:{name}"],
                          category="building")
            else:                  # PaletteBlockPartial.Wall
                members.append(f"create:{name}")
                stonecut(name, material, 1, key=f"{name}_from_stone_types_{stone}_stonecutting")
                shaped(name, f"create:{name}", ["XXX", "XXX"], {"X": f"create:{block}"}, 6)
    tag(f"create:item/stone_types/{stone}", *members)

# `crafting/palettes/scorchia`: eight scoria around black dye.
shaped("crafting/palettes/scorchia", "create:scorchia", ["###", "#D#", "###"],
       {"#": "create:scoria", "D": "#c:dyes/black"}, 8, category="misc")


# `smelting/scoria` (soul sand, vanilla's smelting form) and `smelting/glass_from_*`: the glass blocks
# and panes melt back into vanilla glass in 50 ticks. `splashing/*` is Create's own type, as its
# datagen writes it (read by the fan module).
RECIPES["smelting/scoria"] = {"type": "minecraft:smelting", "category": "blocks", "cookingtime": 200,
                              "experience": 0.0, "ingredient": "minecraft:soul_sand",
                              "result": {"id": "create:scoria"}}
for glass in GLASSES:
    RECIPES[f"smelting/glass_from_{glass}"] = {
        "type": "minecraft:smelting", "category": "blocks", "cookingtime": 50, "experience": 0.0,
        "ingredient": f"create:{glass}", "result": {"id": "minecraft:glass"}}
    RECIPES[f"smelting/glass_pane_from_{glass}_pane"] = {
        "type": "minecraft:smelting", "category": "blocks", "cookingtime": 50, "experience": 0.0,
        "ingredient": f"create:{glass}_pane", "result": {"id": "minecraft:glass_pane"}}
for source, result in (("industrial_iron_block", "weathered_iron_block"),
                       ("industrial_iron_window", "weathered_iron_window"),
                       ("industrial_iron_window_pane", "weathered_iron_window_pane")):
    RECIPES[f"splashing/{source}"] = {"type": "create:splashing",
                                      "ingredients": [{"item": f"create:{source}"}],
                                      "results": [{"id": f"create:{result}"}]}
