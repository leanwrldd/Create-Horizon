"""Build the local Ponder scene pack from Create's schematic and language assets."""

import gzip
from io import BytesIO
import json
from pathlib import Path
import struct
from functools import lru_cache
from zipfile import ZipFile


def read_nbt(raw):
    data = gzip.decompress(raw)
    offset = 0

    def read(fmt):
        nonlocal offset
        value = struct.unpack_from(">" + fmt, data, offset)[0]
        offset += struct.calcsize(">" + fmt)
        return value

    def string():
        nonlocal offset
        size = read("H")
        if offset + size > len(data):
            raise ValueError("Truncated schematic string")
        value = data[offset:offset + size].decode("utf-8", errors="replace")
        offset += size
        return value

    def tag(kind):
        if kind in range(1, 7):
            return read("bhiqfd"[kind - 1])
        if kind == 8:
            return string()
        if kind == 10:
            result = {}
            while child := read("B"):
                name = string()
                result[name] = tag(child)
            return result
        if kind == 9:
            child, size = read("B"), read("i")
            return [tag(child) for _ in range(size)]
        if kind in {7, 11, 12}:
            size = read("i")
            return [read({7: "b", 11: "i", 12: "q"}[kind]) for _ in range(size)]
        raise ValueError(f"Unexpected schematic NBT tag {kind}")

    root = read("B")
    string()
    return tag(root)


def structure(raw):
    nbt = read_nbt(raw)
    palette = [(entry["Name"], entry.get("Properties", {})) for entry in nbt["palette"]]
    blocks = [(entry["pos"], entry["state"]) for entry in nbt["blocks"] if palette[entry["state"]][0] != "minecraft:air"]
    speeds = [(entry["pos"], speed) for entry in nbt["blocks"]
              if (speed := entry.get("nbt", {}).get("Speed", 0)) != 0]
    return nbt["size"], palette, blocks, speeds


def arm_points(raw):
    """Each Mechanical Arm's interaction points (`InteractionPoints`: saved relative to the arm) as
    `[arm pos, [[type, mode, absolute pos], ...]]`, for `world.instructArm`."""
    def offset(pos):
        # Create 6.0.10's Ponder files save `Pos` as `NbtUtils.writeBlockPos`'s {X, Y, Z} compound; newer
        # saves may use an int array.
        return [pos["X"], pos["Y"], pos["Z"]] if isinstance(pos, dict) else list(pos)

    arms = []
    for entry in read_nbt(raw)["blocks"]:
        points = entry.get("nbt", {}).get("InteractionPoints")
        if points:
            at = entry["pos"]
            arms.append([at, [[p["Type"], p.get("Mode", "DEPOSIT"), [a + o for a, o in zip(at, offset(p["Pos"]))]]
                              for p in points]])
    return arms


TANKS = ("create:fluid_tank", "create:creative_fluid_tank")


def tank_data(raw):
    """Each Fluid Tank block's saved multi-block data as `[pos, controller, width, height, window, fluid, amount]`
    (`FluidTankBlockEntity.write`): the controller's `Size`, `Height`, `Window` and `TankContent`, and for the other
    blocks its `Controller`, made relative to the scene through `LastKnownPos`."""
    nbt = read_nbt(raw)
    palette = [entry["Name"] for entry in nbt["palette"]]
    tanks = {tuple(entry["pos"]): entry["nbt"] for entry in nbt["blocks"]
             if palette[entry["state"]] in TANKS and "nbt" in entry}

    def block_pos(value):
        return [value["X"], value["Y"], value["Z"]] if isinstance(value, dict) else list(value)

    rows = {}
    for pos, data in tanks.items():
        controller = list(pos)
        if "Controller" in data and "LastKnownPos" in data:
            known = block_pos(data["LastKnownPos"])
            controller = [a + c - k for a, c, k in zip(pos, block_pos(data["Controller"]), known)]
        content = data.get("TankContent", {})
        fluid = content.get("FluidName", content.get("id", "minecraft:empty"))
        amount = content.get("Amount", content.get("amount", 0))
        rows[pos] = [list(pos), controller, data.get("Size", 0), data.get("Height", 0), int(data.get("Window", 1)),
                     "" if fluid == "minecraft:empty" else fluid, amount]
    for row in rows.values():
        head = rows.get(tuple(row[1]))
        if head is not None and not row[2]:
            row[2], row[3], row[4], row[5], row[6] = head[2], head[3], head[4], head[5], head[6]
    return list(rows.values())


