# Migration to Horizon's new mod API

Step 1 (scaffold and compatibility layer, branch `migrate-scaffold`) and step 2 part B (the hooks, branch
`step2-hooks`). The port builds as one flat package on the developer version of Horizon (3f1843e) and keeps its 137
machine modules as they were, behind a compatibility layer. This file says what is where, what is wired, and what the
next steps have to do. Step 2 part B was type-checked only (`tools/check_module.sh all`): none of it has run in the
game yet, so "Needs testing in game" below is the list to play through first.

## Layout

| Path | What |
|---|---|
| `mod/horizon-mod.toml` | the manifest: content fields plus `[native]` (game 26.3, Horizon 0b14f8a, the official Nightly 2026-11-18 with its published SDK; horizon-mixins e59da90) |
| `mod/Cargo.toml`, `mod/Cargo.lock` | the SDK path dependency the launcher takes, serde, flate2; the lock is `horizon-launcher native-lock mod` |
| `mod/src/kinetics/` | the rules crate (was `native/kinetics`), unchanged: 1,413 tests |
| `mod/src/*.rs`, `machines/`, `ponder/` | the glue (was `native/api/src`): only its imports changed |
| `mod/src/compat/` | the old API's names, on the new one (below) |
| `mod/src/parts.rs`, `parts_gen.rs` | one `BlockPart` per block, one `BlockEntityPart` per block entity |
| `tools/stage_package.py` | assembles `target/package/` (mod/ + content with Create's art); never committed |
| `tools/native_build.ps1` | stages and builds into an instance on the developer version |
| `tools/check_module.sh` | tests the kinetics crate and type-checks the package against the checkout's mod API |

## The compatibility layer

A module used to `use horizon_client::mod_api::*`; it now says `use crate::compat::*` and otherwise stays as it was.

* `compat/old_api.rs` is **generated** (`tools/gen_compat.py`, from the old API's generated source): the 75-hook
  `Hooks` trait, its 75 event structs and the old `Host` with the 146 methods the modules call. Every `Host` default
  is a logged neutral value (`gap`).
* `compat/types.rs` holds the types the new API has no twin for, with the OLD public fields: `ItemStack` (item name,
  count, damage, components; the new one is opaque), `ModBlockEntity` (`id`, `nbt`, `items`, `data`), `BlockStateId`
  (interned from the new process-local `BlockState`), `BlockEntityDraw`, `LevelHost`, `PlacementContext`, and the
  `Native*` data types. Where the new API names a thing the same way it is re-exported (`Compound`, `Tag`, `Text`,
  `Direction`, `Flow`, handles such as `LevelRef` and `EntityRef`).
* `compat/events.rs` turns each new event into the old one, runs the old dispatcher (`lib.rs` `create()`, the
  generated `forward.rs`) and writes the answer back.
* `compat/adapter.rs` implements the old `Host` on `ServerHost` / `ClientHost`.
* The modules changed in three ways beyond imports: about forty call sites that handed the old host to a new-API
  handle now pass `host.new_host()` / `host.new_server()`; six block-state reads go through `compat::state_text`;
  and `Text::colored` / `translate_with_fallback` / `with_color` come from the `TextExt` trait (imported).

## What is wired

`tools/hook_map.txt` is the table. Of the 75 old hooks, 70 are wired (step 1 and step 2 part B, the hooks); the five that
are not are listed at the end (no module answers four of them). A hook is wired one of three ways, and the classes in
the table say which: `global (wired)` (a `mod_api::Hooks` method of the same name), `part (wired)` (a Part method of
the same name) or `adapted (wired)` (the new API has another name or another mechanism for it).

* **Blocks (step 1):** 10 `BlockPart` and 6 `BlockEntityPart` methods on the 607 blocks (181 with an entity), and the
  globals `standalone_models`, `component_types`, `server_tick`, `level_tick_post`, `client_tick_post`,
  `right_click_block`, `left_click_block`, `block_step_on` (and `model_baking_completed`, which resolves drawn models).
* **Items:** one `CreateItem` `ItemPart` for each of the 655 items (99 content items, 556 blocks with `"item": true`;
  `tools/gen_parts.py` writes `ITEMS`). It forwards `use_item`, `inventory_tick`, `tooltip`, `item_bar`,
  `use_duration`, `use_animation`, `release_using` and `on_block_placed_by` (the old `place_block`, see below); the
  globals `living_entity_use_item_tick` and `_finish` complete the item-use set. `compat/items.rs`.
* **HUD, keys, screens, tooltips, world drawing:** `compat/client.rs`.
  * `HudBatch` wraps the new `DrawContext` of the hook call (a pointer, as `BlockEntityDraw` does), so `Host::fill`,
    `quad`, `text`, `sprite`, `item`, `gui_tooltip`, `draw_scene` draw at once; `text_width` asks the client's font.
    `hud` runs from `render_gui_post` (not under F1), `tooltip_render_pre` from `render_tooltip_pre`.
  * `key` runs from `input_key` (`key.keyboard.*`, a press or a release; repeats are dropped) and `mouse_button_post`
    (`key.mouse.left/right/middle`). `Host::is_key_down` turns the GLFW code into a vanilla key name for
    `ClientHost::is_key_down` (letters, digits, space, escape); `has_shift_down`/`control`/`alt` read `modifiers()`.
  * The seven `screen_*` hooks and `movement_input` are the new globals of the same names. The mod screen is the same
    idea in the new API (`mod_screen_open`, `mod_screen_close`, `mod_screen`; the screen hooks see `id == "mod"`).
  * `world_primitives` runs from `render_level_stage` (after the translucent blocks): `WorldPrimitives` collects lines
    (`box_outline` is 12) and quads (`filled_box` is 6) and sends them to the `WorldDrawContext`. World text has no
    counterpart and is dropped.
* **Containers and fluids:** `compat/containers.rs`, at `Hooks::register`.
  * `block_entity_container` / `block_entity_fluids` become `Registrar::block_container` / `block_fluids` for each
    block with an entity (the hook is asked at registration with no host).
  * `container_accepts` has no declaration to hang on, so each of those block entity types also gets a **before-game**
    `ITEMS` provider whose handler (`CreateItems`) reads and writes the entity's slots (`item_slot`, `set_item_slot`),
    honours the container's faces, inputs and outputs, and asks the old hook before every insertion.
  * A block whose `custom_block_fluids` says `custom` gets a `FLUIDS` block provider (`CustomFluids`): its handler's
    tank reads are `custom_fluid_tanks`, `insert` is `custom_fluid_fill` and `extract` is `custom_fluid_drain`.
* **Placing and breaking:** `destroy_block` is `Hooks::break_block`. `place_block` is **not** `entity_place`: that fires
  before the block is written, and the modules read and change the placed block, so it is `ItemPart::on_block_placed_by`
  (after the block, its state and `setPlacedBy`, with the pre-shrink stack).
* **The rest, `compat/world.rs`:** `interact_entity` is `entity_interact`; `player_tick` is `player_tick_pre`;
  `loot_drops`, `living_breathe`, `living_knockback`, `hears_projectile_impacts`, `projectile_impact`,
  `entity_tick_listened`, `entity_tick_post`, `menu_clicked`, `menu_closed` are the new globals;
  `living_incoming_damage_v3` is `living_incoming_damage`.
  * **Commands:** `register_commands` collects the modules' usages and registers them as typed `Command`s;
    `run_mod_command` finds the usage the invocation matches and rebuilds the old raw line from it (`clipboard add
    some text`), so `perform_command` sees what it used to. Only word, text, integer, number and boolean values
    are put back; the modules' usages use only those.
  * **Network:** one bidirectional payload, `create:net` (version 1), carries every old message as
    `[u16 channel length][channel][data]`: `Host::net_send_to_player` / `net_send_to_server` send it,
    `net_from_client` / `net_from_server` receive it. A client needs the mod to join (the payload is required).

### Not wired

* `menu_input_closed` (and `Host::open_menu_with_input`): the Toolbox's menu. The new `open_menu` opens a chest
  menu, but nothing hands the slots back when it closes; this needs a small design with the game running.
* `block_changed`, `block_entity_inside`, `item_model_property`, `block_entity_accepts_fluid`: no module answers them.

## Needs testing in game (step 2 part B)

Each of these compiles and follows the docs, but nothing ran:

* **Item hooks and `acting_player`:** the item hooks hand the player in their events, but the modules read
  `host.acting_player()`; the docs say the game sets it for the hook, which `use_item` and `on_block_placed_by` rely on.
  `inventory_tick` now runs only for stacks of a Create item (it ran for every slot before); a module that needs it
  for a vanilla item in hand will not see it.
* **Hoppers into machines:** a mod block with `block_container` is a vanilla-style container, and the docs say hoppers
  keep their direct path for those. If so, `container_accepts` (Depot, Basin, Crafter, ...) is enforced for funnels,
  arms and anything that looks the `ITEMS` capability up, but not for a hopper. Check a hopper into a Depot.
* **Fluids and transactions:** `CustomFluids` and the old fluid hooks write at once, not into the transaction's journal:
  an aborted transaction does not undo them. Fluid components (potions) are dropped (the new `Components` cannot be
  built from a compound).
* **Damage hooks and uuids:** the damage hooks name entities by id; the players' uuids come from a table
  `player_tick_pre` fills, so a hit before a player's first tick has no uuid (Diving fire immunity, Cardboard Sword).
* **Commands:** the old usages are registered as typed commands; the raw line is rebuilt from the usage. Try
  `/clipboard add x`, `/address <text>` and one schematic command.
* **Network:** every old message rides one payload, `create:net`. Try the Ponder viewer, the value box panel, the
  Linked Controller and the symmetry wand.
* **HUD:** the Goggles overlay needs `client_state`, `raycast` and `equipment` (the other agent's methods) as well as
  the drawing wired here. `render_gui_post` is not drawn under F1.
* **`break_block`** fires for a player breaking a block, not for an explosion, a piston or a machine's own
  `destroy_block`: the belt-item drops and the other destroy cleanups run for players only.
* **World drawing:** lines and quads at `AfterTranslucentBlocks`, depth-tested; world text is dropped (the value box
  labels in the world, which the old API drew as text).

## Compatibility gaps worth knowing

These are the places where the new API has no equivalent in the context the old one answered (not merely work left):

* `LevelHost::is_client_side` — the new level views cannot say which side they are on; the compat view says "server". No machine module calls it (only the test doubles in `diodes_signal.rs` define it), so nothing depends on the answer.
* `NeighborChangedEvent::changed` — the old event named the block by registry number; the new one has a `BlockKey`
  with no number: always `BlockId(0)`.
* `update_tag` / `submit` events have no host that reads item slots, so `ModBlockEntity::items` is empty there.
* `parse_snbt` — no text NBT parser reaches a mod: always an error (schematic text files).
* `Text::colored`, `with_color` — the new API doesn't export `TextStyle`: plain text.
* `brewing::potion_color` — no read of vanilla's potion colours: water's.
* Block entity ticks clone the NBT and items into the old struct and write back only what changed.
* `Host::new_server` off the server panics (a client hook cannot be given a `ServerHost`). `compat::guarded` wraps
  every module's hook call and, since the first smoke run showed it, remembers a (hook, module) pair that panicked and skips
  it afterwards: a caught panic still runs the game's panic hook and a backtrace capture (seconds on Windows), and
  `coupling::client_tick_post` panicking every tick took the client to 0.08 frames a second. `coupling` and `super_glue` now read
  nothing on a client (`window()`), so no known hook panics; the client does not see couplings until replicated state is read
  through the client's own services (step 2).

## Step 2, part A: the game calls

`compat/adapter.rs` now answers the old `Host`'s game calls on the new API (the one-line trait methods; the helpers are in
`compat/host_services.rs`). Grouped:

* **Entities** (`entity`, `entity_position`, `_velocity`, `_bounding_box`, `_rotation`, `_on_ground`, `_kind`, `_uuid`, `_nbt`,
  `_push`, `_set_velocity`, `_teleport_to`, `_set_rotation`, `_set_head_yaw`, `_set_fire`, fire and shift reads, `entities_in`,
  `vehicle_of`, `dismount`, `living`, `living_set_equipment`, `hurt_entity`, `remove_entity`, `remove_mob`, `summon`,
  `summon_with_uuid`): handle methods of `EntityRef`/`LivingRef`; `summon_with_uuid` writes the `UUID` int array into the data.
* **Item entities** (`item_entities`, `set_item_entity`): `LevelRef::entities_in` filtered by `Host::item_entity`; count 0 discards.
* **Items and the player's inventory** (`max_stack_size`, `take_items`, `give_items`, `give_stack`, `player_equipment`, `equipment`,
  `player_stacks`, `player_food`, `set_player_status`, `attribute`, `set_attribute_base`, `add_effect`, `player_break_block`,
  `set_inventory_slot_typed`, `insert_item`, `extract_item`, `component`, `set_component`, `remove_component`):
  `take_items` and `give_items` work on the 36 main slots through `inventory_slots` and `set_inventory_slot` / `give_stack`
  (the room is computed here, since `give_stack` drops what does not fit); `insert_item` and `extract_item` go through the `ITEMS`
  capability inside `host.transaction`; components are edited on the old stack's own patch (no registry validation).
* **Players**: `acting_player` and `server_players` read position, rotation, health, entity id, experience, abilities and so on
  through `PlayerRef::entity()` (the earlier placeholders are gone); `player`, `sneaking`, `raycast`, `view_hit`, `push_player`,
  `playing`, `equipment` use `ClientHost::camera_entity` on a client.
* **Recipes**: `recipes_of_type` (the game hands out the same `Arc` until a reload, so each type's slice is leaked once),
  `recipe_ingredient`, `recipe_result` (the JSON is passed as text).
* **Sounds and particles**: `play_sound`, `send_particles`, `send_particles_to` (the player filter is dropped: the new API sends to
  everyone near). Particle keys exist only at registration, so `Hooks::register` asks for the three the glue uses
  (`compat::register_particles`).
* **The world**: `chunk_is_loaded`, `level_is_loaded`, `block_collision`, `collision_boxes`, `clear_between`, `pick_block` (no fluid
  mode in the new clip), `light`, `day_time`, `world_height`, `world_border`, `level_difficulty`, `fluid_flow`,
  `set_break_progress`, `update_signal_neighbors` and `update_output_neighbors` (`LevelRef::update_neighbors`),
  `destroy_block`, `set_block_property_with`, `block_entity_tag`, `set_block_tag`, `block_name_in_tag`,
  `block_sprites_version`, `upgrade_block_state` (identity: no datafixer), `instance_dir`, `profile_by_uuid`, `level_random`
  (an independent stream per dimension, seeded from the level's: the new API only lends a copy).
* **Block entity slots**: `mod_block_entity` reads `items` from `BlockEntityRef::item_slot` (a probe past the end answers the slot
  count); `change_mod_block_entity` writes changed slots in one transaction, and data slots. Only entities that declare a
  container have slots (declaring them is the containers step).
* **Drawing**: `draw_text`, `draw_terrain_model`, `draw_fluid_box`.
* **Containers and tanks, declared at registration** (`compat::register_containers`, called from `Hooks::register`): the old
  `block_entity_container` and `block_entity_fluids` hooks are asked once per block entity block and answered with
  `r.block_container` / `r.block_fluids`, so a block entity's `items` hold its slots and the game's `ITEMS` / `FLUIDS` capabilities serve
  them (hoppers, `game_container`). The old per-insert gates (`container_accepts`, `block_entity_accepts_fluid`) and the `custom_*`
  fluid hooks are still not wired. `block_entity_update_tag` reads a copy of each entity's slots kept by the tick and the click
  (`events::cache_slots`), since that hook has no host to name an item with.
* **`server_item_in_tag`** through `Host::tags()`.

Still neutral, and why: the fluid calls (`fluid_tanks`, `fluid_insert`, `fluid_extract`) need the fluid capabilities wired and
`fluid_tanks` takes `&self`, which cannot lend the `&mut` host a capability handler needs; `use_as_player` (the Deployer: needs
a `PlayerAction` mapping), `carried_stack` and `carried_items` (they belong to the `inventory_tick` item hook), `held`,
`block_states`, `block_entity_nbt` and `set_block_entity` (SNBT text, no parser), `open_menu_with_input` (menus), `client_state`
(the GUI size comes from the draw context of the HUD hook), `client_resources`, `data`, and what the hooks step owns: network
sends, key state, HUD and screen drawing, commands.

## What the smoke runs showed

First run (step 1, `create-new`, a fresh creative world on a stone platform at y 199): the Creative Motor, shafts, cogwheel, gearbox,
Mechanical Press and Depot were drawn from the generated content and the kinetic network turned them. The depot did not take an
iron ingot and the Goggles showed no overlay (item calls and the HUD were gaps).

After step 2 part A (build `native-3f1843e-…`, 4th build of the branch; screenshots in `target/smoke/`, not committed):

* **Depot and press:** an iron ingot stack clicked onto the Depot is taken from the hand, travels to the centre, the press (Creative
  Motor 16 RPM, shaft along the press's facing axis) presses it: `Items: [{iron_ingot x6}, {create:iron_sheet x2}]` after a few seconds
  (`03-depot-press-iron-sheets.png`). Before the containers were declared at registration the depot swallowed the stack and held nothing
  (`02-depot-no-item.png`).
* **Millstone:** wheat dropped on it became `create:wheat_flour` x6 and `minecraft:wheat_seeds` x2 (`04-millstone.png`).
* **Mixer over a Basin:** andesite and an iron nugget dropped into the basin, the mixer (cog beside it at 64 RPM; at 16 RPM it waits for
  `speed_fulfilled`) turned them into `create:andesite_alloy` (`05-mixer-basin-andesite-alloy.png`).
* **Frame time:** the first build with `compat::guarded` calling `coupling::client_tick_post` every tick ran at 0.08 fps (a panic per tick
  runs the game's panic hook and a backtrace capture). After the fix the game runs at 140 fps and `latest.log` has no "panicked" lines.
* Not tried: Goggles overlay and anything on the HUD (the hooks step), the Deployer, fluids, belts.

<!-- Generated by tools/gen_migration.py: the tables below. The prose above them is written by hand. -->

### Hooks not wired yet

| Old hook | Class | New mechanism | Modules that answer it |
|---|---|---|---|
| `block_entity_accepts_fluid` | no direct match | capability (fluids) | (none) |
| `block_changed` | global (not wired) | Hooks::block_changed | (none) |
| `block_entity_inside` | no direct match | BlockPart::entity_inside | (none) |
| `item_model_property` | global (not wired) | Hooks::item_model_property | (none) |
| `menu_input_closed` | no direct match | MenuPart | toolbox |

### Old `Host` methods still answered with a neutral value (14 of 147)

Each logs `create: compat gap: Host::<name> ...` once. Implemented: `acting_player`, `action_bar`, `add_effect`, `attribute`, `block`, `block_collision`, `block_entity_sync`, `block_entity_tag`, `block_name_in_tag`, `block_property`, `block_sprites_version`, `block_state_property`, `change_mod_block_entity`, `chunk_is_loaded`, `clear_between`, `client_state`, `collision_boxes`, `component`, `day_time`, `default_state`, `destroy_block`, `dimension`, `dismount`, `draw_block`, `draw_cube`, `draw_fluid_box`, `draw_item`, `draw_model`, `draw_quad`, `draw_scene`, `draw_terrain_model`, `draw_text`, `entities_in`, `entity`, `entity_bounding_box`, `entity_fire_immune`, `entity_fire_ticks`, `entity_head_yaw`, `entity_is_shift_key_down`, `entity_kind`, `entity_nbt`, `entity_on_ground`, `entity_position`, `entity_push`, `entity_rotation`, `entity_set_fire`, `entity_set_head_yaw`, `entity_set_rotation`, `entity_set_velocity`, `entity_teleport_to`, `entity_uuid`, `entity_velocity`, `equipment`, `extract_item`, `fill`, `fluid_flow`, `give_items`, `give_stack`, `gui_tooltip`, `has_alt_down`, `has_control_down`, `has_shift_down`, `held_stack`, `hurt_entity`, `insert_item`, `instance_dir`, `is_key_down`, `item`, `item_entities`, `level`, `level_difficulty`, `level_is_loaded`, `level_random`, `light`, `living`, `living_set_equipment`, `max_stack_size`, `mod_block_entity`, `mod_screen`, `mod_screen_close`, `mod_screen_open`, `neighbor_signal`, `net_send_to_player`, `net_send_to_server`, `pick_block`, `play_sound`, `player`, `player_break_block`, `player_equipment`, `player_food`, `player_stacks`, `playing`, `profile_by_uuid`, `push_player`, `quad`, `raycast`, `recipe_ingredient`, `recipe_result`, `recipes_of_type`, `remove_component`, `remove_entity`, `remove_mob`, `run_command`, `send_particles`, `send_particles_to`, `server_item_in_tag`, `server_players`, `set_attribute_base`, `set_block`, `set_block_property`, `set_block_property_with`, `set_block_tag`, `set_break_progress`, `set_component`, `set_inventory_slot_typed`, `set_item_entity`, `set_player_status`, `sneaking`, `sprite`, `summon`, `summon_with_uuid`, `take_items`, `tell`, `text`, `text_width`, `update_output_neighbors`, `update_signal_neighbors`, `upgrade_block_state`, `vehicle_of`, `view_hit`, `window`, `world_border`, `world_height`.

`apply_screen_blur`, `block_entity_nbt`, `block_states`, `carried_items`, `carried_stack`, `client_resources`, `data`, `fluid_extract`, `fluid_insert`, `fluid_tanks`, `held`, `open_menu_with_input`, `set_block_entity`, `use_as_player`
