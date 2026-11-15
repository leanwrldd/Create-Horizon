# Create for Horizon, in Rust

A from-scratch Rust rewrite of [Create](https://github.com/Creators-of-Create/Create) for
[Horizon](https://github.com/horizon-rust/horizon), the Rust reimplementation of Minecraft: Java Edition 26.3.
Horizon has no Java runtime, so Create's jar cannot run there; this ports its rules to Horizon's native mod API.

It is written on Horizon's **trait API** (named hook points with versioned contracts), not on source mixins, so it
is not pinned to one Horizon commit.

## Progress

![How much of Create is ported](progress/progress.svg)

<!-- progress:start -->
| Area | Done | Total | |
|---|---:|---:|---:|
| Kinetics | 36 | 57 | 63% |
| Materials | 18 | 20 | 90% |
| Logistics | 4 | 52 | 8% |
| Fluids | 9 | 18 | 50% |
| Contraptions | 7 | 42 | 17% |
| Schematics | 0 | 5 | 0% |
| Curiosities | 0 | 13 | 0% |
| Palettes | 29 | 39 | 74% |
| **All** | **103** | **246** | **41.9%** |
<!-- progress:end -->

Every block and item registered in Create's `AllBlocks.java` and `AllItems.java` counts once, under the section
it is registered in (a dyed set like the 16 seats counts as one). It counts as done once `content/` declares it.
That measures breadth, not depth: a declared block can still be missing behaviour listed below. Redraw the chart
and this table after adding blocks:

```bash
python tools/progress.py
```

## What works so far: the kinetics core

| Block | Behaviour |
|---|---|
| Shaft | Passes rotation along its axis |
| Cogwheel | Shaft along its axis; reverses a small cogwheel beside it |
| Large Cogwheel | Drives a diagonal small cogwheel at twice the speed, reversed; meshes with a large one on another axis at a corner |
| Gearbox | Four shafts: the one opposite the input is reversed, the side ones turn |
| Creative Motor | Makes 16 RPM by default, with 16,384 su of capacity per RPM |
| Hand Crank | Generates rotation when turned by hand |
| Water Wheel | Generates rotation from nearby flowing water |

Also: Andesite Alloy, recipes, loot tables and a creative tab. Right-click with the Wrench to turn a kinetic block
to its next axis or step a Creative Motor through 16, 32, 64, 128, 256 RPM and the same in reverse.
Sneak-right-click to remove a kinetic block and recover it in Survival.

Network rules follow Create's `RotationPropagator`: the fastest generator sets a network's speed, a block asked for
two speeds at once or for more than 256 RPM breaks, and a network with more stress than capacity stops. Nothing in
this slice costs stress yet; the accounting is there for the machines that will.

### Depot

Right-click the top of a Depot with an item to put the whole stack on it; right-click the top again, with anything
or nothing in hand, to take the stack back (`SharedDepotBlockMethods.onUse`). The item slides in from the side the
player faces and settles in the centre, as Create's does. The stack is saved in the block entity's one item slot and
drops when the block is broken; the sneak-wrench picks the Depot up. Its logic is in `native/api/src/depot.rs`.

Belts ending at it hand their items on, items landing on it are caught, hoppers feed and empty it, and a press
above presses what it holds, putting extra results in eight output slots drawn in a ring (Belts II, below).

Not yet: funnels, upright and package items. A block item's model is told apart from a flat
one by whether the item names a block, not by its model's `gui3d` flag.

### Mechanical Belt (straight, horizontal)

Click a shaft with the Mechanical Belt (`create:belt_connector`, six dried kelp), then a second shaft on the same
axis and level, in line across that axis and closer than 20 blocks (`BeltConnectorItem.canConnect`); sneak-click to
forget the first. The belt turns with the shafts at its ends and at any shaft it was laid through. It moves at
RPM / 480 blocks a tick (`BeltBlockEntity.getBeltMovementSpeed`).

Drop items on it and each stack (up to its maximum size) rides as one, keeping a block from the one ahead
(`BeltInventory.tick`). At the end, an open block lets it fly off as an item entity (`eject`); a block whose face
towards the belt is full holds it 0.45 blocks short, and the stacks behind queue up (`Ending.BLOCKED`). An empty hand
takes the stacks off the block clicked. Breaking a block drops its Belt Connector, a shaft where it had a pulley, and
everything the belt carried; the rest of the belt goes too, leaving shafts at the pulleys. Rules and tests are in
`native/kinetics/src/belt.rs`, the hooks in `native/api/src/belt.rs`.

A belt ending at another belt, a depot, a basin or an upright saw hands its items on (`Ending.INSERT`), a press
over a belt stops each item and presses it one at a time, and casings and dye can be put on (Belts II, below).

Not yet: slopes, vertical and sideways belts, funnels, tunnels, fans over the belt, the texture scroll, dye and
the andesite casing drawn (dye is saved but not drawn; every casing draws as brass), carrying players and mobs, and
upright items. The rest of a broken belt goes one block a tick, not
all at once, and only a player's break drops its items (an explosion loses them). The first shaft is remembered per
player rather than on the item, with no outline shown.

### Machines and blocks added as modules

Everything below was written from Create's source and passes the unit tests, the type-check against Horizon,
the content build and the native build. An in-game test on 2026-11-15 (MCP-driven, `create-dev`) found the
content mod loading with no errors and every module below doing its main job: encasing, clutch and gearshift
on redstone, chain drives and the adjustable gearshift's ratio, the gauges, milling, crushing, fan pushing,
pulling and washing, basin and mixer (unheated and heated brass), burner fuel and Blaze Cakes, ore drops and
the building blocks. Not tested yet: mobs in crushing wheels, the fan on players, the nozzle's air, the
stressometer under load, food, zinc generation. A second test the same day found wave 2 working: tanks and
their windows, pipes and the pump, the drill, both saws, the press over a basin, belt and depot, belt hand-off
and hoppers on a depot, and the Blaze Burner's head in every lit state. Fixed after it, not yet re-tested: a
bucket on a tank also poured water beside it or filled twice, and the saw had no log recipes. Not tested yet:
chutes, funnels and steam (wave 3). Each module's own file
(`native/api/src/machines/<name>.rs`, `tools/content/<name>.py`) lists its known differences from Create at
the top. [MODULES.md](MODULES.md) explains the layout.

| Module | What it adds |
|---|---|
| Transmission | Andesite and Brass Encased Shafts and Cogwheels (casing on a shaft, wrench to toggle ends), Clutch, Gearshift, Encased Chain Drive, Adjustable Chain Gearshift, Speedometer, Stressometer |
| Millstone | Mills items with Create's 47 milling recipes; hoppers in and out, right-click to collect |
| Crushing Wheels | Two wheels make a controller that crushes items (55 crushing recipes, milling as fallback) and hurts mobs |
| Encased Fan, Nozzle | Air current that pushes and pulls items, mobs and players; washing and haunting (34 recipes) with Create's catalysts; damage and effects on entities |
| Basin, Mechanical Mixer | Basin with item slots, fluid tanks and spout output; mixer with Create's mixing recipes and heat requirements |
| Blaze Burner | Fuel, Blaze Cakes, heat levels, capturing a Blaze, the lit burner |
| Materials | 44 items (ingots, sheets, nuggets, crushed ores, foods, Precision Mechanism...), zinc ore with world generation, storage blocks, the five casings |
| Fluid Tank, Pipes, Pump | Tanks up to 3 wide and 32 high with Create's windows, the Creative Fluid Tank, buckets and bottles; fluid and glass pipes; a pump moving fluid through branching pipes (it pulls behind and pushes in front, flipped with the wrench, as in Create); honey and chocolate as fluids with buckets |
| Mechanical Drill | Breaks the block in front at Create's speed for its hardness, with the crack shown; hurts entities |
| Mechanical Saw | Sideways: breaks blocks and fells whole trees (Create's `TreeCutter`). Upright: cuts items dropped on it or carried in by belts with `create:cutting` and stonecutting recipes, and hands results on |
| Belts II | Compacting with the press over a basin; the press over a belt and a depot; belt hand-off; depot output slots and hoppers; belt casings and dye |
| Chutes, Funnels | Chute and Smart Chute (items fall, fans push them up, hoppers and belts feed them); Andesite and Brass Funnels taking from and putting into the block behind, and on belts and depots as belt funnels; filters set by sneak-clicking with an item |
| Steam | The boiler on a Fluid Tank, heated by Blaze Burners and fed water by pipes, with its gauge; the Steam Engine turning a Powered Shaft; the Steam Whistle and its extensions, pitched by size |
| Building blocks | 364 blocks: metal ladders, bars, scaffolding and doors, copper shingles and tiles, glass and windows, 14 stone types in every pattern |

Broad gaps across these modules:

- No vanilla crafting, smelting, smoking or blasting recipe can be read by a mod, so the fan doesn't smoke
  or blast items and the mixer doesn't do shapeless crafting.
- Copper shingles and tiles don't weather or wax, metal scaffolding has no collision, sliding doors swing
  like vanilla doors, and no connected textures are drawn.
- Redstone and neighbour changes are polled each tick rather than heard.
- Vanilla stonecutting recipes are baked into the saw's data (678 of them), and block hardness for the drill is a
  baked table: Horizon gives a mod neither. The wood cutting and concrete washing recipes Create writes at load
  time (`RuntimeDataGenerator`) are baked for vanilla's items.
- Honey and chocolate flow and pour, but Horizon draws a mod fluid as its template (water), and an empty bucket
  didn't pick them back up in the test.
- A mod can't list another block's slots, so chutes and funnels read a container's saved items; the Smart Chute
  and Brass Funnel have no filter slot (sneak-click with an item instead).

### Not done, and known differences from Create

- Tunnels, deployers, the mechanical crafter, open pipe ends and valves,
  contraptions, trains, logistics, schematics and Ponder.
- The Creative Motor's speed is set with the Wrench, not Create's scroll-value box.
- A network only includes loaded, ticking blocks: the part of a line in an unloaded chunk drops out.
- A speed change takes one tick to reach each block, and is not eased.
- The vertical gearbox item, waterlogging and goggles are missing.

All seven kinetic blocks rendered in `create-dev` on the native build. The Creative Motor, Hand Crank and
Water Wheel each drove a connected shaft in the in-game MCP test. See [HANDOFF.md](HANDOFF.md) for the results.

## Layout

```
content/              Horizon declarations, recipes, loot, and item definitions
native/
  horizon-mixin.toml  the native bundle
  kinetics/           create-kinetics: the rotation rules, no game dependency, unit-tested
  api/                the hooks that put create-kinetics in the game
tools/
  gen_content.py      writes Horizon declarations and item definitions
  build_content.py    imports Create artwork from a local JAR and builds create.hmod
  check_native.sh     type-checks native/ against a local Horizon checkout
```

## Build and test

The rules, in seconds:

```bash
cargo test --manifest-path native/kinetics/Cargo.toml
```

The hooks against Horizon's real API (needs a Horizon checkout beside this folder, or its path as an argument;
the first run compiles Horizon's client library):

```bash
bash tools/check_native.sh
```

Build the content mod with Python 3.11 or newer, `cargo-horizon-mod`, and a local copy of
Create 6.0.10 for Minecraft 1.21.1 (NeoForge). On this machine the JAR is already in
Horizon’s `create-resources` directory:

```powershell
python tools/build_content.py "$env:APPDATA/horizon/horizon-mods/create-resources/create-1.21.1-6.0.10.jar"
Copy-Item content/target/horizon-mod/create.hmod "$env:APPDATA/horizon/instances/create-dev/horizon-mods/"
```

The builder uses `cargo-horizon-mod` from PATH or the local build under
`target/horizon-tools/debug/`. Use `--tool` to supply its path explicitly.

Build `native/` for the same instance with
`horizon-launcher native-build native --instance <instance> --trust-native-code --profile dev`. The first native build downloads
Rust and takes 20 to 30 minutes. Use a disposable instance: a world opened without the mod loses its blocks.

## Licence and credit

Code here is MIT. Create's gameplay rules are ported from Create's MIT-licensed source, credit to the Creators of
Create. Create’s original artwork is All Rights Reserved. The build reads models and textures from the local JAR
and packages them into a local `create.hmod`. Keep that generated package local.