def model_resolver(jar):
    entries = set(jar.namelist())

    @lru_cache(maxsize=None)
    def json_asset(kind, name):
        path = f"assets/create/{kind}/{name}.json"
        return json.loads(jar.read(path)) if path in entries else {}

    @lru_cache(maxsize=None)
    def supported(name):
        model = json_asset("models", name)
        if not model or "loader" in model:
            return False
        parent = model.get("parent", "")
        return not parent.startswith("create:") or supported(parent.removeprefix("create:"))

    def check(conditions, props):
        for key, value in conditions.items():
            if key == "OR":
                if not any(check(entry, props) for entry in value):
                    return False
            elif key == "AND":
                if not all(check(entry, props) for entry in value):
                    return False
            elif str(props.get(key, "")).lower() not in str(value).lower().split("|"):
                return False
        return True

    @lru_cache(maxsize=None)
    def models_for(id, properties):
        if not id.startswith("create:"):
            return []
        props = dict(properties)
        asset = json_asset("blockstates", id.removeprefix("create:"))
        candidates = []
        if "variants" in asset:
            variants = sorted(asset["variants"].items(), key=lambda pair: pair[0].count("="), reverse=True)
            for spec, model in variants:
                options = dict(pair.split("=", 1) for pair in spec.split(",") if pair)
                if check(options, props):
                    candidates.append(model)
                    break
        candidates.extend(part["apply"] for part in asset.get("multipart", [])
                          if check(part.get("when", {}), props))
        models = []
        for model in candidates:
            if isinstance(model, list):
                model = model[0]
            name = model["model"].removeprefix("create:")
            if model["model"].startswith("create:") and supported(name):
                models.append((name, model.get("x", 0), model.get("y", 0)))
        return models

    return models_for


OPPOSITES = {"north": "south", "south": "north", "east": "west", "west": "east", "up": "down", "down": "up"}


def expand_properties(jar, palette, blocks, events):
    original = {tuple(pos): palette[index] for pos, index in blocks}
    changed = {}
    archived = set(jar.namelist())

    @lru_cache(maxsize=None)
    def values(id, key):
        if key == "axis":
            return ["x", "y", "z"]
        if key == "power":
            return list(map(str, range(16)))
        file = f"assets/create/blockstates/{id.removeprefix('create:')}.json"
        if id.startswith("create:") and file in archived:
            states = json.loads(jar.read(file)).get("variants", {})
            seen = [v for spec in states for item in spec.split(",")
                    for k, _, v in [item.partition("=")] if key == k]
            if seen:
                return list(dict.fromkeys(seen))
        if key.endswith("facing"):
            return ["north", "east", "south", "west"]
        return ["false", "true"]

    def apply(id, old, modifications):
        props = old.copy()
        for key, value in modifications.items():
            if value is None:
                variants = values(id, key)
                current = props.get(key)
                if current in variants:
                    value = variants[(variants.index(current) + 1) % len(variants)]
                elif current is not None or len(variants) < 2:
                    continue
                else:
                    value = variants[1]
            props[key] = value
        return id, props

    def resolve(spec, old):
        """A property value an instruction takes from the block's old state (`getValue(P)`, offset or turned round)."""
        value = old.get(spec["from"])
        if value is None:
            return None
        if spec.get("opposite"):
            return OPPOSITES.get(value, value)
        if spec.get("add"):
            return str(int(value) + spec["add"]) if value.lstrip("-").isdigit() else None
        return value

    for tick, op, *args in events:
        if op == "update":
            # `modifyBlock(pos, s -> ...s.getValue(P)...)`: the same instruction for blocks that differ.
            groups = {}
            for pos in args[0]:
                spot = tuple(pos)
                state = changed.get(spot, original.get(spot))
                if state is None:
                    continue
                old_id, old_props = state
                new_id = args[1] or old_id
                props = {} if new_id != old_id else dict(old_props)
                for key, spec in args[2].items():
                    value = resolve(spec, old_props) if isinstance(spec, dict) else spec
                    if value is not None:
                        props[key] = value
                if (new_id, props) == (old_id, old_props):
                    continue
                changed[spot] = (new_id, props)
                groups.setdefault((new_id, tuple(sorted(props.items()))), []).append(pos)
            for (id, props), positions in groups.items():
                yield [tick, "block", positions, id, dict(props)]
        elif op == "properties":
            groups = {}
            for pos in args[0]:
                spot = tuple(pos)
                state = changed.get(spot, original.get(spot))
                if state is None:
                    continue
                next_state = apply(*state, args[1])
                if state == next_state:
                    continue
                changed[spot] = next_state
                id, props = next_state
                groups.setdefault((id, tuple(sorted(props.items()))), []).append(pos)
            for (id, props), positions in groups.items():
                yield [tick, "block", positions, id, dict(props)]
        else:
            if op == "block":
                changed.update((tuple(pos), (args[1], args[2])) for pos in args[0])
            elif op == "restore":
                for pos in args[0]:
                    changed.pop(tuple(pos), None)
            yield [tick, op, *args]


