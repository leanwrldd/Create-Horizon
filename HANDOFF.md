# Handoff: Create for Horizon (Rust rewrite)

Written 2026-11-19, at `main` 28f890a. Read this first, then `README.md`, `MODULES.md` and `MIGRATION.md`. Then load
the `minecraft-horizon-modding` skill (`~/.claude/skills/minecraft-horizon-modding/SKILL.md`), which holds the API
notes, the play-testing recipes and every gotcha found so far.

## Goal

Rewrite the Minecraft mod Create (6.0.10) in Rust for Horizon, the Rust reimplementation of Minecraft Java
Edition 26.3 (`github.com/horizon-rust/horizon`). It's a fresh rewrite, not a fork of Delete228's mixin-based
port. Every value, recipe and rule is ported from Create's own source, never from memory.

## Where it stands

**Breadth:** 214 of 246 Create blocks and items (87%), counted by `python tools/progress.py`.

| Area | Done | | Area | Done |
|---|---:|---|---|---:|
| Kinetics | 52 / 57 | | Contraptions | 38 / 42 |
| Materials | 19 / 20 | | Schematics | 5 / 5 |
| Logistics | 31 / 52 | | Curiosities | 12 / 13 |
| Fluids | 18 / 18 | | Palettes | 39 / 39 |

**Code:**
- **Rules:** `mod/src/kinetics` is about 76,000 lines of pure Rust with 1,560 unit tests.
- **Glue:** `mod/src/` is about 75,000 lines, with 138 machine modules in `mod/src/machines`. It runs on Horizon's new
  mod API. Older glue goes through the compatibility layer in `mod/src/compat`; new code is written on the new API
  directly.

**The mod API migration is done.** The mod builds on the official Nightly 2026-11-18 (Horizon `0b14f8a`) and its
published SDK.
- **Parts:** 607 block Parts, 181 block entity Parts and 655 ItemParts.
- **Hooks:** 70 of the 75 old hooks are wired through `mod/src/compat`.
- **Old game calls:** 13 of 147 still return a neutral value (`MIGRATION.md` lists them).

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

**Known bugs:** see the Port Board.
- A Depot under a running Press twice lost its whole stack, with no log line.
- The Clipboard keeps no pages, because it writes through the neutral `carried_stack`.
- Packages loop on the first Chain Conveyor wheel.

**Left:**
- package logistics part 2: Stock Link, Stock Ticker, Redstone Requester, Factory Gauge, Table Cloths, Shopping List,
  Item Hatch. The rules are merged; the glue isn't;
- the Mechanical Roller, seat riders on moving structures, and clicking blocks on a structure;
- minecart contraptions;
- the 13 neutral game calls, including the Toolbox menu;
- vanilla recipes through `recipe_for` (fan smoking and blasting, shapeless mixing, the blueprint);
- neighbour events instead of polling;
- a route for other testers to get Create's art: a pack built from each tester's own Create jar;
- trains, saved for last on purpose.

## Building and testing

- **Build:** `powershell -File tools/native_build.ps1`. It stages the package (Create's art comes from the user's
  jar), builds against the nightly and installs into the `create-new` instance.
  - It takes about 20 to 25 minutes, so run it in a visible window. It writes `target\native-build.exit`, and the
    launcher's log is `%APPDATA%\horizon-launcher\native\build.log`.
  - The old home-built developer version and SDK (`developer-6888…`, `C:\Users\latif\Claude\Projects\horizon-sdk`,
    rebuilt by `tools/dev_horizon_build.ps1`) are a fallback through `-Version` and `-Sdk`.
- **Check:** `bash tools/check_module.sh all` runs the rules tests and type-checks the whole mod against
  `C:\Users\latif\Claude\Projects\horizon-latest` (checked out at `0b14f8a`). It takes about 20 seconds when warm.
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
| This project | `C:\Users\latif\Claude\Projects\Create-Horizon` (`origin`: `github.com/leanwrldd/Create-Horizon`, branch `main`) |
| Create's Java source (sparse, `mc1.21.1/dev`) | `C:\Users\latif\Claude\Projects\create-java-src` |
| Create's jar (models, textures, lang; never committed) | `%APPDATA%\horizon\horizon-mods\create-resources\create-1.21.1-6.0.10.jar` |
| Horizon source we build against (`0b14f8a`, read-only) | `C:\Users\latif\Claude\Projects\horizon-latest` |
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

1. Play-test the rider shift (`collision-disassembly-shift`), plus the gantry and moving stairs.
2. Track down the Depot that empties under the Press, and fix the Clipboard (item writes from `inventory_tick`).
3. Package logistics part 2 glue.
4. Re-test the rows marked "code" on the Port Board, about 25 of them.
5. The tester art route, if the user wants testers.
