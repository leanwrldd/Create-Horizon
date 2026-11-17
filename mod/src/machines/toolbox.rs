//! The Toolbox (`ToolboxBlock`, `ToolboxBlockEntity`, `ToolboxHandler`, `ToolboxInventory`): 16 dyed
//! boxes with 8 compartments of 4 stacks, each compartment one kind of item. A broken one keeps its
//! contents (the `create:toolbox_inventory` item component); a dye recolours it in place; a player can
//! link a hotbar slot to a compartment and the toolbox keeps that slot at half a stack while the
//! player is within 10 blocks. The rules are `create_kinetics::rules::toolbox`; this is the glue.
//!
//! Known differences from Create, for the README:
//! - **No radial menu and no Toolbox screen.** Create's `RadialToolboxMenu` (a client key and a
//!   screen) and `ToolboxScreen` need client UI Horizon doesn't give a mod. The stand-ins: a plain
//!   right click opens a 9 x 4 chest view of the toolbox (a compartment is a column, its stacks run
//!   down, the ninth column is unused) in which items go in and out by vanilla's rules; what doesn't
//!   belong in a compartment is handed back when the view closes. A sneak right click is the quick
//!   equip: with an item in hand it links the selected hotbar slot to the compartment that item
//!   belongs to (again to unlink); with an empty hand it steps through the compartments that hold
//!   something, and past the last unlinks.
//! - A compartment's filter clears when its view is closed empty unless a player is linked to it
//!   (Create clears it by clicking the empty slot in its screen). Items put into a compartment of
//!   another kind are handed back.
//! - Links are kept in memory, not in the player's data: a world reload unlinks every slot. The
//!   hotbar keeps what it holds; nothing is deposited when the link ends (Create deposits the
//!   stack when it ends by choice), because Horizon can't change a player's slots from their own
//!   click.
//! - Breaking: a left click picks the toolbox up at once with its contents, as `ToolboxBlock.attack`
//!   does, creative included. An explosion's drops get the contents from the block entity if it
//!   is still readable then (untried).
//! - The drawers and lid are drawn closed: no opening animation or sounds besides a click sound.
//! - No hopper, funnel or chute access (`ToolboxInventory` is an item handler capability in Create),
//!   no comparator reading, no waterlogging, no `ToolboxMountedStorage` for contraptions, and
//!   no custom name or `TOOLBOX_UUID`. The dyeing recipe is `minecraft:crafting_transmute`,
//!   which keeps the components, in place of Create's `create:toolbox_dyeing`.
//! - Whether items with components (enchantments) tell apart, and tools (`isDamageableItem`),
//!   is by name: see `rules::toolbox::damageable_id`.

use std::collections::BTreeMap;
use std::sync::Mutex;

use create_kinetics::rules::toolbox::{self as rules, Click, Item, Stack, Step, Toolbox};
use crate::compat::item_data::encode_components;
use crate::compat::{
    BlockEntityEvent, BlockEntitySubmitEvent, BlockStateId, Compound, ComponentTypesEvent, Flow,
    Hooks, Host, ItemStack, LeftClickBlockEvent, LevelHost, LevelTickPostEvent, LootDropsEvent,
    MenuInputClosedEvent, NativeInventoryKind, PlaceBlockEvent, PlayerTickEvent, RightClickBlockEvent,
    StandaloneModelsEvent, StateForPlacementEvent, Tag, Text,
};

use crate::util;

/// `AllDataComponents.TOOLBOX_INVENTORY`.
const COMPONENT: &str = "create:toolbox_inventory";
const INVENTORY: &str = "Inventory";
const DRAWER: &str = "create:block/toolbox/drawer";
/// `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;
const WHITE: u32 = 0xFF_FFFF;

struct Toolboxes;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Toolboxes)
}

fn lid(colour: &str) -> String {
    format!("create:block/toolbox/lid/{colour}")
}

// ---- the stack <-> rules bridge ----------------------------------------------------------------

/// Every stack the glue has seen, so what the rules hand back becomes a real stack again.
type Palette = Vec<(Item, ItemStack)>;

fn item_of(host: &dyn Host, stack: &ItemStack) -> Item {
    let max = host.max_stack_size(&stack.item).unwrap_or(64);
    let components = encode_components(stack).map(|t| format!("{t:?}")).unwrap_or_default();
    Item {
        id: stack.item.to_string(),
        // Damage is among the components: a worn tool is not the same stack as a new one.
        components: format!("{}|{components}", stack.damage),
        max_stack: max,
        damageable: max == 1 && rules::damageable_id(&stack.item),
        fits: rules::fits_inside_containers(&stack.item),
    }
}

