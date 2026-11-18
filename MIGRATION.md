# Migration to Horizon's new mod API

Step 1 (scaffold and compatibility layer), branch `migrate-scaffold`. The port builds as one flat package on the
developer version of Horizon (3f1843e) and keeps its 137 machine modules as they were, behind a compatibility
layer. This file says what is where, what is wired, and what the next steps have to do.

## Layout

| Path | What |
|---|---|
| `mod/horizon-mod.toml` | the manifest: content fields plus `[native]` (game 26.3, Horizon 3f1843e, horizon-mixins e59da90) |
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

`tools/hook_map.txt` is the table. Of the 75 old hooks, **30 are global, 23 are Part methods, 22 have no direct
match**. Wired now: 9 global hooks (`standalone_models`, `component_types`, `server_tick`, `level_tick_post`,
`client_tick_post`, `right_click_block`, `left_click_block`, `block_step_on`, plus the new `model_baking_completed`
that resolves drawn models) and 10 `BlockPart` plus 6 `BlockEntityPart` methods, for the 607 blocks (181 with an
entity). The other hooks are listed below.

## Step 2: hooks with no direct match

These are not wired and not stubbed beyond a default. The first group is the one the user named; the table at the
end lists every hook that is not wired, with the modules that answer it.

* **Placing and breaking blocks** (`place_block` → `entity_place`, `destroy_block` → `break_block`): modules that
  decide what a placement or a break does (belts, shafts' neighbour updates, contraption controls).
* **Containers and fluids** (`block_entity_container`, `container_accepts`, `block_entity_fluids`,
  `block_entity_accepts_fluid`, `custom_block_fluids`, `custom_fluid_*`): become item and fluid capabilities. Every
  machine with slots or tanks.
* **Network messages** (`net_from_server`, `net_from_client`, `Host::net_send_*`): become payloads. Ponder, the
  clipboard, value boxes, schematic screens, controllers.
* **Commands** (`register_commands`, `perform_command`): the clipboard, schematic, symmetry wand commands.
* **HUD and screens** (`hud`, `tooltip_render_pre`, `world_primitives`, `screen_*`, `Host::fill/text/item/...`): goggles'
  overlay, value-box panels, Ponder, schematic hologram outlines.
* **Key bindings** (`key`, `Host::is_key_down`): Ponder, linked controller.
* **Items** (`use_item`, `inventory_tick`, `tooltip`, `item_bar`, `use_duration`, `use_animation`, `release_using`):
  the new API has them on `ItemPart`; one Part per Create item still has to be generated.

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
  every module's hook call, so the panic is logged once and that module's hook is skipped for the tick instead of
  ending the game; the host still writes a crash report for the first one. Known case: `coupling::client_tick_post`
  reads the level attachment through `new_server()`. Step 2: read replicated state through the client's own services.

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

Still neutral, and why: the fluid calls (`fluid_tanks`, `fluid_insert`, `fluid_extract`) need the fluid capabilities wired and
`fluid_tanks` takes `&self`, which cannot lend the `&mut` host a capability handler needs; `use_as_player` (the Deployer: needs
a `PlayerAction` mapping), `carried_stack` and `carried_items` (they belong to the `inventory_tick` item hook), `held`,
`block_states`, `block_entity_nbt` and `set_block_entity` (SNBT text, no parser), `open_menu_with_input` (menus), `client_state`
(the GUI size comes from the draw context of the HUD hook), `client_resources`, `data`, and what the hooks step owns: network
sends, key state, HUD and screen drawing, commands.

## What the first smoke run showed

On `create-new`, a fresh creative world, a stone platform at 290..312 / 120 / 290..310: the Creative Motor, shafts,
cogwheel, gearbox, Mechanical Press and Depot were drawn from the generated content and the kinetic network turned
the shaft and cogwheel (frames differ between screenshots). Not working yet, as expected from the gaps: the depot
did not take an iron ingot (item give/take are neutral values), so the press had nothing to press; the Goggles'
stress overlay did not appear (HUD is step 2). No content warnings in `logs/latest.log`.

<!-- Generated by tools/gen_migration.py: the tables below. The prose above them is written by hand. -->

### Hooks not wired yet

| Old hook | Class | New mechanism | Modules that answer it |
|---|---|---|---|
| `block_entity_container` | no direct match | capability (items) | basin, chute, crafter, creative_crate, crushing_wheel, deployer, frogport, item_drain, item_vault, lib, millstone, packager, postbox, saw, schematic_cannon, schematic_table, weighted_ejector |
| `block_entity_fluids` | no direct match | capability (fluids) | basin, item_drain, spout |
| `block_entity_accepts_fluid` | no direct match | capability (fluids) | (none) |
| `container_accepts` | no direct match | capability (items) | basin, chute, crafter, crushing_wheel, deployer, depot, frogport, item_drain, lib, millstone, packager, postbox, saw, weighted_ejector |
| `destroy_block` | no direct match | break_block (Hooks) | arm, basin, bearing, bracket, chain_conveyor, clockwork_bearing, copycat, crafter, creative_crate, elevator, extendo_grip, fluid_tank, frogport, gantry, item_drain, item_vault, large_water_wheel, lectern_controller, lib, packager, piston, placard, pulley, symmetry_wand, tunnel, weighted_ejector |
| `place_block` | no direct match | entity_place (Hooks) | arm_select, backtank, bell, chain_conveyor, chain_drive, chute, clipboard, content_observer, copycat, crushing_wheel, diodes, display_link_select, encased, extendo_grip, fluid_pipe, flywheel, frogport, funnel, gauges, gearshift, large_water_wheel, lib, nixie_tube, packager, rose_quartz_lamp, schematic_cannon, sequenced_gearshift, smart_fluid_pipe, speed_controller, stockpile_switch, symmetry_wand, toolbox, tunnel, vertical_gearbox, weighted_ejector |
| `use_item` | part (not wired) | ItemPart::use_item | clipboard, filter, fluid_tank, item_drain, package, potato_cannon, sand_paper, schematic, symmetry_wand, worldshaper |
| `interact_entity` | no direct match | EntityPart::interact / interact_entity (Hooks) | burner, coupling |
| `block_changed` | global (not wired) | Hooks::block_changed | (none) |
| `block_entity_inside` | no direct match | BlockPart::entity_inside | (none) |
| `player_tick` | no direct match | player_tick_pre/post (Hooks) | bell, belt_carry, cardboard_sword, diving, extendo_grip, lectern_controller, linked_controller, schematic, super_glue, symmetry_wand, toolbox, value_box |
| `inventory_tick` | part (not wired) | ItemPart::inventory_tick | clipboard, extendo_grip, linked_controller, package, potato_cannon, schematic, symmetry_wand |
| `tooltip` | part (not wired) | ItemPart::tooltip | filter, mod, package, schematic, screen, sequenced |
| `hud` | no direct match | client render hook (no match yet) | goggles, linked_controller, value_box_draw |
| `menu_clicked` | global (not wired) | Hooks::menu_clicked | (none) |
| `menu_closed` | global (not wired) | Hooks::menu_closed | (none) |
| `item_model_property` | global (not wired) | Hooks::item_model_property | (none) |
| `loot_drops` | global (not wired) | Hooks::loot_drops | backtank, bracket, clipboard, copycat, lectern_controller, toolbox |
| `net_from_server` | no direct match | payload (PayloadsClientHost) | belt_carry, fan, lectern_controller, schematic_view, symmetry_wand |
| `custom_block_fluids` | no direct match | capability (fluids) | fluid_tank, hose_pulley, portable_interface |
| `custom_fluid_tanks` | no direct match | capability (fluids) | fluid_tank, hose_pulley, portable_interface |
| `custom_fluid_fill` | no direct match | capability (fluids) | fluid_tank, hose_pulley, portable_interface |
| `custom_fluid_drain` | no direct match | capability (fluids) | fluid_tank, hose_pulley, portable_interface |
| `key` | no direct match | key mappings (KeyMappings) | linked_controller, mod, package, screen, value_box_screen |
| `tooltip_render_pre` | no direct match | ItemPart::tooltip / client render | mod |
| `screen_render_post` | global (not wired) | Hooks::screen_render_post | mod, value_box_screen |
| `screen_closing` | global (not wired) | Hooks::screen_closing | mod, value_box_screen |
| `screen_key_pressed_pre` | global (not wired) | Hooks::screen_key_pressed_pre | mod |
| `screen_char_typed_pre` | global (not wired) | Hooks::screen_char_typed_pre | mod |
| `screen_mouse_button_pressed_pre` | global (not wired) | Hooks::screen_mouse_button_pressed_pre | mod |
| `screen_mouse_button_released_pre` | global (not wired) | Hooks::screen_mouse_button_released_pre | mod, value_box_screen |
| `screen_mouse_scrolled_pre` | global (not wired) | Hooks::screen_mouse_scrolled_pre | mod, value_box_screen |
| `item_bar` | part (not wired) | ItemPart::item_bar | backtank, sequenced |
| `use_duration` | part (not wired) | ItemPart::use_duration | package, sand_paper |
| `use_animation` | part (not wired) | ItemPart::use_animation | package, sand_paper |
| `living_entity_use_item_tick` | global (not wired) | Hooks::living_entity_use_item_tick | sand_paper |
| `living_entity_use_item_finish` | global (not wired) | Hooks::living_entity_use_item_finish | sand_paper |
| `entity_tick_listened` | global (not wired) | Hooks::entity_tick_listened | belt_carry |
| `entity_tick_post` | global (not wired) | Hooks::entity_tick_post | belt_carry |
| `menu_input_closed` | no direct match | MenuPart | toolbox |
| `register_commands` | global (not wired) | Hooks::register_commands | clipboard, package, schematic, schematic_cannon |
| `perform_command` | no direct match | register_commands (Command tree) | clipboard, package, schematic, schematic_cannon |
| `movement_input` | global (not wired) | Hooks::movement_input | linked_controller |
| `net_from_client` | no direct match | payload (server payload handler) | linked_controller, value_box |
| `living_breathe` | global (not wired) | Hooks::living_breathe | diving |
| `living_incoming_damage_v3` | no direct match | living_incoming_damage (Hooks) | cardboard_sword, diving, extendo_grip |
| `living_knockback` | global (not wired) | Hooks::living_knockback | extendo_grip |
| `hears_projectile_impacts` | global (not wired) | Hooks::hears_projectile_impacts | potato_cannon |
| `projectile_impact` | global (not wired) | Hooks::projectile_impact | potato_cannon |
| `world_primitives` | no direct match | client render hook (no match yet) | coupling, schematic_view, symmetry_wand, value_box_draw |
| `release_using` | part (not wired) | ItemPart::release_using | package |

### Old `Host` methods still answered with a neutral value (124 of 147)

Each logs `create: compat gap: Host::<name> ...` once. Implemented: `acting_player`, `action_bar`, `block`, `block_entity_sync`, `block_property`, `block_state_property`, `change_mod_block_entity`, `default_state`, `dimension`, `draw_block`, `draw_cube`, `draw_item`, `draw_model`, `draw_quad`, `held_stack`, `level`, `mod_block_entity`, `neighbor_signal`, `run_command`, `set_block`, `set_block_property`, `tell`, `window`.

`add_effect`, `apply_screen_blur`, `attribute`, `block_collision`, `block_entity_nbt`, `block_entity_tag`, `block_name_in_tag`, `block_sprites_version`, `block_states`, `carried_items`, `carried_stack`, `chunk_is_loaded`, `clear_between`, `client_resources`, `client_state`, `collision_boxes`, `component`, `data`, `day_time`, `destroy_block`, `dismount`, `draw_fluid_box`, `draw_scene`, `draw_terrain_model`, `draw_text`, `entities_in`, `entity`, `entity_bounding_box`, `entity_fire_immune`, `entity_fire_ticks`, `entity_head_yaw`, `entity_is_shift_key_down`, `entity_kind`, `entity_nbt`, `entity_on_ground`, `entity_position`, `entity_push`, `entity_rotation`, `entity_set_fire`, `entity_set_head_yaw`, `entity_set_rotation`, `entity_set_velocity`, `entity_teleport_to`, `entity_uuid`, `entity_velocity`, `equipment`, `extract_item`, `fill`, `fluid_extract`, `fluid_flow`, `fluid_insert`, `fluid_tanks`, `give_items`, `give_stack`, `gui_tooltip`, `has_alt_down`, `has_control_down`, `has_shift_down`, `held`, `hurt_entity`, `insert_item`, `instance_dir`, `is_key_down`, `item`, `item_entities`, `level_difficulty`, `level_is_loaded`, `level_random`, `light`, `living`, `living_set_equipment`, `max_stack_size`, `mod_screen`, `mod_screen_close`, `mod_screen_open`, `net_send_to_player`, `net_send_to_server`, `open_menu_with_input`, `pick_block`, `play_sound`, `player`, `player_break_block`, `player_equipment`, `player_food`, `player_stacks`, `playing`, `profile_by_uuid`, `push_player`, `quad`, `raycast`, `recipe_ingredient`, `recipe_result`, `recipes_of_type`, `remove_component`, `remove_entity`, `remove_mob`, `send_particles`, `send_particles_to`, `server_item_in_tag`, `server_players`, `set_attribute_base`, `set_block_entity`, `set_block_property_with`, `set_block_tag`, `set_break_progress`, `set_component`, `set_inventory_slot_typed`, `set_item_entity`, `set_player_status`, `sneaking`, `sprite`, `summon`, `summon_with_uuid`, `take_items`, `text`, `text_width`, `update_output_neighbors`, `update_signal_neighbors`, `upgrade_block_state`, `use_as_player`, `vehicle_of`, `view_hit`, `world_border`, `world_height`
