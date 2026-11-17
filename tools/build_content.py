"""Build a local Horizon content mod using assets from the original Create JAR."""

import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import shutil
import subprocess
from tempfile import TemporaryDirectory
from zipfile import ZipFile

from gen_content import ROOT, generate, write
from goggles_assets import import_goggles
from ponder_assets import import_ponder


CREATE_SHA256 = "ef87fe5709f1ba1f5b8bb20a2925b5afb4669e178fd6d8bf10c167759eefe37a"
PREFIX = "assets/create/"
# What the belt's block entity draws (`BeltRenderer`, `AllPartialModels.BELT_*`).
BELT_MODELS = [f"block/belt/{part}{half}" for part in ("start", "middle", "end")
               for half in ("", "_bottom")] + ["block/belt_pulley"]


def find_models(value, found):
    """Every `create:` model a blockstate or item definition names."""
    if isinstance(value, dict):
        for key, inner in value.items():
            if key == "model" and isinstance(inner, str):
                if inner.startswith("create:"):
                    found.add(inner.removeprefix("create:"))
            else:
                find_models(inner, found)
    elif isinstance(value, list):
        for inner in value:
            find_models(inner, found)


def fit_blockstate(state, declared):
    """Create's blockstate file for a block declared here with only the `declared` state
    properties: variants are kept where every other property (`waterlogged`, `powered`...) is
    `false` or, for one that is never `false`, its first value, and those properties are dropped
    from the keys. A multipart file is passed on as it is."""
    variants = state.get("variants")
    if variants is None:
        return state
    seen = {}
    for key in variants:
        for pair in filter(None, key.split(",")):
            name, _, value = pair.partition("=")
            seen.setdefault(name, []).append(value)
    resting = {name: "false" if "false" in values else values[0]
               for name, values in seen.items() if name not in declared}
    out = {}
    for key, value in variants.items():
        pairs = [pair.partition("=") for pair in filter(None, key.split(","))]
        if any(resting.get(name, v) != v for name, _, v in pairs):
            continue
        out[",".join(f"{name}={v}" for name, _, v in pairs if name in declared)] = value
    return {"variants": out}