fn to_rules(host: &dyn Host, palette: &mut Palette, stack: &ItemStack) -> Stack {
    let item = item_of(host, stack);
    if !palette.iter().any(|(known, _)| *known == item) {
        palette.push((item.clone(), stack.clone()));
    }
    Stack::new(item, stack.count)
}

fn to_real(palette: &Palette, stack: &Stack) -> Option<ItemStack> {
    let (_, template) = palette.iter().find(|(item, _)| *item == stack.item)?;
    let mut real = template.clone();
    real.count = stack.count;
    Some(real)
}

// ---- saved form (`Inventory`: Items with Slot, and Compartments) --------------------------------

fn load(host: &dyn Host, nbt: &Compound) -> (Toolbox, Palette) {
    let mut toolbox = Toolbox::default();
    let mut palette = Palette::new();
    let Some(Tag::Compound(inventory)) = util::get(nbt, INVENTORY) else {
        return (toolbox, palette);
    };
    if let Some(Tag::List(items)) = util::get(inventory, "Items") {
        for tag in items {
            let slot = match tag.get("Slot") {
                Some(Tag::Byte(slot)) if *slot >= 0 => *slot as usize,
                _ => continue,
            };
            if let Some(stack) = util::stack_of(tag)
                && slot < rules::SLOTS
            {
                toolbox.slots[slot] = Some(to_rules(host, &mut palette, &stack));
            }
        }
    }
    if let Some(Tag::List(filters)) = util::get(inventory, "Compartments") {
        for (i, tag) in filters.iter().enumerate().take(rules::COMPARTMENTS) {
            if let Some(stack) = util::stack_of(tag) {
                toolbox.filters[i] = Some(to_rules(host, &mut palette, &stack).item);
            }
        }
    }
    (toolbox, palette)
}

fn save(toolbox: &Toolbox, palette: &Palette) -> Compound {
    let mut items = Vec::new();
    for (slot, stack) in toolbox.slots.iter().enumerate() {
        let Some(real) = stack.as_ref().and_then(|s| to_real(palette, s)) else {
            continue;
        };
        if let Tag::Compound(mut tag) = util::stack_tag(&real) {
            tag.insert("Slot".into(), Tag::Byte(slot as i8));
            items.push(Tag::Compound(tag));
        }
    }
    let filters = toolbox
        .filters
        .iter()
        .map(|filter| {
            filter
                .as_ref()
                .and_then(|item| to_real(palette, &Stack::new(item.clone(), 1)))
                .map_or_else(|| Tag::Compound(Compound::new()), |real| util::stack_tag(&real))
        })
        .collect();
    let mut inventory = Compound::new();
    inventory.insert("Items".into(), Tag::List(items));
    inventory.insert("Compartments".into(), Tag::List(filters));
    inventory.insert("Size".into(), Tag::Int(rules::SLOTS as i32));
    inventory
}

fn read_box(host: &dyn Host, pos: [i32; 3]) -> Option<(Toolbox, Palette)> {
    let entity = host.mod_block_entity(pos)?;
    Some(load(host, &entity.nbt))
}

fn write_box(host: &mut dyn Host, pos: [i32; 3], toolbox: &Toolbox, palette: &Palette) -> bool {
    let inventory = save(toolbox, palette);
    host.change_mod_block_entity(pos, &mut |entity| {
        if util::get(&entity.nbt, INVENTORY) != Some(&Tag::Compound(inventory.clone())) {
            entity.nbt.insert(INVENTORY.into(), Tag::Compound(inventory.clone()));
        }
    })
}

fn tell(host: &mut dyn Host, line: &str) {
    if let Some(player) = host.acting_player() {
        host.action_bar(player.uuid, Text::literal(line.to_owned()));
    }
}

// ---- state kept in memory ----------------------------------------------------------------------

#[derive(Clone)]
struct Link {
    dimension: &'static str,
    pos: [i32; 3],
    compartment: usize,
}

#[derive(Clone)]
struct Seen {
    dimension: &'static str,
    position: [f64; 3],
    tick: i64,
}

