"""Translate Create 6.0.10 storyboards into data for the Rust scene player.

Regeneration takes the matching Java source and tree-sitter/tree-sitter-java.
Ordinary mod builds consume the checked-in instruction data and the existing
local Create JAR. Nothing runs Java code or embeds Create's resource files.
"""

import argparse
from collections import Counter
from itertools import product
import json
from pathlib import Path
import re


def walk(node):
    if node is None:
        return
    for child in node.named_children:
        yield from walk(child)
    yield node


def field(node, name):
    return node.child_by_field_name(name) if node is not None else None


def text(node):
    return node.text.decode("utf-8") if node is not None else ""


def invocation(node):
    return text(field(node, "name")), field(node, "object"), field(node, "arguments").named_children


def java_string(node):
    source = text(node)
    if node is not None and node.type == "string_literal":
        return json.loads(source)
    return None


COLORS = "white orange magenta light_blue yellow lime pink gray light_gray cyan purple blue brown green red black".split()
DYED = {
    "DYED_VALVE_HANDLES": "valve_handle",
    "PACKAGE_POSTBOXES": "postbox",
    "TABLE_CLOTHS": "table_cloth",
    "SEATS": "seat",
}


class Registry:
    def __init__(self, create):
        self.ids = {}
        for owner in ("AllBlocks", "AllItems"):
            source = (create / f"{owner}.java").read_text(encoding="utf-8")
            for part in re.split(r";", source):
                match = re.search(r'\b([A-Z][A-Z0-9_]+)\s*=\s*[\s\S]*?\.(?:block|item)\(\s*"([^"]+)"', part)
                if match:
                    self.ids[f"{owner}.{match[1]}"] = f"create:{match[2]}"

    def items(self, node):
        source = text(node)
        if source.startswith("TrackMaterial.allBlocks("):
            return ["create:track"]
        match = re.match(r"^(AllBlocks|AllItems|Blocks|Items)\.([A-Z][A-Z0-9_]+)", source)
        if not match:
            return []
        owner, name = match.groups()
        if owner in ("Blocks", "Items"):
            return [f"minecraft:{name.lower()}"]
        if name in DYED:
            color = re.search(r"DyeColor\.([A-Z_]+)", source)
            return [f"create:{value}_{DYED[name]}"
                    for value in ([color[1].lower()] if color else COLORS)]
        return [self.ids.get(match[0], f"create:{name.lower()}")]


def method_calls(node, name):
    return (child for child in walk(node)
            if child.type == "method_invocation" and text(field(child, "name")) == name)


def registrations(root, registry):
    rows = {}
    for call in method_calls(root, "addStoryBoard"):
        _, target, args = invocation(call)
        selected = next(method_calls(target, "forComponents"), None)
        if selected is None:
            selected_items, args = args[:1], args[1:]
        else:
            selected_items = invocation(selected)[2]
        path = java_string(args[0])
        reference = text(args[1])
        if not path or "::" not in reference:
            raise ValueError(f"Unsupported storyboard registration: {text(call)}")
        items = [item for node in selected_items for item in registry.items(node)]
        if not items:
            raise ValueError(f"Storyboard without components: {path}")
        row = rows.setdefault((path, reference), {"path": path, "items": [], "tags": []})
        row["items"] = list(dict.fromkeys(row["items"] + items))
        row["tags"] = list(dict.fromkeys(row["tags"] + [text(a).split(".")[-1] for a in args[2:]]))
    return rows


def tags(root, registry):
    rows = {}
    names = {}
    for node in walk(root):
        if node.type == "variable_declarator":
            value = field(node, "value")
            if value is not None and value.type == "method_invocation":
                name, _, args = invocation(value)
                if name == "loc" and args:
                    names[text(field(node, "name"))] = java_string(args[0])
        if node.type != "expression_statement":
            continue
        calls = [n for n in walk(node) if n.type == "method_invocation"]
        registration = next((n for n in calls if text(field(n, "name")) == "registerTag"), None)
        if registration:
            name = text(invocation(registration)[2][0])
            args_by_name = {invocation(n)[0]: invocation(n)[2] for n in calls}
            item_args = args_by_name["item"]
            item = registry.items(item_args[0])[0]
            rows[name] = {
                "id": names[name],
                "icon": item,
                "index": "addToIndex" in args_by_name,
                "items": [],
            }
            if len(item_args) == 1 or text(item_args[2]) == "true":
                rows[name]["main_item"] = item
    for node in walk(root):
        if node.type != "method_invocation":
            continue
        name, target, args = invocation(node)
        if name != "add" or target is None:
            continue
        parent = next(method_calls(target, "addToTag"), None)
        if parent is not None:
            tag = text(invocation(parent)[2][0])
            if tag in rows:
                rows[tag]["items"].extend(item for arg in args for item in registry.items(arg))
    for row in rows.values():
        row["items"] = list(dict.fromkeys(row["items"]))
    return rows


UNKNOWN = object()

# `FluidStack`'s first argument: Create's own fluids and the vanilla ones the storyboards use, as (name, id). A flowing
# fluid is its source here: the scene shows the fluid, not its level.
FLUIDS = {"AllFluids.CHOCOLATE": ("chocolate", "create:chocolate"), "AllFluids.HONEY": ("honey", "create:honey"),
          "NeoForgeMod.MILK": ("milk", "neoforge:milk"), "Fluids.WATER": ("water", "minecraft:water"), "Fluids.LAVA": ("lava", "minecraft:lava"),
          "Fluids.FLOWING_LAVA": ("lava", "minecraft:lava")}


def fluid_of(source):
    """The fluid a `new FluidStack(...)` expression names."""
    if (milk := re.search(r"\bNeoForgeMod\.MILK\b", source)):
        return FLUIDS["NeoForgeMod.MILK"]
    match = re.search(r"\b(AllFluids|Fluids)\.([A-Z_]+)", source)
    if not match:
        return None
    key = f"{match[1]}.{match[2]}"
    if key not in FLUIDS and match[1] == "AllFluids":
        return match[2].lower(), f"create:{match[2].lower()}"
    return FLUIDS.get(key)


# The Fluid Tank blocks the fluid instructions can act on (`FluidTankBlockEntity`, `CreativeFluidTankBlockEntity`).
TANKS = {"create:fluid_tank", "create:creative_fluid_tank"}
# The other blocks a scene fills and drains through their fluid capability (`rules::ponder_fluid::Vessel`).
VESSELS = {"create:item_drain", "create:spout", "create:basin"}
# `BrassTunnelBlockEntity.SelectionMode`, in order: read from the source when the storyboards are generated.
SELECTION_MODES = ["SPLIT", "FORCED_SPLIT", "ROUND_ROBIN", "FORCED_ROUND_ROBIN", "PREFER_NEAREST", "RANDOMIZE", "SYNCHRONIZE"]


def selection_modes(create):
    """The mode names in the enum's order, `SELECTION_MODES` when the source does not have them."""
    path = create / "content/logistics/tunnel/BrassTunnelBlockEntity.java"
    if path.exists():
        match = re.search(r"enum SelectionMode[^{]*\{(.*?);", path.read_text(encoding="utf-8"), re.S)
        if match:
            return re.findall(r"^\s*([A-Z_]+)\(", match[1], re.M) or SELECTION_MODES
    return SELECTION_MODES


# The block a scene's crafter instructions act on (`MechanicalCrafterBlockEntity`).
CRAFTER = "create:mechanical_crafter"


class Links(list):
    """An `ArrayList` a storyboard fills with the links `showIndependentSection` returns."""


