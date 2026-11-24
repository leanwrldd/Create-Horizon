# Handoff: Create for Horizon (Rust rewrite)

Written 2026-11-19 at `main` 28f890a; brought up to date 2026-11-22 at `main` 75e6f22 after an overnight run of agent
waves (summary under "Overnight 2026-11-20/21" below). Read this first, then `README.md`, `MODULES.md` and `MIGRATION.md`. Then load
the `minecraft-horizon-modding` skill (`~/.claude/skills/minecraft-horizon-modding/SKILL.md`), which holds the API
notes, the play-testing recipes and every gotcha found so far.

## Goal

Rewrite the Minecraft mod Create (6.0.10) in Rust for Horizon, the Rust reimplementation of Minecraft Java
Edition 26.3 (`github.com/horizon-rust/horizon`). It's a fresh rewrite, not a fork of Delete228's mixin-based
port. Every value, recipe and rule is ported from Create's own source, never from memory.

## Where it stands

**Breadth:** 225 of 246 Create blocks and items (91%), counted by `python tools/progress.py`. What's missing is
trains, minecart contraptions (blocked on Horizon) and a few list fields.

| Area | Done | | Area | Done |
|---|---:|---|---|---:|
| Kinetics | 52 / 57 | | Contraptions | 39 / 42 |
| Materials | 19 / 20 | | Schematics | 5 / 5 |
| Logistics | 41 / 52 | | Curiosities | 12 / 13 |
| Fluids | 18 / 18 | | Palettes | 39 / 39 |

**Code:**
- **Rules:** `mod/src/kinetics` is about 79,000 lines of pure Rust with 1,647 unit tests.
- **Glue:** `mod/src/` is about 81,000 lines, with 147 machine modules in `mod/src/machines`. It runs on Horizon's new
  mod API. Older glue goes through the compatibility layer in `mod/src/compat`; new code is written on the new API
  directly.

**The mod API migration is done.** The mod builds on the official `nightly-2026-11-22-f1bb66f` (Horizon `f1bb66f`, moved from
`0b14f8a` on 2026-11-24 when the 2026-11-18 nightly was re-published as `dd67b76` and `0b14f8a` stopped being
downloadable) and its published SDK. The pin is exact: move `[native] horizon`, the installers, the README and
`tools/native_build.ps1` together, and check out `horizon-latest` at the same commit.

**The launcher must be 0.2.82 or newer.** Older launchers fail the game patch's crate (`horizon_patched`) with
E0603 on `TryIntoApi` (fixed in horizon-rust/horizon#18509; the report this session filed, #21119, was a false alarm
from launcher 0.2.71 and is closed). With launcher 0.2.108 the mod built on `f1bb66f` on 2026-11-24, patch applied, and
`v0.1.0-test3` (mod 0.1.2) is published; test1 and test2 are marked as no longer installing.
- **Parts:** 607 block Parts, 181 block entity Parts and 655 ItemParts.
- **Hooks:** 70 of the 75 old hooks are wired through `mod/src/compat`.
- **Old game calls:** 3 of 148 still return a neutral value (`block_states`, `data`, `held`), and no module calls
  them. `fluid_tanks` on a shared host can't see vanilla or other-mod tanks (`MIGRATION.md`).

**Played in game on the nightly (`create-new`):**
- **Processing:** the Press making iron sheets on a Depot, the Millstone making flour, and the Mixer making
  andesite alloy in a Basin.
- **Item handling:** hoppers and chutes into Depots.
- **Goggles:** the overlay, with real text.
- **Wrench:** rotation, and sneak pickup.
- **Cog and shaft placement helpers:** snapping and the ghost.
- **Collision on moving structures:** you ride rope pulleys, pistons, bearings, windmills and elevators, and mobs and
  items get pushed and carried.
- **Speed:** frames are about 7 ms (140 fps).

**Waiting to be played:** riders moving with a structure when it snaps to the grid (`28f890a`). The test list is on
the Port Board row `collision-disassembly-shift`.