def import_assets(jar: ZipFile, content: Path, c, ponder_models=()):
    """Copies what the declarations in `c` (gen_content.collect) draw with from Create's jar."""
    entries = set(jar.namelist())

    def copy(name):
        if PREFIX + name not in entries:
            raise SystemExit(f"Create's jar has no {PREFIX}{name}")
        data = jar.read(PREFIX + name)
        path = content / PREFIX / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return data

    models = {"block/shaft_half", "block/hand_crank/handle", "block/water_wheel/wheel",
              "block/mechanical_press/head", *BELT_MODELS,
              *(name.removeprefix("create:") for name in c.models), *ponder_models}
    for path in (content / PREFIX / "items").glob("*.json"):
        find_models(json.loads(path.read_text(encoding="utf-8")), models)
    for name, declaration in c.blocks.items():
        path = f"blockstates/{name}.json"
        if name in c.blockstates:
            # A plugin's own mapping: a blockstate file, or a function of Create's (None if
            # Create has none) that answers one.
            custom = c.blockstates[name]
            if callable(custom):
                custom = custom(json.loads(jar.read(PREFIX + path)) if PREFIX + path in entries
                                else None)
            write(content / PREFIX / path, custom)
            find_models(custom, models)
            continue
        state = json.loads(copy(path))
        if name == "belt":
            # Every state draws the empty particle model; the block entity draws the belt.
            variants = state["variants"]
            state = {"variants": {key.replace(",waterlogged=false", ""): value
                                  for key, value in variants.items()
                                  if key.endswith("waterlogged=false")}}
        elif name == "hand_crank":
            variants = state["variants"]
            state = {"variants": {key.split(",")[0]: value for key, value in variants.items()
                                  if key.endswith("waterlogged=false")}}
        elif name == "water_wheel":
            variants = state["variants"]
            state = {"variants": {f"axis={axis}": variants[f"facing={face}"]
                                  for axis, face in {"x": "east", "y": "up", "z": "south"}.items()}}
        elif name not in c.base_blocks and declaration.get("template") == "minecraft:glass":
            # A `block()` declaration has only the states it declares. A block on any other
            # template (a door, a slab, a ladder) has its template's states, as Create's has.
            state = fit_blockstate(state, set(declaration.get("state_properties", {})))
            write(content / PREFIX / path, state)
        if name in ("belt", "hand_crank", "water_wheel"):
            write(content / PREFIX / path, state)
        find_models(state, models)

    copied = set()
    textures = set()
    # The models a plugin writes itself (`FILES`, as `belts_ii.py`'s sprite list) name textures
    # of Create's too.
    for path, value in c.files.items():
        if path.startswith(f"{PREFIX}models/") and isinstance(value, dict):
            textures.update(
                sprite.removeprefix("create:") for sprite in value.get("textures", {}).values()
                if isinstance(sprite, str) and sprite.startswith("create:"))
    while models:
        name = models.pop()
        if name in copied:
            continue
        copied.add(name)
        if f"{PREFIX}models/{name}.json" in c.files:
            continue  # A model a plugin writes itself (`FILES`): its textures are imported above.
        model = json.loads(copy(f"models/{name}.json"))
        if model.get("loader") == "neoforge:obj":
            mesh = PurePosixPath(model["model"].removeprefix("create:"))
            for line in copy(mesh.as_posix()).decode().splitlines():
                if line.startswith("mtllib "):
                    copy(mesh.with_name(line.split()[1]).as_posix())
        parent = model.get("parent", "")
        if parent.startswith("create:"):
            models.add(parent.removeprefix("create:"))
        # An item model's overrides name models too.
        find_models(model.get("overrides", []), models)
        textures.update(
            sprite.removeprefix("create:") for sprite in model.get("textures", {}).values()
            if isinstance(sprite, str) and sprite.startswith("create:")
        )
        # A `neoforge:composite` model's children (the display link, the nixie tube) have
        # textures and parents of their own.
        for child in (model.get("children") or {}).values():
            if not isinstance(child, dict):
                continue
            if child.get("parent", "").startswith("create:"):
                models.add(child["parent"].removeprefix("create:"))
            textures.update(
                sprite.removeprefix("create:") for sprite in child.get("textures", {}).values()
                if isinstance(sprite, str) and sprite.startswith("create:")
            )

    # Create draws a placed wheel in its material, spruce until changed, by swapping the oak
    # template sprites at runtime (WaterWheelRenderer.generateModel, material SPRUCE_PLANKS).
    # Horizon has no such swap, so the wheel model names spruce itself, as Create's item does.
    wheel = content / PREFIX / "models/block/water_wheel/wheel.json"
    model = json.loads(wheel.read_text(encoding="utf-8"))
    model["textures"] = {"log": "block/spruce_log", "planks": "block/spruce_planks"}
    write(wheel, model)
    # The Large Water Wheel's models likewise (its item model, `large_water_wheel/item`, names spruce
    # for `log`, `planks` and `log_top`).
    for part in ("block", "block_extension"):
        large = content / PREFIX / f"models/block/large_water_wheel/{part}.json"
        if large.is_file():
            model = json.loads(large.read_text(encoding="utf-8"))
            model["textures"] = {"log": "block/spruce_log", "planks": "block/spruce_planks",
                                 "log_top": "block/spruce_log_top"}
            write(large, model)

    # Create's sound events (`AllSoundEvents`) and the `.ogg` files they name, so a module may play
    # `create:whistle` and the rest. Events that name vanilla sounds need no file.
    sounds = json.loads(copy("sounds.json"))
    for event in sounds.values():
        for sound in event.get("sounds", []):
            name = sound if isinstance(sound, str) else sound.get("name", "")
            kind = "file" if isinstance(sound, str) else sound.get("type", "file")
            if kind == "file" and name.startswith("create:"):
                copy(f"sounds/{name.removeprefix('create:')}.ogg")

    for sprite in sorted(textures):
        name = f"textures/{sprite}.png"
        copy(name)
        if PREFIX + name + ".mcmeta" in entries:
            copy(name + ".mcmeta")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("create_jar", type=Path, help="Create 6.0.10 for Minecraft 1.21.1 (NeoForge)")
    parser.add_argument("--tool", type=Path, help="path to cargo-horizon-mod")
    parser.add_argument("--owner", help="build a private package under target/content-check-<owner>/ "
                        "without the plugins other owners are still writing (tools/wip.txt); "
                        "content/ is left alone")
    args = parser.parse_args()
    bundled = ROOT.parent / "target" / "horizon-tools" / "debug" / "cargo-horizon-mod.exe"
    tool = args.tool or (bundled if bundled.is_file() else shutil.which("cargo-horizon-mod"))
    if tool is None:
        parser.error("cargo-horizon-mod was not found. Pass its path with --tool.")

    try:
        with args.create_jar.open("rb") as source:
            if hashlib.file_digest(source, "sha256").hexdigest() != CREATE_SHA256:
                parser.error("Expected the original Create 6.0.10 for Minecraft 1.21.1 (NeoForge) JAR.")
            if args.owner:
                private = ROOT.parent / "target" / f"content-check-{args.owner}"
                generated = private / "content"
                shutil.rmtree(generated, ignore_errors=True)
                output = private / "create.hmod"
            else:
                generated = ROOT
                output = ROOT / "target" / "horizon-mod" / "create.hmod"
            output.parent.mkdir(parents=True, exist_ok=True)
            collected = generate(generated, args.owner)
            with ZipFile(source) as jar, TemporaryDirectory(dir=output.parent) as staging:
                content = Path(staging)
                shutil.copy2(ROOT / "horizon-mod.toml", content)
                shutil.copytree(generated / "assets", content / "assets")
                shutil.copytree(generated / "data", content / "data")
                models = import_ponder(jar, content)
                import_goggles(jar, content)
                import_assets(jar, content, collected, models)
                subprocess.run([str(Path(tool).resolve()), "horizon-mod", "build"],
                               cwd=content, check=True)
                (content / "target" / "horizon-mod" / "create.hmod").replace(output)
    except (OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"{error}\n")
    print(f"Built {output}")


if __name__ == "__main__":
    main()