#[derive(Default)]
struct State {
    /// `CreateToolboxData`: for each player, the compartment each hotbar slot is linked to.
    links: BTreeMap<u128, BTreeMap<usize, Link>>,
    /// Players ticked lately, where, so the level tick can find them in a world with no player list.
    seen: BTreeMap<u128, Seen>,
    /// Views open, by window: the toolbox they show.
    open: BTreeMap<i32, ([i32; 3], &'static str)>,
    /// Contents waiting for a placed toolbox's block entity: position, dimension, the saved inventory.
    pending: Vec<([i32; 3], &'static str, Compound, u32)>,
}

static STATE: Mutex<State> = Mutex::new(State {
    links: BTreeMap::new(),
    seen: BTreeMap::new(),
    open: BTreeMap::new(),
    pending: Vec::new(),
});

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The compartments of the box at `pos` that a player is linked to.
fn linked_compartments(pos: [i32; 3], dimension: &str) -> [bool; rules::COMPARTMENTS] {
    let mut linked = [false; rules::COMPARTMENTS];
    for links in state().links.values() {
        for link in links.values() {
            if link.pos == pos && link.dimension == dimension {
                linked[link.compartment] = true;
            }
        }
    }
    linked
}

fn blockstate_y(facing: &str) -> f32 {
    match facing {
        "east" => 90.0,
        "south" => 180.0,
        "west" => 270.0,
        _ => 0.0,
    }
}

impl Hooks for Toolboxes {
    fn component_types(&self, event: &mut ComponentTypesEvent<'_>) {
        if !event.types.iter().any(|t| t == COMPONENT) {
            event.types.push(COMPONENT.to_owned());
        }
    }

    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.push(DRAWER.to_owned());
        event.models.extend(rules::COLOURS.iter().map(|c| lid(c)));
    }

    fn block_entity(&self, _: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        rules::is_toolbox(event.block).then(|| (event.block.to_owned(), Compound::new()))
    }

    /// `ToolboxRenderer.renderSafe`: the lid and the two drawers over the body in the chunk mesh, shut.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        let Some(colour) = rules::colour_of(event.block) else {
            return;
        };
        let Some(facing) = host.block_state_property(event.state, "facing") else {
            return;
        };
        let draw = &mut *event.draw;
        draw.push();
        // The block state turns the model clockwise by `y`; a pose turns counterclockwise.
        draw.rotate_about([0.0, 1.0, 0.0], -blockstate_y(facing), [0.5; 3]);
        host.draw_model(draw, &lid(colour), WHITE);
        for offset in [0.0_f32, 1.0] {
            draw.push();
            draw.translate([0.0, offset / 8.0, 0.0]);
            host.draw_model(draw, DRAWER, WHITE);
            draw.pop();
        }
        draw.pop();
        draw.keep();
    }

    /// `ToolboxBlock.getStateForPlacement`: the front faces the player.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        if !rules::is_toolbox(&event.context.item) {
            return None;
        }
        let facing = create_kinetics::looking(event.context.yaw, 0.0).opposite();
        level.state_with_property(event.state, "facing", facing.name()).map(Some)
    }

    /// `ToolboxBlock.setPlacedBy`: the contents the item carries go into the block entity.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if !rules::is_toolbox(&event.stack.item) {
            return;
        }
        let Some(Tag::Compound(inventory)) = event.stack.data().and_then(|d| d.other(COMPONENT)).cloned() else {
            return;
        };
        let pos = event.pos;
        let applied = host.change_mod_block_entity(pos, &mut |entity| {
            entity.nbt.insert(INVENTORY.into(), Tag::Compound(inventory.clone()));
        });
        if !applied {
            state().pending.push((pos, host.dimension(), inventory, 0));
        }
    }

    fn level_tick_post(&self, host: &mut dyn Host, event: &mut LevelTickPostEvent<'_>) {
        apply_pending(host, event.dimension);
        tick_links(host, event.dimension);
    }

    /// Remembers who is where, for `level_tick_post` (a player's own hook can't change their slots).
    fn player_tick(&self, host: &mut dyn Host, _: &mut PlayerTickEvent) {
        let Some(player) = host.acting_player() else {
            return;
        };
        let tick = util::game_time(host);
        let dimension = host.dimension();
        let mut state = state();
        if state.links.contains_key(&player.uuid) || state.seen.contains_key(&player.uuid) {
            state.seen.insert(player.uuid, Seen { dimension, position: player.position, tick });
        }
    }

    /// `ToolboxBlock.attack`: a left click takes the box with what it holds.
    fn left_click_block(&self, host: &mut dyn Host, event: &mut LeftClickBlockEvent) -> Flow {
        let pos = event.pos;
        let block = host.block(pos);
        if !rules::is_toolbox(&block) {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        let Some(entity) = host.mod_block_entity(pos) else {
            return Flow::Continue;
        };
        let dimension = host.dimension();
        let mut stack = ItemStack { item: block.as_str().into(), count: 1, damage: 0, extra: None };
        if let Some(Tag::Compound(inventory)) = util::get(&entity.nbt, INVENTORY) {
            let (toolbox, _) = load(&*host, &entity.nbt);
            if toolbox.slots.iter().any(Option::is_some) || toolbox.filters.iter().any(Option::is_some) {
                let _ = host.set_component(&mut stack, COMPONENT, Tag::Compound(inventory.clone()));
            }
        }
        let Some(air) = host.default_state("minecraft:air") else {
            return Flow::Continue;
        };
        // `ToolboxBlockEntity.unequipTracked`: the links to this box end.
        for links in state().links.values_mut() {
            links.retain(|_, link| !(link.pos == pos && link.dimension == dimension));
        }
        if !host.set_block(dimension, pos, air, util::UPDATE_ALL) {
            return Flow::Continue;
        }
        host.play_sound("minecraft:block.wood.break", util::centre(pos), BLOCKS, 1.0, 0.8);
        if let Some(rest) = host.give_stack(player.uuid, stack) {
            util::spawn_item(host, util::centre(pos), &rest, [0.0; 3], 10);
        }
        Flow::Cancel
    }

    /// An explosion's drops: the contents, if the block entity can still be read.
    fn loot_drops(&self, host: &mut dyn Host, event: &mut LootDropsEvent<'_>) {
        let Some((pos, _)) = event.context.block else {
            return;
        };
        for drop in event.drops.iter_mut() {
            if !rules::is_toolbox(&drop.item) || drop.data().and_then(|d| d.other(COMPONENT)).is_some() {
                continue;
            }
            if let Some(entity) = host.mod_block_entity(pos)
                && let Some(Tag::Compound(inventory)) = util::get(&entity.nbt, INVENTORY)
            {
                let _ = host.set_component(drop, COMPONENT, Tag::Compound(inventory.clone()));
            }
        }
    }

    /// `ToolboxBlock.useItemOn` and the radial menu's stand-in.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        let block = host.block(pos);
        let Some(colour) = rules::colour_of(&block) else {
            return Flow::Continue;
        };
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        let held = host.held_stack(player.uuid);
        // The Wrench picks the toolbox up, as it does every block of Create's.
        if held.as_ref().is_some_and(|stack| &*stack.item == util::WRENCH) {
            return Flow::Continue;
        }
        if util::click_repeated(&*host, player.uuid, pos) {
            return Flow::Cancel;
        }
        // A dye of another colour recolours it (not while sneaking), keeping the contents and the facing.
        if !player.sneaking
            && let Some(dye) = held.as_ref().and_then(|stack| rules::dye_colour(&stack.item))
            && dye != colour
        {
            util::click_taken(&*host, player.uuid, pos);
            recolour(host, pos, dye);
            return Flow::Cancel;
        }
        util::click_taken(&*host, player.uuid, pos);
        if player.sneaking {
            equip_click(host, &player, pos, held.as_ref());
        } else {
            open_view(host, player.uuid, pos);
        }
        Flow::Cancel
    }

    /// The view closes: what the player left in it goes back into the compartments.
    fn menu_input_closed(&self, host: &mut dyn Host, event: &mut MenuInputClosedEvent<'_>) {
        let Some((pos, dimension)) = state().open.remove(&event.window) else {
            return;
        };
        let Some((mut toolbox, mut palette)) = read_box(&*host, pos) else {
            return;
        };
        let mut view = vec![None; rules::MENU_SLOTS];
        for (i, slot) in event.slots.iter_mut().enumerate().take(rules::MENU_SLOTS) {
            if let Some(stack) = slot.take() {
                view[i] = Some(to_rules(&*host, &mut palette, &stack));
            }
        }
        let keep_empty = linked_compartments(pos, dimension);
        let rejected = toolbox.apply_menu(view, keep_empty);
        write_box(host, pos, &toolbox, &palette);
        // What doesn't belong goes back to the player through the slots that are left.
        let mut back = rejected.iter().filter_map(|stack| to_real(&palette, stack));
        for slot in event.slots.iter_mut() {
            if slot.is_none() {
                *slot = back.next();
            }
        }
        for rest in back {
            util::spawn_item(host, util::centre(pos), &rest, [0.0; 3], 10);
        }
        host.play_sound("minecraft:block.chest.close", util::centre(pos), BLOCKS, 0.1, 1.1);
    }
}

