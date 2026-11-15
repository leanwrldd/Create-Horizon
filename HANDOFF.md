# Handoff: Create for Horizon (Rust rewrite)

Written 2026-11-14. Read this, then `README.md`, then load the `minecraft-horizon-modding` skill
(`~/.claude/skills/minecraft-horizon-modding/SKILL.md`), which holds the API notes and every gotcha found so far.

## Goal

Rewrite the Minecraft mod Create in Rust for Horizon (the Rust reimplementation of Minecraft Java Edition 26.3,
`github.com/horizon-rust/horizon`). The user chose a **fresh rewrite on Horizon's trait API** over forking
Delete228's existing port (`github.com/Delete228/create-horizon`, MIT, far more complete, built on pinned mixins).
That port is cloned locally as a reference only.

## State

**Done and checked**

- `native/kinetics`: rotation rules, the per-tick network table, and the generators
  (`generators.rs`: Hand Crank, Water Wheel). 24 unit tests pass.
- `native/api`: hook glue (block entities, ticking, sync, drawing, placement, wrench, crank right-click).
  Type-checks against Horizon.
- `content/`: block and item declarations, recipes, and loot. `tools/build_content.py <create jar>` builds a
  local package with the models and textures from the player's own Create 6.0.10 JAR (hash-checked; the
  user's copy is `%APPDATA%\horizon\horizon-mods\create-resources\create-1.21.1-6.0.10.jar`). Create's
  assets never go in the source tree, and the built `.hmod` is for local use only.
- **Kinetics confirmed working in game by the user (2026-11-14)**: shaft, cogwheels, gearbox, creative motor,
  wrench.

**Every ported value is checked against Create's source**, at `C:\Users\latif\Claude\Projects\create-java-src`
(sparse clone of `mc1.21.1/dev` at `4148fd8`: Java code and generated data, no assets). Catnip (`VecHelper`,
`VoxelShaper`) is private on GitHub; read it from the Ponder sources jar on `maven.createmod.net`
(`net/createmod/ponder/ponder-common/1.0.85+mc1.21.1`). Vanilla block numbers come from Horizon's prepared data,
`%APPDATA%\horizon\.horizon-cache\mod-content\<hash>\gameplay-facts.json`. Cite the Create class in comments.

**Hand Crank and Water Wheel verified in `create-dev` through Horizon MCP (2026-11-14).**

- Crank and connected shaft reached 32 RPM on right-click and -32 RPM while sneaking, then returned to zero.
  The survival food cost was not checked in game.
- Wheel and connected shaft reached 8 RPM in either direction with a bubble column and 8 RPM with falling water.
  Still water delivered zero RPM.
- A game screenshot showed all seven kinetic block types placed and visible in the native build.
- Wrench pickup was verified through MCP after rebuilding the content loot tables. A shaft disappeared
  with no item in Creative, and a shaft and Hand Crank each returned one item in Survival. With the
  wrench put away, sneak-right-click still drove the crank's shaft at -32 RPM.

Player placement of a crank or wheel follows nearby shaft ends. Sneaking or competing shaft axes falls
back to the clicked face for the crank and the look axis for the wheel. The shaft preference runs in the
player placement hook, so other placement paths use the fallback. The selection rules are unit-tested.
Placement in the game needs a check on the next native build.

**Known differences from Create, not done yet:**
- A crank doesn't break when the block behind it goes (`HandCrankBlock.canSurvive`).
- The crank handle doesn't ease its spin (`HandCrankBlockEntity.independentAngle`).
- Right-clicking the Hand Crank with a placeable block can briefly show that block next to it.
  Horizon predicts ordinary placement on the client, while the crank consumes the click in the
  server-side `right_click_block` hook. The modding fork already handles this (`arismoko/horizon#171`).
  For builds from the official launcher, see `horizon-rust/horizon#16451` and draft PR `#16455`.
- The wheel checks the water every tick; Create checks every 60 ticks and on neighbour changes.
- No wheel materials (right-click with planks), no waterlogging, no Large Water Wheel.
- Recipes: the Gearbox uses Andesite Alloy for the missing Andesite Casing; the Wrench uses gold ingots and a
  stick for Create's `c:plates/gold` and `c:rods/wooden` tags, which nothing provides here.
  (Fixed 2026-11-15: the casing and the tags exist now, and the recipes use them.)