class Storyboard:
    def __init__(self, registry, method, helpers, dimensions, blocks=None):
        self.registry = registry
        # The block id at each position of the scene's structure.
        self.blocks = blocks or {}
        self.tick = 0
        self.env = {}
        self.ops = []
        self.unhandled = Counter()
        self.inert = Counter()
        self.next_section = 1
        self.next_caption = 1
        self.key = None
        self.helpers = helpers
        self.depth = 0
        self.dimensions = dimensions
        self.next_link = 1
        self.breaking = {}
        self.bounds = [0, 0, max(dimensions[0], dimensions[2])]
        self.scale = 1
        self.offset = 0
        self.execute(field(method, "body"))

    def emit(self, op, *values):
        self.ops.append([self.tick, op, *values])

    def execute(self, node):
        if node is None:
            return
        if node.type in {"block", "constructor_body"}:
            for child in node.named_children:
                self.execute(child)
        elif node.type == "local_variable_declaration":
            for child in node.named_children:
                if child.type == "variable_declarator":
                    self.env[text(field(child, "name"))] = self.value(field(child, "value"))
        elif node.type == "expression_statement":
            self.value(node.named_children[0])
        elif node.type == "if_statement":
            condition = self.value(field(node, "condition"))
            if condition is not UNKNOWN:
                self.execute(field(node, "consequence" if condition else "alternative"))
            else:
                self.unhandled["if"] += 1
        elif node.type == "for_statement":
            init, condition = field(node, "init"), field(node, "condition")
            self.execute(init)
            for _ in range(64):
                check = self.value(condition)
                if check is UNKNOWN:
                    self.unhandled["for"] += 1
                    break
                if not check:
                    break
                self.execute(field(node, "body"))
                self.value(field(node, "update"))
        elif node.type == "enhanced_for_statement":
            values = self.value(field(node, "value"))
            if isinstance(values, list):
                name = text(field(node, "name"))
                before = self.env.get(name, UNKNOWN)
                for value in values:
                    self.env[name] = value
                    self.execute(field(node, "body"))
                self.env[name] = before
            else:
                self.unhandled[node.type] += 1
        elif node.type not in {"line_comment", "block_comment", "return_statement"}:
            self.unhandled[node.type] += 1

    def value(self, node):
        if node is None:
            return UNKNOWN
        source, kind = text(node), node.type
        if kind == "identifier":
            if source not in self.env and len(source) > 1 and source.isupper():
                # A statically imported constant (`SHAPE`, `LEVEL`): named as its field is in a property position.
                return source.lower()
            return self.env.get(source, UNKNOWN)
        if kind == "parenthesized_expression":
            return self.value(node.named_children[0])
        # `new BlockPos[]{a, b}` and `a[i]`: a list of what the initializer holds.
        if kind in {"array_creation_expression", "array_initializer"}:
            initializer = node if kind == "array_initializer" else field(node, "value")
            if initializer is not None:
                return [self.value(child) for child in initializer.named_children]
        if kind == "array_access":
            array, index = self.value(field(node, "array")), self.value(field(node, "index"))
            if isinstance(array, list) and isinstance(index, int) and not isinstance(index, bool) and 0 <= index < len(array):
                return array[index]
            return UNKNOWN
        if kind == "string_literal":
            return java_string(node)
        if kind in {"true", "false"}:
            return kind == "true"
        if kind in {"decimal_integer_literal", "decimal_floating_point_literal"}:
            return float(source.rstrip("fFdDlL")) if any(c in source for c in ".eEfFdD") else int(source.rstrip("lL"))
        if kind == "unary_expression":
            value = self.value(field(node, "operand"))
            if value is not UNKNOWN:
                return {"-": lambda: -value, "+": lambda: value, "!": lambda: not value}.get(
                    text(field(node, "operator")), lambda: UNKNOWN)()
        if kind == "binary_expression":
            left, right = (self.value(field(node, side)) for side in ("left", "right"))
            operator = text(field(node, "operator"))
            # `state.getValue(P) - 1`: the old value's offset, worked out where the old state is known.
            if isinstance(left, tuple) and left[:1] == ("old",) and operator in {"+", "-"} \
                    and isinstance(right, int) and not isinstance(right, bool):
                return ("old", left[1], left[2] + (right if operator == "+" else -right))
            if operator == "+" and UNKNOWN not in (left, right) and (isinstance(left, str) or isinstance(right, str)) \
                    and all(isinstance(v, (str, int, float)) and not isinstance(v, bool) for v in (left, right)):
                return java_text(left) + java_text(right)
            if UNKNOWN not in (left, right):
                operators = {
                    "+": lambda: left + right, "-": lambda: left - right,
                    "*": lambda: left * right, "/": lambda: int(left / right) if isinstance(left, int) and isinstance(right, int) else left / right,
                    "%": lambda: left % right, "<": lambda: left < right, "<=": lambda: left <= right,
                    ">": lambda: left > right, ">=": lambda: left >= right, "==": lambda: left == right,
                    "!=": lambda: left != right, "&&": lambda: left and right, "||": lambda: left or right,
                }
                try:
                    return operators.get(operator, lambda: UNKNOWN)()
                except (TypeError, ZeroDivisionError):
                    return UNKNOWN
        if kind == "ternary_expression":
            condition = self.value(field(node, "condition"))
            if condition is not UNKNOWN:
                return self.value(field(node, "consequence" if condition else "alternative"))
        if kind == "cast_expression":
            return self.value(field(node, "value"))
        if kind == "update_expression":
            name = text(node.named_children[0])
            if name in self.env and isinstance(self.env[name], (int, float)):
                self.env[name] += -1 if "--" in source else 1
                return self.env[name]
        if kind == "assignment_expression":
            name = text(field(node, "left"))
            value = self.value(field(node, "right"))
            if text(field(node, "operator")) == "=":
                self.env[name] = value
            return value
        if kind == "field_access":
            if source == "CommonComponents.EMPTY":
                return ("text", "")
            if ids := self.registry.items(node):
                return ("id", ids[0], {})
            prefix, _, field_name = source.rpartition(".")
            if prefix.split(".")[-1] in {"Direction", "Axis", "Vec3", "BlockPos"}:
                if field_name == "ZERO":
                    return (0, 0, 0)
                return field_name.lower()
            if source == "Iterate.horizontalDirections":
                return ["north", "east", "south", "west"]
            if source == "Iterate.directions":
                return list(DIRECTIONS)
            if field_name.isupper():
                return field_name.lower()
        if kind == "object_creation_expression":
            args = [self.value(a) for a in field(node, "arguments").named_children]
            name = text(field(node, "type"))
            if name == "CreateSceneBuilder":
                return ("scene",)
            if name.startswith("ArrayList"):
                return Links()
            if name in {"Vec3", "Vec3i", "BlockPos"} and numbers(args, 3):
                return tuple(args)
            if name in {"ItemParticleOption", "BlockParticleOption"}:
                return "item"
            if name == "FluidStack" and (fluid := fluid_of(text(node))):
                # (`fluid`, short name, amount in mB, id): the name is what the particle code reads.
                amount = args[1] if len(args) > 1 and isinstance(args[1], int) and not isinstance(args[1], bool) else 0
                return ("fluid", fluid[0], amount, fluid[1])
            if name == "ItemStack" and args and isinstance(args[0], tuple) and args[0][0] == "id":
                # `new ItemStack(item, count)`: the count is a fourth element, which a stack without one leaves out.
                if len(args) == 2 and isinstance(args[1], int) and not isinstance(args[1], bool) and args[1] != 1:
                    return (*args[0][:3], args[1])
                return args[0]
            if name == "AABB":
                if numbers(args, 6):
                    return ("aabb", tuple(args[:3]), tuple(args[3:]))
                if len(args) in {1, 2} and all(isinstance(v, tuple) and numbers(v, 3) for v in args):
                    end = args[1] if len(args) == 2 else tuple(v + 1 for v in args[0])
                    return ("aabb", tuple(map(min, args[0], end)), tuple(map(max, args[0], end)))
            return UNKNOWN
        if kind == "method_invocation":
            return self.call(*invocation(node))
        return UNKNOWN

    def call(self, name, target, args):
        target_text = text(target)
        value = self.value(target)
        values = [self.value(node) for node in args]
        if target_text == "scene" or value == ("scene",):
            if name in {"world", "overlay", "special", "effects"}:
                return ("service", name)
            if name == "title" and values and isinstance(values[0], str):
                self.key = values[0]
            elif name == "idle" and numbers(values, 1):
                self.tick += max(0, int(values[0]))
            elif name in {"addKeyframe", "addLazyKeyframe"}:
                self.emit("marker")
            elif name == "configureBasePlate" and numbers(values, 3):
                self.bounds = values
            elif name == "scaleSceneView" and numbers(values, 1):
                self.scale = values[0]
            elif name == "setSceneOffsetY" and numbers(values, 1):
                self.offset = values[0]
            elif name == "rotateCameraY" and numbers(values, 1):
                self.emit("yaw", values[0])
            elif name == "showBasePlate":
                x, z, size = self.bounds
                self.emit("show", 0, box((x, 0, z), (x + size - 1, 0, z + size - 1)), DIRECTIONS["up"])
            elif name not in {"world", "overlay", "special", "effects", "title", "markAsFinished", "setNextUpEnabled", "getCurrentTime"}:
                self.unhandled[f"scene.{name}"] += 1
            return UNKNOWN

        # `Component.literal(text)`, its `getString()` and `Component.Serializer.toJson(component, ...)`.
        if target_text == "Component" and name == "literal" and len(values) == 1 and isinstance(values[0], str):
            return ("text", values[0])
        if isinstance(value, tuple) and value[:1] == ("text",) and name == "getString" and not values:
            return value[1]
        if target_text == "Component.Serializer" and name == "toJson" and values \
                and isinstance(values[0], tuple) and values[0][:1] == ("text",):
            return ("json", values[0][1])
        # `FluidHelper.copyStackWithAmount(stack, amount)`.
        if target_text == "FluidHelper" and name == "copyStackWithAmount" and len(values) == 2 \
                and isinstance(values[0], tuple) and values[0][:1] == ("fluid",) and isinstance(values[1], int):
            return ("fluid", values[0][1], values[1], values[0][3])

        # The boxes `chaseBoundingBoxOutline` outlines: shapes the storyboard asks for by name and what `AABB` does to one.
        if name == "bounds" and not values:
            # `AllShapes.CASING_13PX.get(direction).bounds()`: the casing 13 pixels tall.
            if target_text.startswith("AllShapes.CASING_13PX.get("):
                return ("aabb", (0, 0, 0), (1, 13 / 16, 1))
            # A Shaft's shape (`SIX_VOXEL_POLE`): six pixels across the middle, the length of the block.
            if "AllBlocks.SHAFT.getDefaultState()" in target_text and (axis := re.search(r"ShaftBlock\.AXIS, ?Axis\.([XYZ])", target_text)):
                low, high = [0.375] * 3, [0.625] * 3
                low["xyz".index(axis[1].lower())], high["xyz".index(axis[1].lower())] = 0, 1
                return ("aabb", tuple(low), tuple(high))
        if isinstance(value, tuple) and value[:1] == ("aabb",) and name in AABB_METHODS:
            return aabb_method(value, name, values)
        # Text a board shows: `Component.literal`, `CreateLang.translateDirect(key)`, `stack.getHoverName()` and `append`
        # make a list of parts the player words from the language: `["lit", text]`, `["lang", key]`, `["name", item]`.
        if target_text == "CreateLang" and name == "translateDirect" and len(values) == 1 and isinstance(values[0], str):
            return ("lang", "create." + values[0])
        if isinstance(value, tuple) and value[:1] == ("id",) and name in {"getHoverName", "asStack"} and not values:
            return ("name", value[1]) if name == "getHoverName" else value
        if isinstance(value, tuple) and value[:1] in {("text",), ("lang",), ("name",), ("parts",)} and name == "append" \
                and len(values) == 1 and isinstance(values[0], tuple) and values[0][:1] in {("text",), ("lang",), ("name",), ("parts",)}:
            return ("parts", text_parts(value) + text_parts(values[0]))

        # `stack.copy()` is the stack; `stack.copyWithCount(n)` is the stack of `n`.
        if isinstance(value, tuple) and value[:1] == ("id",) and name in {"copy", "copyWithCount"}:
            if name == "copy" and not values:
                return value
            if name == "copyWithCount" and len(values) == 1 and isinstance(values[0], int) and not isinstance(values[0], bool):
                return (*value[:3], values[0])

        # `PackageItem.containing(...)` picks a random style (`PackageStyles.getRandomBox`): the first one stands for it.
        if target_text == "PackageItem" and name == "containing":
            return ("id", "create:cardboard_package_12x12", {})

        # `FluidFX.getFluidParticle(stack)`: Create's `FluidStackParticle`. `VecHelper.offsetRandomly(vec, random, radius)`
        # is `vec` plus up to `radius` either way on each axis: the player draws it, so only the radius is kept.
        if target_text == "FluidFX" and name == "getFluidParticle" and values and isinstance(values[0], tuple) and values[0][0] == "fluid":
            return ("particle", values[0][1])
        if target_text == "VecHelper" and name == "offsetRandomly" and len(values) == 3 and numbers(values[2:], 1):
            return ("jitter", values[2])

        if target_text == "util" and name in {"select", "grid", "vector"}:
            return ("service", name)

        if target is None and name in self.helpers and self.depth < 8:
            method = self.helpers[name]
            params = field(method, "parameters").named_children
            if len(params) == len(values):
                previous = self.env
                self.env = {text(field(param, "name")): v for param, v in zip(params, values)}
                self.depth += 1
                self.execute(field(method, "body"))
                self.depth -= 1
                self.env = previous
            return UNKNOWN

        if isinstance(value, tuple) and value[0] == "service":
            owner = value[1]
            match owner:
                case "select":
                    return self.select(name, values)
                case "grid" | "vector":
                    return vector(name, values)
                case "world":
                    return self.world(name, values, args)
                case "overlay":
                    if name in {"showText", "showOutlineWithText"}:
                        duration = next((v for v in values if isinstance(v, int)), None)
                        if duration is not None:
                            if name == "showOutlineWithText" and isinstance(values[0], list) and values[0]:
                                self.emit("outline", *selection_bounds(values[0]), "white", duration)
                            return ("caption", duration, {})
                    if name in {"showOutline", "chaseBoundingBoxOutline", "showLine", "showBigLine"} and len(values) == 4 and isinstance(values[3], int):
                        color, first, second, duration = values
                        color = color if isinstance(color, str) else "white"
                        if name == "showOutline" and isinstance(second, list) and second:
                            self.emit("outline", *selection_bounds(second), color, duration)
                            return UNKNOWN
                        if name == "chaseBoundingBoxOutline" and isinstance(second, tuple) and second[0] == "aabb":
                            self.emit("outline", *second[1:], color, duration)
                            return UNKNOWN
                        if name in {"showLine", "showBigLine"} and isinstance(first, tuple) and isinstance(second, tuple) and numbers(first, 3) and numbers(second, 3):
                            self.emit("line", first, second, color, duration)
                            return UNKNOWN
                    # `showFilterSlotInput`, `showScrollInput` and friends: `HighlightValueBoxInstruction`, a white
                    # outline box of the given size for the duration.
                    if name in HIGHLIGHTS and (box_ := highlight_box(name, values)):
                        self.emit("outline", *box_, "white", values[-1])
                        return UNKNOWN
                    if name == "showControls" and len(values) == 3 and isinstance(values[0], tuple):
                        self.emit("control", values[0], values[2] if isinstance(values[2], int) else 40)
                    else:
                        self.unhandled[f"overlay.{name}"] += 1
                case "effects":
                    # `CreateSceneBuilder.EffectInstructions.superGlue` -> `SuperGlueItem.spawnParticles`.
                    if name == "superGlue" and len(values) == 3 and numbers(values[0], 3) and isinstance(values[1], str)                             and values[1] in DIRECTIONS and isinstance(values[2], bool):
                        self.emit("glue", list(values[0]), list(DIRECTIONS[values[1]]), values[2])
                        return UNKNOWN
                    # `PonderEffectInstructions.simpleParticleEmitter` / `particleEmitterWithinBlockSpace`.
                    particle = values[0][1] if values and isinstance(values[0], tuple) and values[0][0] == "particle" else None
                    if name in {"simpleParticleEmitter", "particleEmitterWithinBlockSpace"} and values                             and (isinstance(values[0], str) or particle):
                        motion = values[1] if len(values) > 1 and isinstance(values[1], tuple) and numbers(values[1], 3) else (0, 0, 0)
                        jitter = values[1][1] if len(values) > 1 and isinstance(values[1], tuple) and values[1][0] == "jitter" else 0
                        return ("emitter", values[0] if isinstance(values[0], str) else particle, motion,
                                name == "particleEmitterWithinBlockSpace", jitter)
                    # `emitParticles(location, emitter, amountPerCycle, cycles)`.
                    if name == "emitParticles" and len(values) == 4 and numbers(values[0], 3)                             and isinstance(values[1], tuple) and values[1][0] == "emitter" and numbers(values[2:], 2):
                        self.emit("particles", values[1][1], list(values[0]), list(values[1][2]), values[1][3],
                                  values[2], values[3], *([values[1][4]] if values[1][4] else []))
                        return UNKNOWN
                    if name == "createRedstoneParticles" and len(values) == 3 and numbers(values[0], 3) and numbers(values[1:], 2):
                        self.emit("particles", "redstone", [v + 0.5 for v in values[0]], [0, 0, 0], True, values[2], 1)
                        return UNKNOWN
                    if name in {"indicateSuccess", "indicateRedstone", "rotationDirectionIndicator", "rotationSpeedIndicator"} and values:
                        if isinstance(values[0], tuple) and numbers(values[0], 3):
                            point = tuple(v + 0.5 for v in values[0])
                            self.emit("control", point, 15)
                            return UNKNOWN
                    self.unhandled[f"effects.{name}"] += 1
                case "special":
                    self.unhandled[f"{owner}.{name}"] += 1
            return UNKNOWN

        if isinstance(value, tuple) and value:
            if value[0] == "aabb":
                lo, hi = value[1:]
                if name in {"inflate", "deflate"} and (numbers(values, 1) or numbers(values, 3)):
                    sign = 1 if name == "inflate" else -1
                    sizes = [sign * v for v in (values * 3 if len(values) == 1 else values)]
                    return ("aabb", tuple(x - d for x, d in zip(lo, sizes)), tuple(x + d for x, d in zip(hi, sizes)))
                if name == "move" and len(values) == 1 and isinstance(values[0], tuple):
                    values = values[0]
                if numbers(values, 3):
                    if name == "move":
                        return ("aabb", tuple(a + b for a, b in zip(lo, values)), tuple(a + b for a, b in zip(hi, values)))
                    if name == "expandTowards":
                        return ("aabb", tuple(a + min(b, 0) for a, b in zip(lo, values)), tuple(a + max(b, 0) for a, b in zip(hi, values)))
                    if name == "contract":
                        return ("aabb", tuple(a - min(b, 0) for a, b in zip(lo, values)), tuple(a - max(b, 0) for a, b in zip(hi, values)))
                return UNKNOWN
            if value[0] == "caption":
                options = value[2]
                if name == "text":
                    key = f"text_{self.next_caption}"
                    self.next_caption += 1
                    self.emit("caption", key, value[1], options,
                              values[0] if values and isinstance(values[0], str) else None)
                elif name == "attachKeyFrame":
                    self.emit("marker")
                elif name == "pointAt" and len(values) == 1 and isinstance(values[0], tuple) and numbers(values[0], 3):
                    options["target"] = values[0]
                elif name == "placeNearTarget":
                    options["near"] = True
                elif name == "independent" and numbers(values, 1):
                    options["y"] = values[0]
                elif name == "colored" and len(values) == 1 and isinstance(values[0], str):
                    options["color"] = values[0]
                return value
            # `state.getValue(P).getOpposite()`.
            if value[0] == "old" and name == "getOpposite" and not values and value[2] == 0:
                return ("opposite", value[1])
            if value[0] == "id":
                if name in {"getDefaultState", "defaultBlockState", "get", "asStack", "asItem", "copy", "copyWithCount"}:
                    return value
                # `FluidPipeBlock.getAxisState(axis)`: open on both sides along the axis and shut on the others.
                if name == "getAxisState" and len(values) == 1 and values[0] in {"x", "y", "z"}:
                    axes = {"east": "x", "west": "x", "up": "y", "down": "y", "north": "z", "south": "z"}
                    return ("id", value[1], value[2] | {side: str(axis == values[0]).lower() for side, axis in axes.items()})
                # The state a `modifyBlock` lambda gets is the old one: `getValue` reads its property.
                if name == "getValue" and value[1] == "" and len(values) == 1 and isinstance(values[0], str):
                    return ("old", values[0], 0)
                # `setValue(P, old value, with an offset or turned round)`: worked out per block when the scene is built.
                if name == "setValue" and len(values) == 2 and isinstance(values[0], str) \
                        and isinstance(values[1], tuple) and values[1][:1] in {("old",), ("opposite",)}:
                    spec = {"from": values[1][1]}
                    if values[1][0] == "opposite":
                        spec["opposite"] = True
                    elif values[1][2]:
                        spec["add"] = values[1][2]
                    return ("id", value[1], value[2] | {values[0]: spec})
                if name == "setValue" and len(values) == 2 and all(isinstance(v, (str, bool, int)) for v in values):
                    return ("id", value[1], value[2] | {str(values[0]): str(values[1]).lower()})
                if name == "cycle" and len(values) == 1 and isinstance(values[0], str):
                    return ("id", value[1], value[2] | {values[0]: None})
            return vector(name, [value, *values])
        if isinstance(value, list) and not isinstance(value, Links) and name == "copy" and not values:
            return list(value)
        if isinstance(value, Links):
            if name == "add" and len(values) == 1:
                value.append(values[0])
            elif name == "get" and len(values) == 1 and isinstance(values[0], int) and 0 <= values[0] < len(value):
                return value[values[0]]
            elif name == "size":
                return len(value)
            return UNKNOWN
        if isinstance(value, list) and values and isinstance(values[0], list):
            if name == "add":
                return value + values[0]
            if name in {"substract", "subtract"}:
                return [p for p in value if p not in values[0]]
            if name == "intersect":
                return [p for p in value if p in values[0]]
        return UNKNOWN

    def lambda_body(self, node):
        """The lambda's first parameter name and its body."""
        parameters = field(node, "parameters")
        if parameters is None:
            return None, None
        if parameters.type != "identifier":
            parameters = parameters.named_children[0] if parameters.named_children else None
            if parameters is not None and parameters.type == "formal_parameter":
                parameters = field(parameters, "name")
        return (text(parameters) if parameters is not None else None), field(node, "body")

    def nbt_entries(self, node):
        """`{key: [type, value]}` for the `put*` calls in an `nbt -> ...` lambda, or None when one is not understood."""
        param, body = self.lambda_body(node)
        if param is None or body is None:
            return None
        entries = {}
        # Locals the lambda declares (`Component component = Component.literal("0F");`) are known to the puts below.
        declared = {}
        if body.type == "block":
            for statement in body.named_children:
                if statement.type == "local_variable_declaration":
                    declared.update((text(field(d, "name")), self.env.get(text(field(d, "name")), UNKNOWN))
                                    for d in statement.named_children if d.type == "variable_declarator")
                    self.execute(statement)
        try:
            return self.nbt_puts(body, param, entries)
        finally:
            for name, before in declared.items():
                self.env[name] = before

    def nbt_puts(self, body, param, entries):
        for call in method_calls_all(body):
            name, target, args = invocation(call)
            owner = text(target)
            if owner == param and name in NBT_TYPES and len(args) == 2:
                value = self.value(args[1])
                # `Component.Serializer.toJson(..)`: this port saves a tube's text as the plain string.
                if isinstance(value, tuple) and value[:1] == ("json",):
                    value = value[1]
                kind = NBT_TYPES[name]
                key = java_string(args[0])
                if kind == "b" and isinstance(value, bool):
                    entries[key] = ["b", value]
                elif kind in ("i", "f") and isinstance(value, (int, float)) and not isinstance(value, bool):
                    entries[key] = [kind, int(value) if kind == "i" else float(value)]
                elif kind == "s" and isinstance(value, str):
                    entries[key] = ["s", value]
                else:
                    return None
            elif owner == "NBTHelper" and name == "writeEnum" and len(args) == 3 and text(args[0]) == param:
                entries[java_string(args[1])] = ["s", text(args[2]).split(".")[-1]]
            # `put("HeldItem", new TransportedItemStack(stack).serializeNBT(..))`: a Depot holding the stack.
            elif owner == param and name == "put" and len(args) == 2 and java_string(args[0]) == "HeldItem" \
                    and text(args[1]).startswith("new TransportedItemStack("):
                made = field(args[1], "object")
                stack = self.value(field(made, "arguments").named_children[0]) if made is not None else UNKNOWN
                if not (isinstance(stack, tuple) and stack[0] == "id"):
                    return None
                entries["HeldItem"] = ["held", stack[1]]
            # `put("VisualizedItems", NBTHelper.writeCompoundList(List.of(IntAttached.with(n, stack)), ..))`: the stacks a
            # Basin is giving out (`BasinBlockEntity.visualizedOutputItems`).
            elif owner == param and name == "put" and len(args) == 2 and java_string(args[0]) == "VisualizedItems":
                stacks = []
                for attached in method_calls_all(args[1]):
                    attached_name, attached_target, attached_args = invocation(attached)
                    if text(attached_target) == "IntAttached" and attached_name == "with" and len(attached_args) == 2:
                        stack = self.value(attached_args[1])
                        if not (isinstance(stack, tuple) and stack[0] == "id"):
                            return None
                        stacks.append(stack[1])
                if not stacks:
                    return None
                entries["VisualizedItems"] = ["items", stacks]
            elif owner == param and name == "put" and len(args) == 2 and args[1].type == "method_invocation" \
                    and text(field(args[1], "name")) == "saveOptional":
                holder = field(args[1], "object")
                stack = self.value(holder)
                if isinstance(stack, tuple) and stack[0] == "id":
                    entries[java_string(args[0])] = ["item", stack[1]]
                elif text(holder) == "ItemStack.EMPTY":
                    entries[java_string(args[0])] = ["item", ""]
                else:
                    return None
        if entries or not [c for c in walk(body) if c.type == "method_invocation"]:
            return entries
        return None

    def entity_entries(self, node):
        """Data for the `be -> ...` statements whose effect a renderer shows."""
        param, body = self.lambda_body(node)
        if body is not None and body.type == "block" and len(body.named_children) == 1:
            body = body.named_children[0]
            body = body.named_children[0] if body.type == "expression_statement" else body
        if param is not None and body is not None and body.type == "method_invocation":
            name, target, args = invocation(body)
            if text(target) == param and name == "setAnimatedSpeed" and len(args) == 1:
                value = self.value(args[0])
                if isinstance(value, (int, float)) and not isinstance(value, bool):
                    return {"CreatePonderAnimatedSpeed": ["f", float(value)]}
        # `be.forceFlag = raise` (`PostboxBlockEntity`: the flag is up while it holds a package or is forced) and
        # `ccte.disabled = true` (`ContraptionControlsBlockEntity.disabled`, saved as `Disabled`): a field a renderer reads.
        if param is not None and body is not None and body.type == "assignment_expression" \
                and text(field(body, "operator")) == "=":
            left = text(field(body, "left"))
            keys = {"forceFlag": "Filled", "disabled": "Disabled"}
            value = self.value(field(body, "right"))
            if left.startswith(param + ".") and left[len(param) + 1:] in keys and isinstance(value, bool):
                return {keys[left[len(param) + 1:]]: ["b", value]}
        return None

    def crafter_rows(self, node, positions):
        """Emits the `crafter` row for a `be -> ...` lambda that acts on the Mechanical Crafters at `positions`.

        `getInventory().insertItem(0, stack, false)` puts one item in a crafter's slot (the slot holds one);
        `getInventory().setItem(0, stack)` sets it; `insertItemStacked(handler, stack, false)` through the item handler
        of a crafter spreads the stack over its joined crafters; `checkCompletedRecipe(true)` and `ejectWholeGrid()` are
        the calls themselves. All of them need the blocks to be crafters, as the structure has them.
        """
        if not all(self.blocks.get(tuple(int(v) for v in pos)) == CRAFTER for pos in positions):
            return False
        body = re.sub(r"\s+", " ", text(node))
        stacked = lambda name: self.env.get(name) if isinstance(self.env.get(name), tuple) and self.env[name][:1] == ("id",) else None
        for pattern, kind in ((r"getInventory\(\) ?\.insertItem\( ?0, ?(\w+)(?:\.copy\(\))?, ?false ?\)", "insert"),
                              (r"getInventory\(\) ?\.setItem\( ?0, ?(\w+) ?\)", "set"),
                              (r"insertItemStacked\( ?\w+, ?(\w+), ?false ?\)", "stack")):
            if (match := re.search(pattern, body)) and (stack := stacked(match[1])):
                self.emit("crafter", kind, [list(pos) for pos in positions], stack[1], stack[3] if len(stack) > 3 else 1)
                return True
        for call, kind in (("checkCompletedRecipe(true)", "check"), ("ejectWholeGrid()", "eject")):
            if call in body:
                self.emit("crafter", kind, [list(pos) for pos in positions], "", 0)
                return True
        return False

    def tunnel_entries(self, node, positions):
        """Data for a Brass Tunnel's `be.getBehaviour(..)` statements: the mode (`ScrollOptionBehaviour.setValue` of a
        `SelectionMode`'s ordinal, saved as `ScrollValue`) and a side's filter (`SidedFilteringBehaviour.setFilter`,
        saved under `Filters` by side, an empty stack taking the filter off)."""
        if not all(self.blocks.get(tuple(int(v) for v in pos)) == "create:brass_tunnel" for pos in positions):
            return None
        body = re.sub(r"\s+", " ", text(node))
        if match := re.search(r"getBehaviour\(ScrollOptionBehaviour\.TYPE\) ?\.setValue\( ?\w*\.?SelectionMode\.(\w+)\.ordinal\(\) ?\)", body):
            if match[1] in SELECTION_MODES:
                return {"ScrollValue": ["i", SELECTION_MODES.index(match[1])]}
        if match := re.search(r"getBehaviour\(SidedFilteringBehaviour\.TYPE\) ?\.setFilter\( ?Direction\.(\w+), ?([\w.]+) ?\)", body):
            stack = self.env.get(match[2])
            if match[2] == "ItemStack.EMPTY":
                return {"Filters": ["filters", {match[1].lower(): ""}]}
            if isinstance(stack, tuple) and stack[:1] == ("id",):
                return {"Filters": ["filters", {match[1].lower(): stack[1]}]}
        return None

    def press_burst(self, node):
        """The arguments of the `particles` row for `PressingBehaviour.makePressingParticleEffect(pos, stack)` and
        `makeCompactingParticleEffect(pos, stack)`: the item particles of one press stroke.

        The pressing burst is `getParticleAmount()` (the press: 15) particles at `pos` less a quarter block, thrown up
        at 1/8 with `offsetRandomly(.125)` across; the compacting one 20 at `pos`, up at 1/4, `.175` across. Both zero
        the vertical part of the random offset (`multiply(1, 0, 1)`).
        """
        param, body = self.lambda_body(node)
        for call in method_calls_all(body) if body is not None else ():
            name, _, args = invocation(call)
            at = self.value(args[0]) if len(args) == 2 else UNKNOWN
            if not numbers(at, 3):
                continue
            if name == "makePressingParticleEffect":
                return [[at[0], at[1] - 0.25, at[2]], [0, 0.125, 0], False, 15, 1, 0.125, True]
            if name == "makeCompactingParticleEffect":
                return [list(at), [0, 0.25, 0], False, 20, 1, 0.175, True]
        return None

    def fluid_op(self, node):
        """`[kind, fluid id, amount]` for a lambda that fills or drains a tank, or None.

        The `IFluidHandler` of a tank block (`ifh.fill(stack, EXECUTE)`, `ifh.drain(amount, EXECUTE)`), the tank
        behind `getTankInventory()` and `CreativeSmartFluidTank.setContainedFluid(stack)` all act on the controller.
        """
        param, body = self.lambda_body(node)
        if body is None:
            return None
        for call in method_calls_all(body):
            name, _, args = invocation(call)
            if name in {"fill", "drain"} and len(args) == 2 and text(args[1]).endswith("FluidAction.EXECUTE"):
                value = self.value(args[0])
                if name == "fill" and isinstance(value, tuple) and value[:1] == ("fluid",) and value[2] > 0:
                    return ["fill", value[3], value[2]]
                if name == "drain" and isinstance(value, int) and not isinstance(value, bool) and value > 0:
                    return ["drain", "", value]
            if name == "setContainedFluid" and len(args) == 1:
                value = self.value(args[0])
                if isinstance(value, tuple) and value[:1] == ("fluid",):
                    return ["set", value[3], 0]
        return None

    def select(self, name, values):
        x, y, z = self.dimensions
        if name == "everywhere":
            return box((0, 0, 0), (x - 1, y - 1, z - 1))
        if name == "layer" and numbers(values, 1):
            return box((0, values[0], 0), (x - 1, values[0], z - 1))
        if name == "layersFrom" and numbers(values, 1):
            return box((0, values[0], 0), (x - 1, y - 1, z - 1))
        if name == "layers" and numbers(values, 2):
            return box((0, values[0], 0), (x - 1, values[0] + values[1] - 1, z - 1))
        if name == "position":
            pos = vector("at", values)
            return [pos] if pos is not UNKNOWN else UNKNOWN
        if name in {"fromTo", "cuboid"}:
            if numbers(values, 6):
                return box(values[:3], values[3:])
            if len(values) == 2 and all(isinstance(v, tuple) for v in values):
                end = tuple(a + b for a, b in zip(*values)) if name == "cuboid" else values[1]
                return box(values[0], end)
        if name == "column" and numbers(values, 2):
            return box((values[0], 0, values[1]), (values[0], y - 1, values[1]))
        return UNKNOWN

    def world(self, name, values, args):
        if name == "getHolderLookupProvider":
            return UNKNOWN  # a registry handle `put`s pass on; the saved data is what matters
        selected = values[0] if values else UNKNOWN
        section = values[1] if len(values) > 1 else UNKNOWN
        if name in {"showSection", "showIndependentSection", "showIndependentSectionImmediately", "showSectionAndMerge"} and isinstance(selected, list):
            direction = DIRECTIONS.get(section) if isinstance(section, str) else None
            if name == "showSectionAndMerge" and len(values) > 2 and isinstance(values[2], int):
                section = values[2]
            elif name.startswith("showIndependent"):
                section = self.next_section
                self.next_section += 1
            else:
                section = 0
            self.emit("show", section, selected, direction)
            return section
        if name == "makeSectionIndependent" and isinstance(selected, list):
            section = self.next_section
            self.next_section += 1
            self.emit("hide", selected)
            self.emit("show", section, selected)
            return section
        if name == "hideSection" and isinstance(selected, list):
            self.emit("hide", selected, DIRECTIONS.get(section) if isinstance(section, str) else None)
            return UNKNOWN
        if name == "hideIndependentSection" and isinstance(selected, int):
            self.emit("hide_section", selected, DIRECTIONS.get(section) if isinstance(section, str) else None)
            return UNKNOWN
        if name == "moveSection" and isinstance(selected, int) and isinstance(section, tuple) and len(values) == 3 and isinstance(values[2], int):
            self.emit("move", selected, section, values[2])
            return UNKNOWN
        if name == "rotateSection" and isinstance(selected, int) and numbers(values[1:], 4):
            self.emit("rotate", selected, values[1:4], values[4])
            return UNKNOWN
        if name == "configureCenterOfRotation" and isinstance(selected, int) and isinstance(section, tuple):
            self.emit("pivot", selected, section)
            return UNKNOWN
        if name in {"setBlocks", "replaceBlocks", "setBlock"} and isinstance(section, tuple) and section[0] == "id":
            if isinstance(selected, tuple):
                selected = [selected]
            if isinstance(selected, list):
                self.emit("block", selected, section[1], section[2])
                return UNKNOWN
        if name == "destroyBlock" and isinstance(selected, tuple):
            self.emit("block", [selected], "minecraft:air", {})
            return UNKNOWN
        if name == "restoreBlocks" and isinstance(selected, list):
            self.emit("restore", selected)
            return UNKNOWN
        if name == "cycleBlockProperty" and isinstance(section, str) and isinstance(selected, (tuple, list)):
            self.emit("properties", [selected] if isinstance(selected, tuple) else selected, {section: None})
            return UNKNOWN
        if name in {"modifyBlock", "modifyBlocks"} and isinstance(selected, (tuple, list)) and len(args) > 1 and args[1].type == "lambda_expression":
            param = text(field(args[1], "parameters"))
            if param.isidentifier():
                before = self.env.get(param, UNKNOWN)
                self.env[param] = ("id", "", {})
                changed = self.value(field(args[1], "body"))
                self.env[param] = before
                if isinstance(changed, tuple) and changed[0] == "id":
                    positions = [selected] if isinstance(selected, tuple) else selected
                    if any(isinstance(spec, dict) for spec in changed[2].values()):
                        # Reads the old state: `ponder_assets.expand_properties` resolves it against the structure.
                        self.emit("update", positions, changed[1], changed[2])
                    elif changed[1]:
                        self.emit("block", positions, changed[1], changed[2])
                    elif changed[2]:
                        self.emit("properties", positions, changed[2])
                    return UNKNOWN
        if name in {"setKineticSpeed", "multiplyKineticSpeed"} and isinstance(selected, list) and isinstance(section, (int, float)):
            self.emit("speed" if name == "setKineticSpeed" else "multiply_speed", selected, section)
            return UNKNOWN
        if name == "modifyKineticSpeed" and isinstance(selected, list) and len(args) == 2 and args[1].type == "lambda_expression":
            param = text(field(args[1], "parameters"))
            if param.isidentifier():
                before = self.env.get(param, UNKNOWN)
                self.env[param] = 0.0
                base = self.value(field(args[1], "body"))
                self.env[param] = 1.0
                scaled = self.value(field(args[1], "body"))
                self.env[param] = before
                if numbers([base, scaled], 2):
                    self.emit("affine_speed", selected, scaled - base, base)
                    return UNKNOWN
        # `createItemOnBelt(pos, side, stack)` returns the item's `ElementLink`; `createItemOnBeltLike` is void. The
        # belt (or depot) carries the item from there (`rules::ponder_belt`).
        if name in {"createItemOnBelt", "createItemOnBeltLike"} and len(values) == 3 and numbers(selected, 3)                 and isinstance(values[2], tuple) and values[2][0] == "id":
            link = self.next_link
            self.next_link += 1
            self.emit("belt_put", link, list(selected), values[2][1])
            # `createItemOnBelt` and `createItemOnBeltLike` end with `flapFunnel(beltLocation.above(), true)`.
            self.emit("flap", [selected[0], selected[1] + 1, selected[2]], True)
            return ("link", link)
        if name == "removeItemsFromBelt" and len(values) == 1 and numbers(selected, 3):
            self.emit("belt_remove", list(selected))
            return UNKNOWN
        if name == "stallBeltItem" and len(values) == 2 and isinstance(selected, tuple) and selected[0] == "link"                 and isinstance(section, bool):
            self.emit("belt_stall", selected[1], section)
            return UNKNOWN
        if name == "changeBeltItemTo" and len(values) == 2 and isinstance(selected, tuple) and selected[0] == "link"                 and isinstance(section, tuple) and section[0] == "id":
            self.emit("belt_change", selected[1], section[1])
            return UNKNOWN
        # `createItemEntity(location, motion, stack)` returns the entity's `ElementLink`; `modifyEntity(link,
        # Entity::discard)` and `modifyEntities(ItemEntity.class, Entity::discard)` remove it (`item_end`).
        if name == "createItemEntity" and len(values) == 3:
            stack, motion = values[2], section
            if numbers(selected, 3) and isinstance(stack, tuple) and stack[0] == "id" and isinstance(motion, tuple) and numbers(motion, 3):
                link = self.next_link
                self.next_link += 1
                self.emit("item", stack[1], selected, motion, 40, link)
                return ("link", link)
        if name == "modifyEntity" and len(args) == 2 and text(args[1]) == "Entity::discard":
            if isinstance(selected, tuple) and selected[0] == "link":
                self.emit("item_end", selected[1])
                return UNKNOWN
        if name == "modifyEntities" and len(args) == 2 and text(args[0]) == "ItemEntity.class" and text(args[1]) == "Entity::discard":
            self.emit("item_end", 0)
            return UNKNOWN
        # `setFilterData(selection, class, stack)`: `FilteringBehaviour.setFilter`, saved as the block entity's `Filter`.
        if name == "setFilterData" and len(values) == 3 and isinstance(values[2], tuple) and values[2][0] == "id":
            positions = [selected] if numbers(selected, 3) else selected
            if isinstance(positions, list) and positions:
                self.emit("nbt", positions, {"Filter": ["item", values[2][1]]})
                return UNKNOWN
        # `flapFunnel(pos, outward)`: `FunnelBlockEntity.flap(!outward)`.
        if name == "flapFunnel" and len(values) == 2 and numbers(selected, 3) and isinstance(section, bool):
            self.emit("flap", list(selected), section)
            return UNKNOWN
        # The Ponder library's `incrementBlockBreakingProgress(pos)`: the stage shown is the number of calls less one,
        # and the tenth call breaks the block (`addBlockDestroyEffects`, `destroyBlock`) and starts over.
        if name == "incrementBlockBreakingProgress" and numbers(selected, 3):
            key = tuple(selected)
            calls = self.breaking.get(key, 0) + 1
            if calls == 10:
                self.breaking.pop(key, None)
                self.emit("destroy", list(key))
                self.emit("block", [key], "minecraft:air", {})
                self.emit("crack", list(key), -1)
            else:
                self.breaking[key] = calls
                self.emit("crack", list(key), calls - 1)
            return UNKNOWN
        # `modifyBlockEntityNBT(selection, class, nbt -> ...)`: the `put*` calls of the lambda, applied to the block
        # entity's data (the renderers read the same keys Create saves).
        if name == "modifyBlockEntityNBT" and len(args) >= 3 and args[2].type == "lambda_expression":
            positions = [selected] if isinstance(selected, tuple) and numbers(selected, 3) else selected
            entries = self.nbt_entries(args[2]) if isinstance(positions, list) else None
            if entries is not None:
                if entries:
                    self.emit("nbt", positions, entries)
                return UNKNOWN
        # `modifyBlockEntity(pos, class, be -> ...)`: the statements known to change what a renderer draws.
        if name == "modifyBlockEntity" and len(args) == 3 and args[2].type == "lambda_expression":
            positions = [selected] if isinstance(selected, tuple) and numbers(selected, 3) else selected
            if isinstance(positions, list) and positions:
                if self.crafter_rows(args[2], positions):
                    return UNKNOWN
                entries = self.tunnel_entries(args[2], positions) or self.entity_entries(args[2])
                if entries:
                    self.emit("nbt", positions, entries)
                    return UNKNOWN
                burst = self.press_burst(args[2])
                if burst:
                    self.emit("particles", "item", *burst)
                    return UNKNOWN
                # Fluid in a Fluid Tank: the tank rules (`rules::ponder_fluid`) replay it.
                fluid = self.fluid_op(args[2])
                if fluid and all(self.blocks.get(tuple(int(v) for v in pos)) in TANKS | VESSELS for pos in positions):
                    self.emit("fluid", positions, *fluid)
                    return UNKNOWN
        # `setCraftingResult(crafter, stack)` is `setScriptedResult`; `connectCrafterInvs(a, b)` toggles the two crafters'
        # joined inventory (`ConnectedInputHandler.toggleConnection`). The replay of the crafters is `rules::ponder_crafter`.
        if name == "setCraftingResult" and numbers(selected, 3) and isinstance(section, tuple) and section[:1] == ("id",):
            self.emit("crafter", "result", [list(selected)], section[1], section[3] if len(section) > 3 else 1)
            return UNKNOWN
        if name == "connectCrafterInvs" and numbers(selected, 3) and numbers(section, 3):
            self.emit("crafter", "connect", [list(selected), list(section)], "", 0)
            return UNKNOWN
        # `setDisplayBoardText(board, line, text)` is `FlapDisplayBlockEntity.applyTextManually` and `dyeDisplayBoard`
        # `setColour` (white clears it): the board's controller shows them.
        if name == "setDisplayBoardText" and numbers(selected, 3) and isinstance(section, int) and len(values) == 3 \
                and isinstance(values[2], tuple) and values[2][:1] in {("text",), ("lang",), ("name",), ("parts",)}:
            self.emit("board", "text", [list(selected)], section, text_parts(values[2]))
            return UNKNOWN
        if name == "dyeDisplayBoard" and numbers(selected, 3) and isinstance(section, int) and len(values) == 3 \
                and isinstance(values[2], str):
            self.emit("board", "dye", [list(selected)], section, values[2])
            return UNKNOWN
        # `flashDisplayLink(pos)` is `LinkWithBulbBlockEntity.pulse`: the bulb glows from now on.
        if name == "flashDisplayLink" and numbers(selected, 3):
            self.emit("pulse", [selected])
            return UNKNOWN
        # `toggleControls(pos)` is `cycleBlockProperty(pos, ControlsBlock.VIRTUAL)`.
        if name == "toggleControls" and numbers(selected, 3):
            self.emit("properties", [selected], {"virtual": None})
            return UNKNOWN
        # `propagatePipeChange(pos)` is `onSpeedChanged(0)` on the pump: it only restarts the pipes' flow, which
        # nothing here draws, so there is nothing to show.
        if name == "propagatePipeChange" and numbers(selected, 3):
            self.inert["world.propagatePipeChange"] += 1
            return UNKNOWN
        if name == "toggleRedstonePower" and isinstance(selected, list):
            self.emit("toggle", selected)
            return UNKNOWN
        # `pte.getPressingBehaviour().start(Mode.BELT)`: the press head's stroke (PressingBehaviour.start).
        if name == "modifyBlockEntity" and isinstance(selected, tuple) and len(args) == 3 and args[2].type == "lambda_expression":
            started = re.search(r"\.getPressingBehaviour\(\)\s*\.start\(\s*Mode\.(WORLD|BELT|BASIN)\s*\)", text(args[2]))
            if started and numbers(selected, 3):
                self.emit("press", [selected], started[1].lower())
                return UNKNOWN
        # `instructArm(armLocation, phase, heldItem, targetedPoint)`: `ArmBlockEntity.Phase`, `ItemStack.EMPTY` for none.
        if name == "instructArm" and len(values) == 4 and isinstance(selected, tuple) and numbers(selected, 3)                 and isinstance(section, str) and isinstance(values[3], int):
            held = values[2][1] if isinstance(values[2], tuple) and values[2][0] == "id" else None
            self.emit("arm", list(selected), section.upper(), held, values[3])
            return UNKNOWN
        # `CreateSceneBuilder.WorldInstructions`: `AnimateBlockEntityInstruction.bearing / pulley / deployer`.
        if name in ANIMATED and isinstance(selected, tuple) and numbers(selected, 3) and numbers(values[1:], 2):
            self.emit("animate", ANIMATED[name], list(selected), values[1], max(0, int(values[2])))
            return UNKNOWN
        self.unhandled[f"world.{name}"] += 1
        return UNKNOWN


