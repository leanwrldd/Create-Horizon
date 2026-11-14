"""Build a local Horizon content mod using assets from the original Create JAR."""

import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import shutil
import subprocess
from tempfile import TemporaryDirectory
from zipfile import ZipFile

from gen_content import BLOCKS, ITEMS, ROOT, main as generate, write


CREATE_SHA256 = "ef87fe5709f1ba1f5b8bb20a2925b5afb4669e178fd6d8bf10c167759eefe37a"
PREFIX = "assets/create/"


def import_assets(jar: ZipFile, content: Path):
    def copy(name):
        data = jar.read(PREFIX + name)
        path = content / PREFIX / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return data

    models = {"block/shaft_half", "block/hand_crank/handle", "block/water_wheel/wheel",
              "item/wrench/gear", *(f"item/{name}" for name in [*BLOCKS, *ITEMS])}
    for name in BLOCKS:
        path = f"blockstates/{name}.json"
        variants = json.loads(copy(path))["variants"]
        if name == "hand_crank":
            variants = {key.split(",")[0]: value for key, value in variants.items()
                        if key.endswith("waterlogged=false")}
            write(content / PREFIX / path, {"variants": variants})
        elif name == "water_wheel":
            variants = {f"axis={axis}": variants[f"facing={face}"]
                        for axis, face in {"x": "east", "y": "up", "z": "south"}.items()}
            write(content / PREFIX / path, {"variants": variants})
        models.update(value["model"].removeprefix("create:") for value in variants.values())

    copied = set()
    textures = set()
    while models:
        name = models.pop()
        if name in copied:
            continue
        copied.add(name)
        model = json.loads(copy(f"models/{name}.json"))
        if model.get("loader") == "neoforge:obj":
            mesh = PurePosixPath(model["model"].removeprefix("create:"))
            for line in copy(mesh.as_posix()).decode().splitlines():
                if line.startswith("mtllib "):
                    copy(mesh.with_name(line.split()[1]).as_posix())
        parent = model.get("parent", "")
        if parent.startswith("create:"):
            models.add(parent.removeprefix("create:"))
        textures.update(
            sprite.removeprefix("create:") for sprite in model.get("textures", {}).values()
            if sprite.startswith("create:")
        )

    # Create draws a placed wheel in its material, spruce until changed, by swapping the oak
    # template sprites at runtime (WaterWheelRenderer.generateModel, material SPRUCE_PLANKS).
    # Horizon has no such swap, so the wheel model names spruce itself, as Create's item does.
    wheel = content / PREFIX / "models/block/water_wheel/wheel.json"
    model = json.loads(wheel.read_text(encoding="utf-8"))
    model["textures"] = {"log": "block/spruce_log", "planks": "block/spruce_planks"}
    write(wheel, model)

    entries = set(jar.namelist())
    for sprite in sorted(textures):
        name = f"textures/{sprite}.png"
        copy(name)
        if PREFIX + name + ".mcmeta" in entries:
            copy(name + ".mcmeta")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("create_jar", type=Path, help="Create 6.0.10 for Minecraft 1.21.1 (NeoForge)")
    parser.add_argument("--tool", type=Path, help="path to cargo-horizon-mod")
    args = parser.parse_args()
    bundled = ROOT.parent / "target" / "horizon-tools" / "debug" / "cargo-horizon-mod.exe"
    tool = args.tool or (bundled if bundled.is_file() else shutil.which("cargo-horizon-mod"))
    if tool is None:
        parser.error("cargo-horizon-mod was not found. Pass its path with --tool.")

    try:
        with args.create_jar.open("rb") as source:
            if hashlib.file_digest(source, "sha256").hexdigest() != CREATE_SHA256:
                parser.error("Expected the original Create 6.0.10 for Minecraft 1.21.1 (NeoForge) JAR.")
            output = ROOT / "target" / "horizon-mod" / "create.hmod"
            output.parent.mkdir(parents=True, exist_ok=True)
            generate()
            with ZipFile(source) as jar, TemporaryDirectory(dir=output.parent) as staging:
                content = Path(staging)
                shutil.copy2(ROOT / "horizon-mod.toml", content)
                shutil.copytree(ROOT / "assets", content / "assets")
                shutil.copytree(ROOT / "data", content / "data")
                import_assets(jar, content)
                subprocess.run([str(Path(tool).resolve()), "horizon-mod", "build"],
                               cwd=content, check=True)
                (content / "target" / "horizon-mod" / "create.hmod").replace(output)
    except (OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"{error}\n")
    print(f"Built {output}")


if __name__ == "__main__":
    main()