def import_ponder(jar, content):
    manifest = Path(__file__).with_name("ponder_storyboards.json")
    catalogue = json.loads(manifest.read_text(encoding="utf-8"))
    lang = json.loads(jar.read("assets/create/lang/en_us.json"))
    output = content / "assets/create/ponder"
    output.mkdir(parents=True, exist_ok=True)
    ponder_path = next(name for name in jar.namelist()
                       if name.startswith("META-INF/jarjar/ponder-") and name.endswith(".jar"))
    gui = content / "assets/create/textures/gui/ponder"
    gui.mkdir(parents=True, exist_ok=True)
    with ZipFile(BytesIO(jar.read(ponder_path))) as ponder:
        for name in ("logo", "widgets"):
            (gui / f"{name}.png").write_bytes(ponder.read(f"assets/ponder/textures/gui/{name}.png"))
        catalogue["ui"] = {
            key: value for key, value in json.loads(ponder.read("assets/ponder/lang/en_us.json")).items()
            if key.startswith("ponder.ui.")
        }
    models_for = model_resolver(jar)
    models = set()
    for tag in catalogue["tags"]:
        prefix = f'create.ponder.tag.{tag["id"]}'
        tag["title"] = lang[prefix]
        tag["description"] = lang[f"{prefix}.description"]
    for index, scene in enumerate(catalogue["scenes"]):
        key, path = scene.pop("key"), scene.pop("path")
        prefix = f"create.ponder.{key}."
        scene["title"] = lang[prefix + "header"]
        scene["file"] = f"scene_{index}.json"
        size, palette, blocks, speeds = structure(jar.read(f"assets/create/ponder/{path}.nbt"))
        schedule = {op[2] for op in scene["ops"] if op[1] == "caption"}
        events = []
        for tick, op, *values in expand_properties(jar, palette, blocks, scene.pop("ops")):
            if op == "caption":
                if values[0] not in lang:
                    continue
                values[0] = lang[values[0]]
            events.append([tick, op, *values])
        palette_states = palette + [(values[1], values[2]) for _, op, *values in events if op == "block"]
        fallback = []
        for id, props in palette_states:
            if parts := models_for(id, tuple(sorted(props.items()))):
                fallback.append(((id, props), parts))
                models.update(name for name, _, _ in parts)
        # Preserve captions a helper or conditional emitted in the Java
        # storyboard even if its timing could not be translated.
        end = scene.pop("duration")
        end = max([end, *(tick + min(140, v[1]) for tick, op, *v in events if op == "caption")])
        for text_key in sorted((k for k in lang if k.startswith(prefix + "text_")),
                               key=lambda k: int(k.rsplit("_", 1)[1])):
            if text_key not in schedule:
                events.append([end, "caption", lang[text_key], 100])
                end += 100
        arms = arm_points(jar.read(f"assets/create/ponder/{path}.nbt"))
        tanks = tank_data(jar.read(f"assets/create/ponder/{path}.nbt"))
        timeline = dict(size=size, palette=palette, blocks=blocks, speeds=speeds, events=events, models=fallback, arms=arms, tanks=tanks, duration=max(80, end),
                        bounds=scene.pop("bounds"), scale=scene.pop("scale"), offset=scene.pop("offset"))
        (output / scene["file"]).write_text(json.dumps(timeline, separators=(",", ":")), encoding="utf-8")
    (output / "catalog.json").write_text(json.dumps(catalogue, separators=(",", ":")), encoding="utf-8")
    return models