NBT_TYPES = {"putInt": "i", "putFloat": "f", "putDouble": "f", "putLong": "i", "putByte": "i", "putShort": "i",
             "putBoolean": "b", "putString": "s"}


def method_calls_all(node):
    return (child for child in walk(node) if child.type == "method_invocation")


ANIMATED = {"rotateBearing": "bearing", "movePulley": "pulley", "moveDeployer": "deployer"}

DIRECTIONS = {"up": (0, 1, 0), "down": (0, -1, 0), "north": (0, 0, -1),
              "south": (0, 0, 1), "east": (1, 0, 0), "west": (-1, 0, 0)}


HIGHLIGHTS = {"showFilterSlotInput", "showScrollInput", "showCenteredScrollInput", "showRepeaterScrollInput"}


def highlight_box(name, values):
    """`(low, high)` of the box `PonderOverlayInstructions.<name>` outlines, from the bytecode of the Ponder library:

    - `showFilterSlotInput(location, duration)`: 0.1 either way; `(location, side, duration)`: the location moved
      0.0234375 into the block and `VecHelper.axisAlingedPlaneOf(side)` (1 on the axes across `side`) * 0.0859375.
    - `showScrollInput(location, side, duration)`: 0.0625 along `side`'s axis, 0.25 across it;
      `showCenteredScrollInput(pos, side, duration)` is that at the block's surface on `side`.
    - `showRepeaterScrollInput(pos, duration)`: (1/6, 0.0625, 1/6) around the bottom surface raised 0.1875.
    """
    if not values or not isinstance(values[-1], int) or isinstance(values[-1], bool):
        return None
    sides = [v for v in values if isinstance(v, str) and v in DIRECTIONS]
    side = sides[0] if sides else None
    if name == "showRepeaterScrollInput" and len(values) == 2 and numbers(values[0], 3):
        centre = vector("blockSurface", [values[0], "down"])
        centre = (centre[0], centre[1] + 0.1875, centre[2])
        extent = (1 / 6, 0.0625, 1 / 6)
    elif name == "showFilterSlotInput" and len(values) == 2 and numbers(values[0], 3):
        centre, extent = values[0], (0.1, 0.1, 0.1)
    elif name == "showFilterSlotInput" and len(values) == 3 and numbers(values[0], 3) and side:
        centre = tuple(c - 0.0234375 * n for c, n in zip(values[0], DIRECTIONS[side]))
        extent = tuple((1 - abs(n)) * 0.0859375 for n in DIRECTIONS[side])
    elif name in {"showScrollInput", "showCenteredScrollInput"} and len(values) == 3 and side and numbers(values[0], 3):
        centre = values[0] if name == "showScrollInput" else vector("blockSurface", [values[0], side])
        extent = tuple(0.0625 if n else 0.25 for n in map(abs, DIRECTIONS[side]))
    else:
        return None
    if centre is UNKNOWN:
        return None
    return (tuple(c - e for c, e in zip(centre, extent)), tuple(c + e for c, e in zip(centre, extent)))


