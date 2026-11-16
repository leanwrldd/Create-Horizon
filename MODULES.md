# Adding a machine

Every block family past the first slices is a **module**: a few files of its own, hooked in by file
name, so two people (or agents) can work on different machines at once without touching the same
file. A module named `millstone` is some or all of:

| File | What it holds |
|---|---|
| `native/kinetics/src/rules/millstone.rs` | The rules: numbers and state machines ported from Create. No game types, so unit tests run in seconds. `create_kinetics::rules::millstone` |
| `native/api/src/machines/millstone.rs` | The glue: a `Hooks` of its own for the module's blocks. Exports `pub fn hooks() -> Box<dyn Hooks>` |
| `tools/content/millstone.py` | The declarations: blocks, items, names, recipes, tags, loot |

More files for one module are named `millstone_<part>.rs` / `.py`. Nothing else needs editing:
`tools/gen_modules.py` writes the two `mod.rs` lists from the files that exist, and the content
generator loads every `tools/content/*.py`.

The first slices (shafts, cogwheels, gearbox, generators, depot, belt, press) predate this and live
in `native/api/src/lib.rs` (`Core`), `belt.rs`, `depot.rs`, `press.rs`, `draw.rs`,
`native/kinetics/src/{lib,table,belt,press,generators}.rs` and `tools/gen_content.py`.

## Rules for the port

- **Every number, recipe and rule comes from Create's source**, not from memory and not from the
  reference port alone: `C:\Users\latif\Claude\Projects\create-java-src` (Create `mc1.21.1/dev` at
  `4148fd8`: Java under `src/main/java/com/simibubi/create`, datagen output under
  `src/generated/resources`). Name the Create class or file in a comment beside what you port.
- **Create's ids, state property names and NBT key names are kept** (`create:millstone`, `Timer`).
- **No Create artwork in the tree.** Models, blockstates and textures are read from the player's own
  Create jar when the package is built. Name the models you need; don't copy them.
- **Only declare what works.** A block or item goes in `content/` when its behaviour is written (a
  plain material or decorative block needs none). The progress chart counts declarations.
- Known differences from Create go in a comment at the top of the module, for the README.
- **Keep the glue thin.** Horizon's official mod API changed (the arismoko API was merged upstream,
  horizon-rust/horizon#16941), so `native/api` will be rewritten for it later while
  `native/kinetics` carries over. Put every decision, state machine, number and table in the rules
  crate as plain Rust with tests; the glue only reads the world, calls the rules and applies what
  they return. A rule that needs the world takes a small trait or plain values, not `Host`.

## The glue: `native/api/src/machines/<name>.rs`

A module implements Horizon's `Hooks` trait for its own blocks and returns early for anything
else. `forward.rs` hands each hook to `Core` first and then to every module, in file-name order: a
hook returning `Flow` stops at the first `Cancel`, one returning an `Option` at the first `Some`,
any other reaches everyone.

```rust
//! Example Machine: a casing with a shaft out of its bottom that costs 4 su per RPM
//! (`ExampleBlock`, `ExampleBlockEntity`, `ExampleRenderer`).

use create_kinetics::{Dir, Kind, Node};
use horizon_client::mod_api::{
    BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent,
    BlockEntityUpdateTagEvent, Compound, Flow, Hooks, Host, UseItemOnEvent,
};

use crate::{kinetic, util};

const ID: &str = "create:example";

struct Example;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Example)
}

impl Hooks for Example {
    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == ID).then(|| (ID.to_owned(), kinetic::new_entity()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ID {
            return;
        }
        let node = Node {
            impact: 4.0,
            ..Node::passive(Kind::shafts(&[Dir::Down]))
        };
        let Some((_rpm, changed)) = kinetic::tick(host, event.pos, event.entity, node) else {
            return; // The network jammed and the block broke.
        };
        // ... the machine's own work at `_rpm`, changing `event.entity.nbt` only when a value differs ...
        if changed {
            host.block_entity_sync(event.pos);
        }
    }

    fn block_entity_update_tag(
        &self,
        event: &mut BlockEntityUpdateTagEvent<'_>,
    ) -> Option<Compound> {
        (event.entity.id == ID).then(|| kinetic::update_tag(event.entity))
    }

    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ID {
            return;
        }
        let rpm = kinetic::speed(event.entity);
        kinetic::draw_shaft_half(host, &mut *event.draw, Dir::Down, rpm);
        // Only geometry that reads no `partial`, `game_time` or neighbours may be kept.
        event.draw.keep();
    }

    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || host.block(event.pos) != ID {
            return Flow::Continue;
        }
        util::wrench(host, event.pos, |_| {})
    }
}
```

What the crate gives a module:

- `crate::kinetic`: `tick` (report to the rotation network, get the speed), `speed`, `new_entity`,
  `update_tag`, `draw_shaft`, `draw_shaft_half`, `networks`. A machine is `Kind::shafts(&[faces])`
  to its network, with `impact` (su per RPM it costs) or `generated` and `capacity`. Stress values
  are in Create's `infrastructure/config/CStress.java`.
- `crate::recipes`: Create's processing recipes read from data (`for_stack`, `all`, `roll`), as
  Create's datagen writes them. Copy the recipe JSON from `src/generated/resources/data/create/recipe/`
  into the module's `RECIPES`, leaving out the ones for other mods (`neoforge:conditions`).
