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


class Links(list):
    """An `ArrayList` a storyboard fills with the links `showIndependentSection` returns."""


class Storyboard:
    def __init__(self, registry, method, helpers, dimensions):
        self.registry = registry
        self.tick = 0
        self.env = {}
        self.ops = []
        self.unhandled = Counter()
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
            return self.env.get(source, UNKNOWN)
        if kind == "parenthesized_expression":
            return self.value(node.named_children[0])
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
            if UNKNOWN not in (left, right):
                operator = text(field(node, "operator"))
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
            if name == "ItemStack" and args and isinstance(args[0], tuple) and args[0][0] == "id":
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

        # `PackageItem.containing(...)` picks a random style (`PackageStyles.getRandomBox`): the first one stands for it.
        if target_text == "PackageItem" and name == "containing":
            return ("id", "create:cardboard_package_12x12", {})

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
                    if name in {"simpleParticleEmitter", "particleEmitterWithinBlockSpace"} and values                             and isinstance(values[0], str):
                        motion = values[1] if len(values) > 1 and isinstance(values[1], tuple) and numbers(values[1], 3) else (0, 0, 0)
                        return ("emitter", values[0], motion, name == "particleEmitterWithinBlockSpace")
                    # `emitParticles(location, emitter, amountPerCycle, cycles)`.
                    if name == "emitParticles" and len(values) == 4 and numbers(values[0], 3)                             and isinstance(values[1], tuple) and values[1][0] == "emitter" and numbers(values[2:], 2):
                        self.emit("particles", values[1][1], list(values[0]), list(values[1][2]), values[1][3],
                                  values[2], values[3])
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
            if value[0] == "id":
                if name in {"getDefaultState", "defaultBlockState", "get", "asStack", "asItem", "copy", "copyWithCount"}:
                    return value
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
        for call in method_calls_all(body):
            name, target, args = invocation(call)
            owner = text(target)
            if owner == param and name in NBT_TYPES and len(args) == 2:
                value = self.value(args[1])
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
                    if changed[1]:
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
                entries = self.entity_entries(args[2])
                if entries:
                    self.emit("nbt", positions, entries)
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
    index = registrations(parse(directory / "AllCreatePonderScenes.java"), registry)
    groups = tags(parse(directory / "AllCreatePonderTags.java"), registry)
    method_groups = {}
    for path in (directory / "scenes").rglob("*.java"):
        for method in (n for n in walk(parse(path)) if n.type == "method_declaration"):
            method_groups.setdefault(path.stem, {})[text(field(method, "name"))] = method
    missing, unhandled, models = [], Counter(), set()
    with ZipFile(jar) as archive:
        lang = json.loads(archive.read("assets/create/lang/en_us.json"))
        models_for = model_resolver(archive)
        for (path, reference), scene in index.items():
            owner, method = reference.split("::")
            size, palette, blocks, _ = structure(archive.read(f"assets/create/ponder/{path}.nbt"))
            helpers = method_groups[owner]
            script = Storyboard(registry, helpers[method], helpers, size)
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


if __name__ == "__main__":
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("source", type=Path, help="Create checkout at mc1.21.1-6.0.10")
    cli.add_argument("jar", type=Path)
    cli.add_argument("--output", type=Path, default=Path(__file__).with_name("ponder_storyboards.json"))
    cli.add_argument("--models", type=Path, default=Path(__file__).parents[1] / "mod/src/ponder/archive_models.rs")
    args = cli.parse_args()
    generate(args.source / "src/main/java/com/simibubi/create", args.jar, args.output, args.models)