AABB_METHODS = {"move", "inflate", "deflate", "contract", "expandTowards", "getCenter"}


def aabb_method(box, name, values):
    """`AABB.move`, `inflate`, `deflate`, `contract`, `expandTowards` and `getCenter` of a `("aabb", low, high)`."""
    _, low, high = box
    if name == "getCenter" and not values:
        return tuple((a + b) / 2 for a, b in zip(low, high))
    if len(values) == 1 and isinstance(values[0], tuple) and numbers(values[0], 3):
        values = list(values[0])
    elif len(values) == 1 and isinstance(values[0], (int, float)) and not isinstance(values[0], bool) and name in {"inflate", "deflate"}:
        values = [values[0]] * 3
    if not numbers(values, 3):
        return UNKNOWN
    low, high = list(low), list(high)
    for axis, amount in enumerate(values):
        if name == "move":
            low[axis] += amount
            high[axis] += amount
        elif name in {"inflate", "deflate"}:
            amount = amount if name == "inflate" else -amount
            low[axis] -= amount
            high[axis] += amount
        elif name == "expandTowards":
            # Towards a negative amount the low side grows, towards a positive one the high side.
            if amount < 0:
                low[axis] += amount
            else:
                high[axis] += amount
        else:
            # `contract` shrinks: a negative amount raises the low side, a positive one lowers the high side.
            if amount < 0:
                low[axis] -= amount
            else:
                high[axis] -= amount
    return ("aabb", tuple(low), tuple(high))