**Fixed after the 2026-11-22 smoke test (`target/smoke/smoke-1122.md`), not yet re-tested in game:** the Press not
pressing Depot stacks (`aed018f`: my `6827498` write order undid slot writes), invisible moving structures (`020cea5`:
the draw cache was keyed on the compat revision, always 0), shafts rolled against motors and gearboxes (`83f73f6`), the
Toolbox view bouncing items (`8659866`), the stock network seeing 0 links (`ae10b15`), the lava fan destroying items on a
failed lookup (`cc09448`). The mod's `eprintln!` warnings don't reach `latest.log`.

**Known bugs:** see the Port Board.
- A Depot under a running Press twice lost its whole stack, with no log line. Not found by reading the code;
  `change_mod_block_entity` now writes slots before data and stops on a refused slot write (6827498), which may or
  may not be it. Needs a repro in game: watch `data get block` on the depot every few seconds.
- Packages looping on the first Chain Conveyor wheel: not reproduced by a glue-level two-wheel test. Most likely
  the Frogport on wheel B never attached (placed by `setblock`): wrench-click it, then check A's routing table.

**Overnight 2026-11-20/21** (all type-checked and unit-tested; **none of it built or played yet**):
- **Compat:** `carried_stack`/`carried_items` (Clipboard pages, package addresses, wand), SNBT block entity reads and
  writes, fluid insert/extract, `use_as_player` (Deployer), screen blur, `fluid_tanks` for mod tanks, the Toolbox
  menu (a generic 9x4 chest view, clicks handled in `rules/menu_input.rs`).
- **Vanilla recipes through `recipe_for`:** fan smoking and blasting, the crafter, shapeless mixing, press compacting,
  saw stonecutting, the blueprint, attribute filters. The baked tables are gone.
- **Package logistics part 2, all of it:** Stock Link, Stock Ticker (`/stock`), Redstone Requester (`/requester`),
  Item Hatch, Factory Gauge (`/gauge`), 19 Table Cloths and the Shopping List (`/cloth price`); packager arrivals
  fulfil promises.
- **Contraptions:** the Mechanical Roller (value-box filter and mode, three paving modes), seat riders carried and put
  back on set-down, the piston restarts its actors on reversal, bells, campfire smoke, dispensers and droppers as
  actors (their vanilla sound ids were written from memory: check them).
- **Neighbour events:** about 20 blocks read redstone on `neighbor_changed` (`mod/src/signals.rs`,
  `rules/signals.rs`) with a 20-tick heartbeat; diodes, observers and the display link still poll, on purpose.
- **Chain Conveyor:** Frogports jump to the opposite chain when a wheel reverses.

