# Migration to Horizon's new mod API

Step 1 (scaffold and compatibility layer, branch `migrate-scaffold`) and step 2 part B (the hooks, branch
`step2-hooks`). The port builds as one flat package on the developer version of Horizon (3f1843e) and keeps its 137
machine modules as they were, behind a compatibility layer. This file says what is where, what is wired, and what the
next steps have to do. Step 2 part B was type-checked only (`tools/check_module.sh all`): none of it has run in the
game yet, so "Needs testing in game" below is the list to play through first.

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

* `LevelHost::is_client_side` — the new level views cannot say which side they are on; the compat view says "server".
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
| `block_entity_accepts_fluid` | no direct match | capability (fluids) | (none) |
| `block_changed` | global (not wired) | Hooks::block_changed | (none) |
| `block_entity_inside` | no direct match | BlockPart::entity_inside | (none) |
| `item_model_property` | global (not wired) | Hooks::item_model_property | (none) |
| `menu_input_closed` | no direct match | MenuPart | toolbox |

### Old `Host` methods still answered with a neutral value (107 of 147)

Each logs `create: compat gap: Host::<name> ...` once. Implemented: `acting_player`, `action_bar`, `block`, `block_entity_sync`, `block_property`, `block_state_property`, `change_mod_block_entity`, `default_state`, `dimension`, `draw_block`, `draw_cube`, `draw_item`, `draw_model`, `draw_quad`, `draw_scene`, `fill`, `gui_tooltip`, `has_alt_down`, `has_control_down`, `has_shift_down`, `held_stack`, `is_key_down`, `item`, `level`, `mod_block_entity`, `mod_screen`, `mod_screen_close`, `mod_screen_open`, `neighbor_signal`, `net_send_to_player`, `net_send_to_server`, `quad`, `run_command`, `set_block`, `set_block_property`, `sprite`, `tell`, `text`, `text_width`, `window`.

`add_effect`, `apply_screen_blur`, `attribute`, `block_collision`, `block_entity_nbt`, `block_entity_tag`, `block_name_in_tag`, `block_sprites_version`, `block_states`, `carried_items`, `carried_stack`, `chunk_is_loaded`, `clear_between`, `client_resources`, `client_state`, `collision_boxes`, `component`, `data`, `day_time`, `destroy_block`, `dismount`, `draw_fluid_box`, `draw_terrain_model`, `draw_text`, `entities_in`, `entity`, `entity_bounding_box`, `entity_fire_immune`, `entity_fire_ticks`, `entity_head_yaw`, `entity_is_shift_key_down`, `entity_kind`, `entity_nbt`, `entity_on_ground`, `entity_position`, `entity_push`, `entity_rotation`, `entity_set_fire`, `entity_set_head_yaw`, `entity_set_rotation`, `entity_set_velocity`, `entity_teleport_to`, `entity_uuid`, `entity_velocity`, `equipment`, `extract_item`, `fluid_extract`, `fluid_flow`, `fluid_insert`, `fluid_tanks`, `give_items`, `give_stack`, `held`, `hurt_entity`, `insert_item`, `instance_dir`, `item_entities`, `level_difficulty`, `level_is_loaded`, `level_random`, `light`, `living`, `living_set_equipment`, `max_stack_size`, `open_menu_with_input`, `pick_block`, `play_sound`, `player`, `player_break_block`, `player_equipment`, `player_food`, `player_stacks`, `playing`, `profile_by_uuid`, `push_player`, `raycast`, `recipe_ingredient`, `recipe_result`, `recipes_of_type`, `remove_component`, `remove_entity`, `remove_mob`, `send_particles`, `send_particles_to`, `server_item_in_tag`, `server_players`, `set_attribute_base`, `set_block_entity`, `set_block_property_with`, `set_block_tag`, `set_break_progress`, `set_component`, `set_inventory_slot_typed`, `set_item_entity`, `set_player_status`, `sneaking`, `summon`, `summon_with_uuid`, `take_items`, `update_output_neighbors`, `update_signal_neighbors`, `upgrade_block_state`, `use_as_player`, `vehicle_of`, `view_hit`, `world_border`, `world_height`