- `crate::util`: NBT readers (`int`, `float`, `long`, `boolean`, `get`, `has`), item stacks as NBT
  (`stack_tag`, `stack_of`, `stacks_tag`, `stacks_of`), `merge`, `spawn_item`, `game_time`,
  `wrench`, `facing`, `dir`/`direction`.
- `press.rs`, `depot.rs` and `belt.rs` are worked examples of a processing machine, an item
  holder with a container, and a multi-block.

Exact hook and helper signatures: the generated API, already under `target/apigen/`
(`mod_api_generated.rs` has `trait Hooks` and every event struct; `mod_api_generated_host.rs` has
`trait Host` with doc comments, the best API reference there is). `grep` them; never guess.
The long-form docs are `horizon-src/docs/native-mods.md` (grep its headings) and `docs/mods.md`
(content declarations). The `minecraft-horizon-modding` skill lists every gotcha found so far:
read its Gotchas section before writing a block entity.

Only the hooks in `tools/forward_hooks.txt` reach a module. To use another, add its name there and
run `python tools/gen_forward.py`.

## The declarations: `tools/content/<name>.py`

A plugin is a Python file defining any of these names. Import helpers from `gen_content`
(`block`, `AXIS_STATE`, `FACING_STATE`, `HORIZONTAL_FACING_STATE`, `axis_shapes`, `facing_shapes`,
`ANDESITE`, `WOODEN`, map colours).

| Name | Value |
|---|---|
| `BLOCKS` | `{"millstone": block(...)}`: Horizon block declarations (`data/create/horizon/block/`). `block()` gives a glass-template block drawn by its block entity; pass `render_shape="MODEL"` when Create's own block model should sit in the chunk mesh. Any dict in Horizon's format works (other templates for doors, slabs, ores...) |
| `ITEMS` | `{"wheat_flour": {"template": "minecraft:iron_ingot", "max_stack_size": 64}}` |
| `NAMES` | `{"block.create.millstone": "Millstone"}` (Create's `en_us` names) |
| `RECIPES` | `{"milling/wheat": {...}}`: `data/create/recipe/<key>.json`. Vanilla types in Horizon's 26.3 form (`"#tag"` strings); Create's own types exactly as Create's datagen writes them |
| `TAGS` | `{"c:item/ingots/zinc": ["create:zinc_ingot"], "create:block/fan_transparent": [...]}`: `<namespace>:<registry>/<path>`, merged with other plugins' |
| `TAB` / `PALETTES_TAB` | Item ids to add to the main creative tab / the building blocks tab, in order |
| `PICKAXE_ONLY` | Set of block names mined by pickaxe only (default: axe or pickaxe) |
| `MINEABLE` | `{"name": ["axe"]}`: the `mineable/<tool>` tags for a block, replacing the default; `[]` for none |
| `LOOT` | `{"name": [pools]}` replacing the default (the block drops itself); `None` for no table. Use the `"condition": {"type": ...}` form (see the skill) |
| `MODELS` | Partial models the block entity draws (`"block/millstone/inner"`), imported from the jar; also list them in the module's `standalone_models` hook as `create:block/millstone/inner` |
| `BLOCKSTATES` | `{"name": json or function(create_blockstate_json) -> json}` when Create's blockstate file doesn't fit the declared states. Default for a `block()` declaration: Create's file with the undeclared properties (`waterlogged`, ...) dropped at their resting value; for a block on any other template (a door, a slab), Create's file as it is |
| `ITEM_MODELS` | `{"name": {...}}` replacing `assets/create/items/<name>.json` (default: Create's `item/<name>` model) |
| `FILES` | `{"data/create/...json": value}`: any other file of the package |

State property names must not collide with the template's, shapes are in blocks (0 to 1), and
`"sounds"` names a block. One bad declaration makes the game drop the whole content mod, so keep to
forms that already load (see `tools/gen_content.py` and `horizon-src/docs/mods.md`).

To look at Create's models, blockstates and lang (never copy them into the tree):

```bash
python -c "from zipfile import ZipFile; import os; z = ZipFile(os.path.expandvars(r'%APPDATA%/horizon/horizon-mods/create-resources/create-1.21.1-6.0.10.jar')); print(z.read('assets/create/blockstates/millstone.json').decode())"
```

## Checking

`tools/wip.txt` says who is writing which modules. Each owner checks a private copy that leaves
the other owners' unfinished modules out:

```bash
bash tools/check_module.sh <owner>
```

```bash
python tools/build_content.py "$APPDATA/horizon/horizon-mods/create-resources/create-1.21.1-6.0.10.jar" --owner <owner>
```

The first runs the rules' unit tests and type-checks the glue against Horizon (seconds once warm;
it waits for the build lock if another check is running). The second generates the declarations
into `target/content-check-<owner>/`, imports the assets and runs `cargo-horizon-mod build`; it
fails on a model or texture the jar doesn't have. Neither proves the game loads the declarations:
only a launch does.

With owner `all`, nothing is left out. After adding or removing a module file in the real tree run
`python tools/gen_modules.py`; `tools/native_build.ps1` does it too.