**Modules (2026-11-15).** Machines past the first slices are modules, one set of files each, written by
Sonnet subagents in parallel: see `MODULES.md` for the layout, the shared helpers (`kinetic.rs`, `recipes.rs`,
`util.rs`), the generated hook forwarder (`forward.rs` from `tools/forward_hooks.txt`) and the per-owner checks
(`tools/check_module.sh <owner>`, `build_content.py --owner <owner>`, with `tools/wip.txt` naming who writes
what). Done this way so far: transmission (encased shafts and cogwheels, clutch, gearshift, chain drives,
gauges), millstone, crushing wheels, encased fan and nozzle, basin, mixer, blaze burner, materials, building
blocks (wave 1); fluid tanks, pipes and pump, drill, saw, and Belts II (press over basin, belt and depot; belt
hand-off; casings) (wave 2). Each passes the unit tests, the type-check, the content build and the native build.
Wave 1 was tested in game through MCP on 2026-11-15 and worked (README has the details; the Blaze Burner's head
didn't show). **Wave 2 has not been tried in game.** Each module file lists its known differences.

**Not started:** funnels, tunnels, chutes, deployer, crafter, steam engine, sloped belts, contraptions, trains,
logistics, schematics, goggles, Ponder.

**Agent waves:** the user wants at most 3 subagents at a time; after each wave, build, redraw the progress
(`python tools/progress.py`), commit and push.

**Git:** `origin` is `github.com/leanwrldd/Create-Horizon`, branch `main`. Author commits as leanwrldd with no
AI attribution (the user's global rule).

## Where things are

| What | Path |
|---|---|
| This project | `C:\Users\latif\Claude\Projects\Create-Horizon` |
| Horizon source, shallow clone at `0de8932` (reference; the instance builds on `9f42064`) | `C:\Users\latif\Claude\Projects\horizon-src` |
| Delete228's port (reference, MIT) | `C:\Users\latif\Claude\Projects\create-horizon-ref` |
| Create's Java source (sparse, `mc1.21.1/dev` at `4148fd8`) | `C:\Users\latif\Claude\Projects\create-java-src` |
| Launcher | `%LOCALAPPDATA%\Programs\Horizon Launcher\horizon-launcher.exe` (not on PATH) |
| Test instance `create-dev` | `%APPDATA%\horizon\instances\create-dev` |
| Game log | `%APPDATA%\horizon\instances\create-dev\logs\latest.log` |
| Native build log | `%APPDATA%\horizon-launcher\native\build.log` |
| Content tool, built from the clone | `target\horizon-tools\debug\cargo-horizon-mod.exe` |

**Never build or test in the user's main `horizon` instance.** It holds their worlds and has Delete228's
`create.hnmod` installed, which uses the same mod id (`create`).

## The loop

```bash
# 1. Rules (seconds)
cargo test --manifest-path native/kinetics/Cargo.toml --target-dir target

# 2. Hooks against Horizon's API (about 3 minutes cold, seconds warm)
bash tools/check_native.sh

# 3. Content: edit tools/gen_content.py for declarations
python tools/build_content.py "$APPDATA/horizon/horizon-mods/create-resources/create-1.21.1-6.0.10.jar"
cp content/target/horizon-mod/create.hmod "$APPDATA/horizon/instances/create-dev/horizon-mods/"

# 4. Native build (defaults to dev; start long builds in a visible terminal)
powershell -ExecutionPolicy Bypass -File tools/native_build.ps1

# 5. Play (use the native version assigned to create-dev in the launcher's instances.json)
horizon-launcher play --instance create-dev --version <native-version> --world create-test --creative --arg=--control
```

Content-only changes need steps 3 and 5. Rust changes need 1, 2, 4 and 5.

After every launch, grep the game log for `content mods not loaded`: one bad declaration drops the whole content mod.
If icons appear but every placed Create block is invisible, check which executable the launcher started.
The stock nightly loads `create.hmod` and its icons, but only the native build draws the placed blocks.
On Windows, the launcher puts `source.json` one directory above the running `game/` folder. It records
which installed version supplied the executable.

## How it works

- Each kinetic block has a block entity. Its server tick (`block_entity_tick`) reports the block to `Networks`
  (a table inside the mod, behind a `Mutex`) and reads back its speed.
- `server_tick` (start phase) prunes blocks that stopped ticking and, if anything changed, re-solves every network
  with `create_kinetics::solve`.
- A changed speed is written to the entity's NBT (`Speed`, a float) and synced; `block_entity_update_tag` sends it.
- The client draws in `block_entity_submit` using `BlockEntityDraw::spin` plus `keep`, so geometry is rebuilt only
  when the speed changes.
- A jammed block (two speeds at once, opposing generators, over 256 RPM) is destroyed from its own tick.

## Problems hit this session, all fixed

| Symptom | Cause |
|---|---|
| Type-check failed inside `horizon-client-render` (`disk_cache` not found) | A crate outside Horizon's workspace doesn't inherit its `[patch.crates-io]`; `check_native.sh` now passes the three patches |
| Launcher: "create was left out: made for another Horizon version" | A `Cargo.lock` left in `native/kinetics` by `cargo test`; `native_build.ps1` deletes it first |
| Build window: path "not recognized" | Space in "Horizon Launcher" lost its quotes through `Start-Process -Command`; use the `.ps1` with `-File` |
| Game log: `sounds minecraft:wood is not a block` | A block's `sounds` names a block; now `minecraft:oak_planks` |

## Constraints

- **Licence:** Create's code is MIT. The original art is All Rights Reserved. Keep asset imports in local
  generated packages.
- **Trait API only:** use named hook points so the mod isn't pinned to a Horizon commit. Get exact signatures from
  the skill's `apigen` helper, not from memory.
- **The trait crate may only use crates in Horizon's `Cargo.lock`.**
- **User rules:** no browser/preview verification unless asked; jobs of 10 minutes or more go in a visible window;
  commits authored as leanwrldd.

## Suggested next steps

1. Test wave 2 in game (fluids, drill, saw, Belts II): grep `logs\latest.log` for `content mods not loaded`
   first (the honey and chocolate fluid sprites are the risk), then each module's "most likely wrong" points.
2. Look at the Blaze Burner head, which didn't show in the wave 1 test.
3. Next slices: funnels and tunnels, chutes, deployer and crafter, steam engine, sloped belts.
4. Keep the skill current: add each new gotcha in the session that finds it.