fn open_view(host: &mut dyn Host, uuid: u128, pos: [i32; 3]) {
    let Some((toolbox, palette)) = read_box(&*host, pos) else {
        return;
    };
    let items = toolbox.to_menu().iter().map(|s| s.as_ref().and_then(|s| to_real(&palette, s))).collect();
    let dimension = host.dimension();
    if let Some(window) =
        host.open_menu_with_input(uuid, rules::MENU_ROWS as u8, Text::literal("Toolbox".to_owned()), items, 0..rules::MENU_SLOTS)
    {
        state().open.insert(window, (pos, dimension));
        let at = util::centre(pos);
        host.play_sound("minecraft:block.iron_door.open", at, BLOCKS, 0.25, 1.25);
        host.play_sound("minecraft:block.chest.open", at, BLOCKS, 0.1, 1.15);
    }
}

/// A dye on a toolbox (`ToolboxBlock.useItemOn`): the same box in another colour.
fn recolour(host: &mut dyn Host, pos: [i32; 3], colour: &str) {
    let facing = host.block_property(pos, "facing");
    let Some(entity) = host.mod_block_entity(pos) else {
        return;
    };
    let nbt = entity.nbt.clone();
    let dimension = host.dimension();
    let Some(state) = host.default_state(&format!("create:{colour}_toolbox")) else {
        return;
    };
    if !host.set_block(dimension, pos, state, util::UPDATE_ALL) {
        return;
    }
    if let Some(facing) = facing {
        host.set_block_property(pos, "facing", &facing);
    }
    let saved = nbt.clone();
    let restored = host.change_mod_block_entity(pos, &mut |entity| {
        entity.nbt = saved.clone();
    });
    if !restored {
        state_pending(pos, host.dimension(), nbt);
    }
}