def java_text(value):
    """`"" + value` as Java writes it: a whole number without a point."""
    if isinstance(value, float) and value == int(value):
        return str(int(value))
    return str(value)


def text_parts(value):
    """The parts of a board's text: `[["lit", text], ["lang", key], ["name", item]]`."""
    kind = value[0]
    if kind == "parts":
        return value[1]
    if kind == "text":
        return [["lit", value[1]]] if value[1] else []
    return [[kind if kind != "text" else "lit", value[1]]]


def numbers(values, length):
    return isinstance(values, (list, tuple)) and len(values) == length and all(isinstance(value, (int, float)) for value in values)


def vector(name, values):
    if name in {"of", "at"} and numbers(values, 3):
        return tuple(values)
    if name == "at" and len(values) == 1 and isinstance(values[0], tuple):
        return values[0]
    if name in {"topOf", "centerOf", "blockSurface"}:
        direction = values[-1] if values and isinstance(values[-1], str) else "up"
        coordinates = values[:-1] if values and isinstance(values[-1], str) else values
        pos = vector("at", coordinates)
        if pos is not UNKNOWN:
            offset = DIRECTIONS.get(direction, (0, 0, 0)) if name != "centerOf" else (0, 0, 0)
            return tuple(a + (1 + b) * 0.5 for a, b in zip(pos, offset))
    if values and isinstance(values[0], tuple) and len(values[0]) == 3:
        origin, *rest = values
        alias = {"above": "up", "below": "down"}.get(name, name)
        direction = DIRECTIONS.get(alias, DIRECTIONS.get(rest[0]) if rest and isinstance(rest[0], str) else None)
        if direction is not None and (alias in DIRECTIONS or name == "relative"):
            length = next((value for value in rest if isinstance(value, (int, float))), 1)
            return tuple(a + b * length for a, b in zip(origin, direction))
        if name in {"add", "offset", "subtract", "substract"}:
            offset = rest[0] if len(rest) == 1 and isinstance(rest[0], tuple) else rest
            if numbers(offset, 3):
                sign = 1 if name in {"add", "offset"} else -1
                return tuple(a + sign * b for a, b in zip(origin, offset))
        if name == "distanceTo" and len(rest) == 1 and isinstance(rest[0], tuple) and numbers(rest[0], 3):
            return sum((a - b) ** 2 for a, b in zip(origin, rest[0])) ** 0.5
        if name == "scale" and numbers(rest, 1):
            return tuple(a * rest[0] for a in origin)
    return UNKNOWN


