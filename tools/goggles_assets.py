"""Package the words of Engineer's Goggles' overlay from the player's own Create jar.

The overlay (`native/api/src/machines/goggles.rs`) writes no English of its own: every line is one
of Create's language keys (`create.gui.goggles.*`, `create.tooltip.*`, `create.boiler.*`, ...) read
from `assets/create/goggles/lang.json` in the package, which this copies from the jar's
`assets/create/lang/en_us.json`. Create's text is not kept in the source tree.
"""

import json
from pathlib import Path
from zipfile import ZipFile

LANG = "assets/create/lang/en_us.json"
OUT = "assets/create/goggles/lang.json"

# What `CreateLang.translate` and `translateDirect` read in the goggle overlay: `GoggleOverlayRenderer`,
# `KineticBlockEntity`, `IRotate`, the gauges, `BoilerData`, `TooltipHelper.addHint`, the hints,
# `IDisplayAssemblyExceptions`, the chute, deployer, tunnel, lever and whistle.
PREFIXES = (
    "create.generic.",
    "create.gui.goggles.",
    "create.gui.gauge.",
    "create.gui.speedometer.",
    "create.gui.stressometer.",
    "create.gui.contraptions.",
    "create.tooltip.",
    "create.boiler.",
    "create.hint.",
    # `SequencedAssemblyRecipe.addToTooltip` and `IAssemblyRecipe.getDescriptionForAssembly`.
    "create.recipe.assembly.",
)
KEYS = ("create.gui.assembly.exception", "create.recipe.sequenced_assembly")


def import_goggles(jar: ZipFile, content: Path):
    """Writes the overlay's language keys into the package `content`."""
    lang = json.loads(jar.read(LANG))
    chosen = {key: value for key, value in lang.items()
              if key.startswith(PREFIXES) or key in KEYS}
    missing = [key for key in (
        "create.gui.goggles.kinetic_stats", "create.tooltip.stressImpact",
        "create.boiler.status", "create.hint.empty_bearing", "create.recipe.assembly.next",
    ) if key not in chosen]
    if missing:
        raise SystemExit(f"Create's language file lacks {missing}")
    path = content / OUT
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(chosen, indent=1, ensure_ascii=False), encoding="utf-8", newline="\n")