fn state_pending(pos: [i32; 3], dimension: &'static str, nbt: Compound) {
    if let Some(Tag::Compound(inventory)) = util::get(&nbt, INVENTORY) {
        state().pending.push((pos, dimension, inventory.clone(), 0));
    }
}

/// `ToolboxHandler`/`ToolboxEquipPacket`: link or unlink the selected hotbar slot.
fn equip_click(host: &mut dyn Host, player: &crate::compat::NativeServerPlayer, pos: [i32; 3], held: Option<&ItemStack>) {
    let Some((toolbox, palette)) = read_box(&*host, pos) else {
        return;
    };
    let dimension = host.dimension();
    let hotbar = usize::from(player.selected_slot);
    let held_item = held.map(|stack| item_of(&*host, stack));
    let counts: [u32; rules::COMPARTMENTS] = std::array::from_fn(|c| toolbox.total_in_compartment(c));
    let linked = state()
        .links
        .get(&player.uuid)
        .and_then(|links| links.get(&hotbar))
        .filter(|link| link.pos == pos && link.dimension == dimension)
        .map(|link| link.compartment);
    match rules::sneak_click(&toolbox.filters, counts, held_item.as_ref(), linked) {
        Click::Equip(compartment) => {
            {
                let mut state = state();
                state.links.entry(player.uuid).or_default().insert(hotbar, Link { dimension, pos, compartment });
                let tick = util::game_time(&*host);
                state.seen.insert(player.uuid, Seen { dimension, position: player.position, tick });
            }
            let name = toolbox.filters[compartment]
                .as_ref()
                .and_then(|item| to_real(&palette, &Stack::new(item.clone(), 1)))
                .map_or_else(String::new, |stack| stack.item.to_string());
            tell(host, &format!("Hotbar slot {} linked to {name}", hotbar + 1));
        }
        Click::Unequip => {
            if let Some(links) = state().links.get_mut(&player.uuid) {
                links.remove(&hotbar);
            }
            tell(host, &format!("Hotbar slot {} unlinked", hotbar + 1));
        }
        Click::Nothing => {}
    }
}