def box(start, end):
    if not numbers(start, 3) or not numbers(end, 3):
        return UNKNOWN
    axes = [range(int(min(a, b)), int(max(a, b)) + 1) for a, b in zip(start, end)]
    return list(product(*axes))


def selection_bounds(points):
    axes = list(zip(*points))
    return tuple(map(min, axes)), tuple(max(axis) + 1 for axis in axes)


def generate(create, jar, output, models_output):
    from zipfile import ZipFile
    from tree_sitter import Language, Parser
    import tree_sitter_java
    from ponder_assets import structure, model_resolver, expand_properties

    parser = Parser(Language(tree_sitter_java.language()))
    parse = lambda path: parser.parse(path.read_bytes()).root_node
    directory = create / "infrastructure/ponder"
    registry = Registry(create)
    SELECTION_MODES[:] = selection_modes(create)
    index = registrations(parse(directory / "AllCreatePonderScenes.java"), registry)
    groups = tags(parse(directory / "AllCreatePonderTags.java"), registry)
    method_groups = {}
    for path in (directory / "scenes").rglob("*.java"):
        for method in (n for n in walk(parse(path)) if n.type == "method_declaration"):
            method_groups.setdefault(path.stem, {})[text(field(method, "name"))] = method
    missing, unhandled, inert, models = [], Counter(), Counter(), set()
    with ZipFile(jar) as archive:
        lang = json.loads(archive.read("assets/create/lang/en_us.json"))
        models_for = model_resolver(archive)
        for (path, reference), scene in index.items():
            owner, method = reference.split("::")
            size, palette, blocks, _ = structure(archive.read(f"assets/create/ponder/{path}.nbt"))
            helpers = method_groups[owner]
            script = Storyboard(registry, helpers[method], helpers, size,
                                {tuple(pos): palette[index][0] for pos, index in blocks})
            key = script.key
            if key is None or f"create.ponder.{key}.header" not in lang:
                missing.append((path, reference, key))
                continue
            prefix = f"create.ponder.{key}."
            captions = [code for code in lang if code.startswith(prefix + "text_")]
            used = set()
            for op in script.ops:
                if op[1] == "caption":
                    source = op.pop()
                    if not op[-1]:
                        op.pop()
                    fallback = prefix + op[2]
                    op[2] = next((code for code in captions if code not in used and lang[code] == source), fallback)
                    used.add(op[2])
            expanded_states = [(op[3], op[4]) for op in expand_properties(archive, palette, blocks, script.ops) if op[1] == "block"]
            for id, props in [*palette, *expanded_states]:
                models.update(name for name, _, _ in models_for(id, tuple(sorted(props.items()))))
            scene.update(key=key, bounds=script.bounds, scale=script.scale,
                         offset=script.offset, duration=script.tick, ops=script.ops)
            scene["tags"] = [groups[tag]["id"] for tag in scene["tags"]]
            unhandled.update(script.unhandled)
            inert.update(script.inert)
        if missing:
            raise ValueError(f"Unmatched Java scenes: {missing}")
    scenes = list(index.values())
    lines = [json.dumps(row, separators=(",", ":")) for row in scenes]
    categories = json.dumps(list(groups.values()), separators=(",", ":"))
    output.write_text('{"tags":' + categories + ',\n"scenes":[\n' + ',\n'.join(lines) + '\n]}\n', encoding="utf-8")
    lines = "".join(f"create:{name}\n" for name in sorted(models))
    models_output.write_text(
        "//! Generated by tools/ponder_storyboards.py: the models the Ponder scenes draw. A `.rs` file, because the package\n"
        "//! reader accepts only `.rs` and `.toml` under `src/`.\n\n"
        f'pub const MODELS: &str = "\\\n{lines}";\n',
        encoding="utf-8",
    )
    print(f"{len(scenes)} registered storyboards and {len(groups)} Ponder tags")
    print("Unsupported source operations:", dict(unhandled))
    print("Instructions with nothing to draw:", dict(inert))


if __name__ == "__main__":
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("source", type=Path, help="Create checkout at mc1.21.1-6.0.10")
    cli.add_argument("jar", type=Path)
    cli.add_argument("--output", type=Path, default=Path(__file__).with_name("ponder_storyboards.json"))
    cli.add_argument("--models", type=Path, default=Path(__file__).parents[1] / "mod/src/ponder/archive_models.rs")
    args = cli.parse_args()
    generate(args.source / "src/main/java/com/simibubi/create", args.jar, args.output, args.models)