**Left:**
- play-testing everything above (test plans are in the merge commits' agents' notes and on the board rows);
- trains: the rules are merged in three parts (`rules/trains/`) and the glue in three: part 1 (`a1e4d19`, track), part 2 (`4c953e7`,
  station, signal, observer, bogeys, Train Casing, schedule item). **Part 3 (merged `d46b3b5`, unplayed; a moving carriage sends only its
  pose, `d4a15f4`; seats on a carriage carry riders, `ab1473a`):** assembly and
  disassembly at the station, `Railways::tick` from the level tick, trains saved in the level attachment `create:trains`, each carriage
  drawn by an anchor block entity in an invisible block that follows it (`create:carriage_anchor`, why in `rules/trains/README.md`),
  bogeys with turning wheels, carriages solid (`solid::Sample::Oriented`), driving with W/S/A/D/Space/Sneak (a click on `create:controls` or
  `/train drive`), the station's schedule slot and `/train`. To check in game: assemble a two-bogey carriage with Train Controls on a
  straight station track (`/station assembly`, Train Casing on the track, Super Glue, `/station assemble`), watch the carriage appear on
  the next tick, `/train drive`, drive, `/train info`, stop at a station and `/station disassemble`, save and reload with a train standing
  still and one moving, ride a slope or a curve (pose and bogey angles), a Schedule in the station slot, and the frame time with a long
  train (the anchor's update tag carries the structure every other tick while moving). **Part 4 (branch `worktree-agent-a285f9aef99a726ef`, unmerged, unplayed):** carriage cargo and fuel (`train_cargo.rs`), mobs seated on carriage seats as conductors, handing a Schedule to one or to a Blaze Burner (`train_riders.rs`), the Train Doors opening at a station by its door control (`train_doors.rs`, `/station doors`); the test list is in `rules/trains/README.md` ("Glue part 4"). Left for later: `rules/trains/README.md`'s "Next".
  Catnip's `intersect_ranged` and `intersect_sphere` are reconstructed, unverified (see its README);
- merged 2026-11-23, unplayed: Deployer schematic printing on structures (`fc27597`), Factory Gauge links to
  Redstone and Display Links (`42abf2c`), structure clicks cancelled on the client, real reach, saved disabled
  actors and elevator floor choice (`22f24bd`); a chest, barrel or shulker box on a structure opens a chest view of
  its slots (`0c1b52c`);
- Ponder: bearings, glue, belt items, pulleys, deployers, arms and particles animate (0d8653f). Then: `modifyBlockEntityNBT`
  (the `put*`/`writeEnum`/`saveOptional` calls become an `nbt` op the player merges into the drawn block entity, so
  every renderer that reads the key Create saves shows it), `setFilterData`, Harvester/Roller `setAnimatedSpeed`,
  `flapFunnel` (and the flap `createItemOnBelt` ends with), `incrementBlockBreakingProgress` (crack texture over the
  block, ten calls break it, with crumbs), fluid particles (honey, chocolate), item entity discards, and the particle
  constants now follow Horizon's ports of the vanilla classes. Then (unplayed): fluid in Fluid Tanks
  (`rules/ponder_fluid.rs`; the tank blocks come from the scene's structure file, `ponder_assets.tank_data`),
  `modifyBlock` that reads the old state (an `update` op `ponder_assets.expand_properties` resolves against the
  structure), `toggleControls`, the filter and scroll input boxes (read from the Ponder library's bytecode:
  `tools` has no disassembler, the constants are in `highlight_box`), nixie text, the postbox flag, the controls'
  `Disabled`, a press's item particles, the display link's bulb pulse and the fluid pipe's `getAxisState`;
  `propagatePipeChange` is counted as having nothing to draw (the pipes' flow isn't drawn). `python -m unittest
  tools/test_ponder.py` checks the generator's pure parts. Left (`ponder_storyboards.py` prints what it skips):
  Mechanical Crafter inventories, `setCraftingResult` and `connectCrafterInvs` (needs `rules::crafter_grid::World`
  run inside the replay), display board text and dye (item names come from the client's language files),
  `VisualizedItems` on a basin and a depot's `HeldItem` (nothing draws them), brass tunnel modes and filters (not
  drawn), item drain, basin, spout and hose pulley fluid, parrots, carts, `hideElement` (parrots and carts only),
  `chaseBoundingBoxOutline`, trains;
- minecart contraptions (blocked: nothing can draw a structure on a vanilla minecart);
- merged 2026-11-23, unplayed: clicking levers, doors, trapdoors, gates, seats, Deployers and Controls on a moving
  structure (`006778e`); items carrying frequencies and orders, the gauge click picking the panel, arrows, `/gauge move`
  (`01b5a40`); Mixer brewing (`507bcfc`; the brewing recipe JSON field names are a guess), pumps and pipes joining
  other mods' tank block entities (`b67432a`; cauldrons stay unconnected, as in Create 6.0.10);
- the Boiler's particle flood (Port Board `boiler-particles`): needs an in-game look first.

Testers get Create's art in the release files (`v0.1.0-test3`), with the permission of Create's developers, so the
old plan of a pack built from each tester's own jar is dropped.

## Building and testing

- **Build:** `powershell -File tools/native_build.ps1`. It stages the package (Create's art comes from the user's
  jar), builds against the nightly and installs into the `create-new` instance.
  - It takes about 20 to 25 minutes, so run it in a visible window. It writes `target\native-build.exit`, and the
    launcher's log is `%APPDATA%\horizon-launcher\native\build.log`.
  - The old home-built developer version and SDK (`developer-6888…`, `C:\Users\latif\Claude\Projects\horizon-sdk`,
    rebuilt by `tools/dev_horizon_build.ps1`) are a fallback through `-Version` and `-Sdk`.
- **Check:** `bash tools/check_module.sh all` runs the rules tests and type-checks the whole mod against
  `C:\Users\latif\Claude\Projects\horizon-latest` (checked out at `f1bb66f`). It takes about 20 seconds when warm.
- **Lock file:** `mod/Cargo.lock` is committed. Remake it with the launcher's `native-lock`, then
  `python tools/pin_lock.py`.
- **Game patches:** `[[native.mixin]]` rows in `mod/horizon-mod.toml`, with helpers in `mod/src/mixins/`. Today's
  only one is `local_player_solid`, a HEAD patch on `Game::remote_send_state` that collides the local player with
  moving structures.
- **Launch for testing:** use the horizon MCP `play` tool on `create-new` with control on, then `game_wait_ready`.
  The user often plays in the same window. Before driving the player, count down in chat (`tellraw`, 5 seconds), and
  say in chat when they can move again.

## Where things are

| What | Path |
|---|---|
| This project | `C:\Projects\Create-Horizon` (moved here 2026-11-22; `origin`: `github.com/leanwrldd/Create-Horizon`, branch `main`) |
| Create's Java source (sparse, `mc1.21.1/dev`) | `C:\Users\latif\Claude\Projects\create-java-src` |
| Create's jar (models, textures, lang; never committed) | `%APPDATA%\horizon\horizon-mods\create-resources\create-1.21.1-6.0.10.jar` |
| Horizon source we build against (`f1bb66f`, read-only) | `C:\Users\latif\Claude\Projects\horizon-latest` |
| Ola's port (MIT, design ideas; uses Parts) | `C:\Users\latif\Claude\Projects\create-horizon-ola` |
| Delete228's port (MIT, reference) | `C:\Users\latif\Claude\Projects\create-horizon-ref` |
| Launcher (not on PATH) | `%LOCALAPPDATA%\Programs\Horizon Launcher\horizon-launcher.exe` |
| Test instance | `%APPDATA%\horizon\instances\create-new` (log: `logs\latest.log`) |
| Play-test screenshots (not committed) | `target/smoke/` |
| Port Board (task list; rows in its database) | `https://claude.ai/artifact/YGFoFYxhHFMELsUzYxt35D` |

## Rules

- **Never build or test in the `horizon` instance or `create-dev`.** `horizon` holds the user's real worlds and
  Delete228's `create` mod. Use `create-new`.
- **Don't quit or kill a game you didn't launch,** and don't native-build into an instance the user is playing in.
- **Commits** are authored as leanwrldd, with no Co-Authored-By or other AI attribution lines.
- **Long jobs** (10 minutes or more) run in a visible PowerShell window. Don't poll them; the user reports back.
- **No unprompted browser or preview checks.**
- **Create's art is never committed.** It's imported from the user's own jar at build time.
- **Thin glue:** every decision, number and table goes in `mod/src/kinetics/src/rules/` with unit tests. Glue only
  reads the world, calls the rules and applies the result.
- **Agents:** Sonnet on medium effort, each in its own worktree (`.claude/worktrees/<name>`). In-game tests go to an
  agent, with the chat countdown.

## Merging an agent's branch

1. `git merge --no-ff <branch>` on `main`.
2. Regenerate: `python tools/gen_modules.py`, `gen_forward.py`, `gen_parts.py`, `gen_migration.py`. Keep only real
   changes; line-ending noise is common.
3. `bash tools/check_module.sh all`.
4. `python tools/progress.py`, commit, push.
5. Update the Port Board rows (writes pinned with `if_version`), then remove the worktree and branch.

Don't run `tools/gen_content.py` over all of `content/`. The script has moved on to a `properties` shape format that
the committed files don't use yet, so a full run rewrites about 110 files that have never been tried in game. Edit
single files instead, and keep the script in step.

## Next steps

1. A native build of `main`, then a broad smoke test: launch, `latest.log` free of panics, frame time.
2. Play-test the overnight work: the Clipboard, the Toolbox menu, fluids (pump, basin, tank), the fan and mixer with
   vanilla recipes, a Stock Ticker order end to end, seat riders, the Roller, redstone on gearshifts and funnels.
3. Play-test the rider shift (`collision-disassembly-shift`), plus the gantry and moving stairs.
4. The Depot under the Press and the Chain Conveyor, with the repro notes above.
5. Trains, part by part.