/// Contents a placed box was waiting for, once its block entity exists.
fn apply_pending(host: &mut dyn Host, dimension: &str) {
    let jobs: Vec<_> = {
        let mut state = state();
        let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut state.pending).into_iter().partition(|job| job.1 == dimension);
        state.pending = rest;
        mine
    };
    for (pos, dim, inventory, tries) in jobs {
        let applied = host.change_mod_block_entity(pos, &mut |entity| {
            entity.nbt.insert(INVENTORY.into(), Tag::Compound(inventory.clone()));
        });
        if !applied && tries < 40 {
            state().pending.push((pos, dim, inventory, tries + 1));
        }
    }
}

/// `ToolboxBlockEntity.tickPlayers`, for the players of this level: each linked hotbar slot is kept
/// at half a stack of its compartment's item.
fn tick_links(host: &mut dyn Host, dimension: &str) {
    let now = util::game_time(host);
    let players: Vec<(u128, Seen, Vec<(usize, Link)>)> = {
        let state = state();
        state
            .links
            .iter()
            .filter_map(|(uuid, links)| {
                let seen = state.seen.get(uuid)?;
                (seen.dimension == dimension && now - seen.tick <= 2)
                    .then(|| (*uuid, seen.clone(), links.iter().map(|(slot, link)| (*slot, link.clone())).collect()))
            })
            .collect()
    };
    for (uuid, seen, links) in players {
        for (hotbar, link) in links {
            if link.dimension != dimension {
                continue;
            }
            match tick_link(host, uuid, &seen, hotbar, &link) {
                LinkResult::Keep => {}
                LinkResult::Drop => {
                    if let Some(links) = state().links.get_mut(&uuid) {
                        links.remove(&hotbar);
                    }
                }
            }
        }
    }
}

enum LinkResult {
    Keep,
    Drop,
}

fn tick_link(host: &mut dyn Host, uuid: u128, seen: &Seen, hotbar: usize, link: &Link) -> LinkResult {
    // `ToolboxBlockEntity.isOpenInContainer`: nothing moves while the view is open.
    if state().open.values().any(|(pos, _)| *pos == link.pos) {
        return LinkResult::Keep;
    }
    if !host.block(link.pos).ends_with("_toolbox") {
        // Gone (an unloaded chunk reads as air too: stay linked while the chunk isn't there).
        let chunk = [link.pos[0] >> 4, link.pos[2] >> 4];
        return if host.chunk_is_loaded(link.dimension, chunk) { LinkResult::Drop } else { LinkResult::Keep };
    }
    if !rules::within_range(seen.position, link.pos) {
        return LinkResult::Keep;
    }
    let Some((mut toolbox, mut palette)) = read_box(&*host, link.pos) else {
        return LinkResult::Keep;
    };
    let Some(stacks) = host.player_stacks(uuid) else {
        return LinkResult::Keep;
    };
    let slot = stacks.get(hotbar).cloned().flatten();
    let slot_rules = slot.as_ref().map(|stack| to_rules(&*host, &mut palette, stack));
    let filter = toolbox.filters[link.compartment].clone();
    let step = rules::tick_link(slot_rules.as_ref(), filter.as_ref());
    let new_slot: Option<ItemStack> = match step {
        Step::Nothing => return LinkResult::Keep,
        Step::Unlink => return LinkResult::Drop,
        Step::Take(amount) => {
            let Some(got) = toolbox.take_from_compartment(amount, link.compartment, false) else {
                return LinkResult::Keep;
            };
            let Some(mut real) = to_real(&palette, &got) else {
                return LinkResult::Keep;
            };
            real.count += slot.as_ref().map_or(0, |stack| stack.count);
            Some(real)
        }
        Step::Deposit(amount) => {
            let Some(current) = slot.as_ref() else {
                return LinkResult::Keep;
            };
            let part = Stack::new(slot_rules.as_ref().map(|s| s.item.clone()).unwrap_or_else(|| item_of(&*host, current)), amount);
            let left = toolbox.distribute_to_compartment(part, link.compartment, false, false);
            let deposited = amount - left.map_or(0, |l| l.count);
            if deposited == 0 {
                return LinkResult::Keep;
            }
            let mut real = current.clone();
            real.count -= deposited;
            (real.count > 0).then_some(real)
        }
    };
    let changed = host.set_inventory_slot_typed(uuid, NativeInventoryKind::Player, hotbar, slot.as_ref(), new_slot).is_ok();
    if changed {
        write_box(host, link.pos, &toolbox, &palette);
    }
    LinkResult::Keep
}
