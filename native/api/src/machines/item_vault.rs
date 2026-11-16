//! The Item Vault (`ItemVaultBlock`, `ItemVaultBlockEntity`, `ConnectivityHandler`,
//! `ItemHelper.calcRedstoneFromInventory`): a storage block of 20 stacks that joins its neighbours
//! along its axis into one wider, longer vault (up to 3 wide and 3 times its width long, `large`
//! from 3 wide). The rules (which blocks join, the slots' order, the comparator reading) are
//! `create_kinetics::rules::item_vault`; this is the glue.
//!
//! Each block keeps its own 20 slots in its block entity, as Create does (`Inventory`); Create's
//! combined handler (`initCapability`, the controller's slots first) is what chutes and funnels
//! see: `chute_inv` reads it from the saved `Controller`, `Size` and `Length`, so a funnel or a
//! chute on any block of a vault fills and empties the whole of it, and a Smart Observer or a
//! Threshold Switch reads all of it.
//!
//! Known differences from Create:
//! - Horizon's hoppers (and droppers) reach only the slots of the block they touch, and a mod
//!   can't give a block a bigger container than its own. So that a hopper on any block of a
//!   vault can still fill and empty the whole vault, a block with a hopper beside it moves one
//!   stack to or from the rest of the vault every few ticks (`rules::serve`): a full block makes
//!   room, an empty one is given stock. A hopper under a block that holds nothing waits up to a
//!   fifth of a second for its first stack.
//! - The blocks are drawn from Create's blockstate (`axis`, `large`) and its single-block model:
//!   Create's connected textures (`ItemVaultCTBehaviour`, the larger texture sheets that run
//!   across a vault) are not drawn, so a large vault looks like its blocks side by side.
//! - A block saves `Items` (Horizon's container) rather than Create's `Inventory`, and no
//!   `LastKnownPos` (a vault moved by a contraption); nothing here moves a vault.
//! - `ItemVaultItem.tryMultiPlace` (a whole layer of a wide vault placed at once) is not done.
//! - Joining happens at the block entity's first tick, and a vault that lost a block without a
//!   player breaking it (an explosion) notices within a second (`VERIFY`), not at once.
//! - The comparator reads the whole vault from a table the blocks keep (a comparator can't read a
//!   block entity), so a vault in a chunk that has just loaded reads 0 until its blocks have
//!   ticked once.
//! - The vault's sound is the netherite block's (Create's `SILENCED_METAL` while placing a row of
//!   them is not done), and no Create sounds play for the Wrench.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{LazyLock, Mutex};

use create_kinetics::rules::item_vault::{self as rules, Axis, Need, Part, Region, SLOTS};
use create_kinetics::{Dir, Pos, looking};
use horizon_client::mod_api::{
    AnalogOutputEvent, BlockEntityContainerEvent, BlockEntityEvent, BlockEntityTickEvent,
    BlockEntityTicksEvent, BlockStateId, Compound, DestroyBlockEvent, Flow, HasAnalogOutputEvent,
    Hooks, Host, ItemStack, LevelHost, ModBlockEntity, ModContainer, ServerTickEvent,
    ServerTickPhase, StateForPlacementEvent, Tag, UseItemOnEvent,
};

use super::diodes_signal as signal;
use crate::util;

pub const ID: &str = "create:item_vault";

/// Saved as Create's `ItemVaultBlockEntity.write` names them.
const CONTROLLER: &str = "Controller";
const SIZE: &str = "Size";
const LENGTH: &str = "Length";
const UNINITIALIZED: &str = "Uninitialized";
/// How often a block checks that the vault it belongs to is still whole.
const VERIFY: i64 = 20;
/// How often a block with a hopper beside it looks at what the hopper needs.
const SERVE: i64 = 4;
/// Ticks a block may miss before its table entry counts as gone.
const MAX_AGE: i64 = 3;
/// `Slot limit`: a vault's slots take 64 of an item at most (`ItemStackHandler.getSlotLimit`).
const SLOT_LIMIT: u32 = 64;

struct Vaults;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Vaults)
}

// --- the saved state ----------------------------------------------------------------------------

fn read_pos(nbt: &Compound) -> Option<Pos> {
    match util::get(nbt, CONTROLLER) {
        Some(Tag::IntArray(v)) if v.len() == 3 => Some([v[0], v[1], v[2]]),
        _ => None,
    }
}

fn read_part(nbt: &Compound, pos: Pos, axis: Axis) -> Part {
    Part {
        controller: read_pos(nbt).unwrap_or(pos),
        width: util::int(nbt, SIZE).max(1),
        length: util::int(nbt, LENGTH).max(1),
        axis,
        update_connectivity: util::has(nbt, UNINITIALIZED),
    }
}

/// What a block's saved compound should hold for `part`: a key with no value is removed.
fn part_tags(pos: Pos, part: &Part) -> Vec<(&'static str, Option<Tag>)> {
    vec![
        (
            CONTROLLER,
            (part.controller != pos).then(|| Tag::IntArray(part.controller.to_vec())),
        ),
        (SIZE, Some(Tag::Int(part.width))),
        (LENGTH, Some(Tag::Int(part.length))),
        (UNINITIALIZED, part.update_connectivity.then_some(Tag::Byte(1))),
    ]
}

/// Writes only what differs: reaching a block entity's fields mutably counts as a change.
fn apply_tags(entity: &mut ModBlockEntity, tags: &[(&'static str, Option<Tag>)]) -> bool {
    let differs = tags.iter().any(|(key, value)| match value {
        Some(value) => util::differs(&entity.nbt, key, value),
        None => util::has(&entity.nbt, key),
    });
    if !differs {
        return false;
    }
    for (key, value) in tags {
        match value {
            Some(value) => {
                if util::differs(&entity.nbt, key, value) {
                    entity.nbt.insert((*key).into(), value.clone());
                }
            }
            None => {
                if util::has(&entity.nbt, key) {
                    util::remove(&mut entity.nbt, key);
                }
            }
        }
    }
    true
}

pub fn new_entity() -> Compound {
    let mut nbt = Compound::new();
    nbt.insert(SIZE.into(), Tag::Int(1));
    nbt.insert(LENGTH.into(), Tag::Int(1));
    // `updateConnectivity`: the block looks for neighbours on its first tick.
    nbt.insert(UNINITIALIZED.into(), Tag::Byte(1));
    nbt
}

/// `CLogistics.vaultCapacity` slots, every one reachable from every side.
fn container() -> ModContainer {
    let all: Vec<usize> = (0..SLOTS).collect();
    ModContainer {
        slots: SLOTS,
        faces: [(); 6].map(|()| all.clone()),
        inputs: all.clone(),
        outputs: all,
        menu: None,
    }
}

// --- the multi-block, through the game ----------------------------------------------------------------

fn axis_of(host: &dyn Host, pos: Pos) -> Option<Axis> {
    host.block_property(pos, "axis").as_deref().and_then(Axis::from_name)
}

type Loader<'a> = Box<dyn FnMut(Pos) -> Option<Part> + 'a>;

fn load_part(host: &dyn Host, pos: Pos) -> Option<Part> {
    let entity = host.mod_block_entity(pos)?;
    if entity.id != ID {
        return None;
    }
    Some(read_part(&entity.nbt, pos, axis_of(host, pos)?))
}

/// A block that changed and its new state.
type Change = (Pos, Part);

/// Runs `op` over the vault blocks around a position and answers what it changed. `own` is the
/// block entity ticking now, which `Host::mod_block_entity` can't reach.
fn solve<'a>(
    host: &'a dyn Host,
    own: Option<(Pos, Part)>,
    op: impl FnOnce(&mut Region<Loader<'a>>),
) -> Vec<Change> {
    let load: Loader<'a> = Box::new(move |pos| {
        if let Some((at, part)) = &own
            && *at == pos
        {
            return Some(part.clone());
        }
        load_part(host, pos)
    });
    let mut region = Region::new(load);
    op(&mut region);
    region.changes()
}

/// Writes what `solve` found: each block's saved state and its `large`.
fn apply(host: &mut dyn Host, mut own: Option<(Pos, &mut ModBlockEntity)>, changes: Vec<Change>) {
    for (pos, part) in changes {
        let tags = part_tags(pos, &part);
        match &mut own {
            Some((at, entity)) if *at == pos => {
                apply_tags(entity, &tags);
            }
            _ => {
                host.change_mod_block_entity(pos, &mut |entity| {
                    apply_tags(entity, &tags);
                });
            }
        }
        // `notifyMultiUpdated` and `removeController`.
        let large = if part.large() { "true" } else { "false" };
        if host.block_property(pos, "large").as_deref() != Some(large) {
            host.set_block_property(pos, "large", large);
        }
    }
}

// --- the tick -------------------------------------------------------------------------------------------

fn tick(host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
    let pos = event.pos;
    let Some(axis) = axis_of(&*host, pos) else {
        return;
    };
    let mut part = read_part(&event.entity.nbt, pos, axis);
    let now = util::game_time(host);

    publish(host, pos, &part, event.entity);

    // `updateConnectivity`: a block that just appeared, or was split off, looks for neighbours.
    if part.update_connectivity {
        part.update_connectivity = false;
        let changes = if part.is_controller(pos) {
            solve(&*host, Some((pos, part.clone())), |region| {
                rules::form_multi(region, pos);
            })
        } else {
            Vec::new()
        };
        let own_changed = changes.iter().any(|(at, _)| *at == pos);
        apply(host, Some((pos, &mut *event.entity)), changes);
        if !own_changed {
            apply_tags(event.entity, &part_tags(pos, &part));
        }
        return;
    }

    // Now and then, check that the vault is still whole: a block broken without a player (an
    // explosion) gave no warning.
    if (now + i64::from(pos[0] ^ pos[2]) + i64::from(pos[1])).rem_euclid(VERIFY) == 0
        && verify(host, pos, &part, event.entity)
    {
        return;
    }

    if (now + i64::from(pos[0] ^ pos[2]) + i64::from(pos[1])).rem_euclid(SERVE) == 0 {
        serve_hoppers(host, pos, &part, event.entity);
    }
}

/// Whether anything was done: a block that lost its controller stands alone, and a controller
/// that lost a block splits.
fn verify(host: &mut dyn Host, pos: Pos, part: &Part, entity: &mut ModBlockEntity) -> bool {
    if !part.is_controller(pos) {
        let covered = load_part(&*host, part.controller).is_some_and(|c| {
            c.controller == part.controller
                && rules::members(part.controller, c.axis, c.width, c.length).contains(&pos)
        });
        if covered {
            return false;
        }
        let changes = vec![(pos, Part { controller: pos, width: 1, length: 1, update_connectivity: true, ..part.clone() })];
        apply(host, Some((pos, entity)), changes);
        return true;
    }
    if part.width == 1 && part.length == 1 {
        return false;
    }
    let dimension = host.dimension();
    for at in rules::members(pos, part.axis, part.width, part.length) {
        if at == pos || host.block(at) == ID {
            continue;
        }
        // An unloaded chunk reads as air: that is not a broken block.
        if !host.chunk_is_loaded(dimension, [at[0] >> 4, at[2] >> 4]) {
            continue;
        }
        // A block is missing: the vault comes apart.
        let missing = Part { controller: pos, ..part.clone() };
        let changes = solve(&*host, Some((pos, part.clone())), |region| {
            rules::split_removed(region, at, missing);
        });
        apply(host, Some((pos, entity)), changes);
        return true;
    }
    false
}

// --- keeping a hopper's block useful ---------------------------------------------------------------

/// What the hoppers (and droppers) beside the block at `pos` need of it: a hopper facing into it
/// pushes items in, one under it pulls items out.
fn hopper_needs(host: &dyn Host, pos: Pos) -> (bool, bool) {
    let (mut room, mut stock) = (false, false);
    for dir in Dir::ALL {
        let beside = util::add(pos, dir.normal());
        let name = host.block(beside);
        if !matches!(name.as_str(), "minecraft:hopper" | "minecraft:dropper") {
            continue;
        }
        let facing = host.block_property(beside, "facing").and_then(|f| Dir::from_name(&f));
        if facing == Some(dir.opposite()) {
            room = true;
        }
        if dir == Dir::Down && name == "minecraft:hopper" {
            stock = true;
        }
    }
    (room, stock)
}

fn stacks_in(items: &[Option<ItemStack>]) -> usize {
    items.iter().filter(|slot| slot.as_ref().is_some_and(|s| s.count > 0)).count()
}

/// `rules::serve` for the block at `pos`: see the module's differences from Create.
fn serve_hoppers(host: &mut dyn Host, pos: Pos, part: &Part, entity: &mut ModBlockEntity) {
    if part.width == 1 && part.length == 1 {
        return;
    }
    let (room, stock) = hopper_needs(&*host, pos);
    if !room && !stock {
        return;
    }
    let members = rules::members(part.controller, part.axis, part.width, part.length);
    let Some(at) = members.iter().position(|member| *member == pos) else {
        return;
    };
    let counts: Vec<usize> = members
        .iter()
        .map(|member| {
            if *member == pos {
                stacks_in(&entity.items)
            } else {
                host.mod_block_entity(*member)
                    .filter(|e| e.id == ID)
                    .map_or(SLOTS, |e| stacks_in(&e.items))
            }
        })
        .collect();
    let mut plan = None;
    if room {
        plan = rules::serve(&counts, at, Need::Room);
    }
    if plan.is_none() && stock {
        plan = rules::serve(&counts, at, Need::Stock);
    }
    let Some((from, to)) = plan else {
        return;
    };
    // The stack that goes: the last of a full block, the first of the one that gives stock.
    let last = from == at;
    let Some(stack) = edit_items(host, pos, entity, members[from], &mut |items| {
        let index = if last {
            items.iter().rposition(|s| s.is_some())
        } else {
            items.iter().position(|s| s.is_some())
        }?;
        items[index].take()
    }) else {
        return;
    };
    let mut placed = Some(stack.clone());
    edit_items(host, pos, entity, members[to], &mut |items| {
        items.resize(SLOTS, None);
        let free = items.iter().position(|s| s.is_none())?;
        items[free] = placed.take();
        Some(())
    });
    // No room after all: it goes back where it came from.
    if let Some(back) = placed {
        edit_items(host, pos, entity, members[from], &mut |items| {
            items.resize(SLOTS, None);
            let free = items.iter().position(|s| s.is_none())?;
            items[free] = Some(back.clone());
            Some(())
        });
    }
}

/// Runs `edit` on the slots of the vault block at `at`: the ticking block's own entity when it is
/// `own_pos`, else the block entity in the world. The answer of `edit`, or `None` when there was
/// no such block or `edit` said so.
fn edit_items<R>(
    host: &mut dyn Host,
    own_pos: Pos,
    own: &mut ModBlockEntity,
    at: Pos,
    edit: &mut dyn FnMut(&mut Vec<Option<ItemStack>>) -> Option<R>,
) -> Option<R> {
    if at == own_pos {
        // Look first: a `&mut` reach into the entity counts as a change.
        let mut probe = own.items.clone();
        let out = edit(&mut probe)?;
        own.items = probe;
        return Some(out);
    }
    let mut out = None;
    host.change_mod_block_entity(at, &mut |entity| {
        let mut probe = entity.items.clone();
        if let Some(result) = edit(&mut probe) {
            entity.items = probe;
            out = Some(result);
        }
    });
    out
}

// --- the comparator -------------------------------------------------------------------------------

/// What one vault block's stacks add up to, as `calcRedstoneFromInventory` counts them.
#[derive(Clone, Debug, PartialEq)]
struct Entry {
    controller: Pos,
    width: i32,
    length: i32,
    axis: Axis,
    fullness: f32,
    non_empty: usize,
    seen: i64,
}

/// Every ticking vault block: its stacks, by level and position. A comparator hook can read only
/// the level's blocks, so the block entities keep this up to date.
static TABLE: LazyLock<Mutex<HashMap<(u8, Pos), Entry>>> = LazyLock::new(Default::default);
/// The signal each vault last gave its comparators, by level and controller.
static SIGNALS: LazyLock<Mutex<HashMap<(u8, Pos), i32>>> = LazyLock::new(Default::default);
/// A server tick counter, for pruning.
static NOW: AtomicI64 = AtomicI64::new(0);

/// The signal a comparator reads at any block of the vault whose controller, size and axis are
/// in `entry`: `ItemHelper.calcRedstoneFromBlockEntity` over the combined handler.
fn combined(table: &HashMap<(u8, Pos), Entry>, level: u8, entry: &Entry) -> (i32, Vec<Pos>) {
    let members = rules::members(entry.controller, entry.axis, entry.width, entry.length);
    let (mut fullness, mut non_empty) = (0.0, 0);
    for member in &members {
        if let Some(e) = table.get(&(level, *member)) {
            fullness += e.fullness;
            non_empty += e.non_empty;
        }
    }
    (rules::comparator_signal(fullness, non_empty, members.len() * SLOTS), members)
}

fn entry_of(host: &dyn Host, part: &Part, entity: &ModBlockEntity, seen: i64) -> Entry {
    let (mut fullness, mut non_empty) = (0.0f32, 0usize);
    for stack in entity.items.iter().flatten() {
        if stack.count == 0 {
            continue;
        }
        let max = host.max_stack_size(&stack.item).unwrap_or(64).min(SLOT_LIMIT);
        fullness += stack.count as f32 / max as f32;
        non_empty += 1;
    }
    Entry {
        controller: part.controller,
        width: part.width,
        length: part.length,
        axis: part.axis,
        fullness,
        non_empty,
        seen,
    }
}

/// Keeps the table up to date and, when the vault's signal changed, tells the comparators beside
/// each of its blocks (`ItemVaultBlockEntity.updateComparators`).
fn publish(host: &mut dyn Host, pos: Pos, part: &Part, entity: &ModBlockEntity) {
    let level = signal::host_level(host);
    let now = NOW.load(Ordering::Relaxed);
    let entry = entry_of(&*host, part, entity, now);
    let (signal, members) = {
        let Ok(mut table) = TABLE.lock() else { return };
        // Most ticks nothing changed: only the entry's age does.
        if let Some(known) = table.get_mut(&(level, pos)) {
            let mut same = entry.clone();
            same.seen = known.seen;
            if same == *known {
                known.seen = now;
                return;
            }
        }
        table.insert((level, pos), entry.clone());
        combined(&table, level, &entry)
    };
    let changed = SIGNALS
        .lock()
        .map(|mut signals| signals.insert((level, entry.controller), signal) != Some(signal))
        .unwrap_or(false);
    if changed {
        let dimension = host.dimension();
        for member in members {
            host.update_output_neighbors(dimension, member);
        }
    }
}

/// States already looked at: a state never changes.
static VAULT_STATES: LazyLock<Mutex<HashMap<u32, bool>>> = LazyLock::new(Default::default);

/// Whether `state` is a vault's: a block with a boolean `large` and a horizontal `axis`.
fn is_vault(level: &dyn LevelHost, state: BlockStateId) -> bool {
    if let Ok(cache) = VAULT_STATES.lock()
        && let Some(known) = cache.get(&state.0)
    {
        return *known;
    }
    let found = signal::takes(level, state, "large", "true")
        && signal::takes(level, state, "axis", "x")
        && signal::takes(level, state, "axis", "z")
        && !signal::takes(level, state, "axis", "y");
    if let Ok(mut cache) = VAULT_STATES.lock() {
        cache.insert(state.0, found);
    }
    found
}

// --- the hooks ------------------------------------------------------------------------------------------

impl Hooks for Vaults {
    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == ID).then(|| (ID.to_owned(), new_entity()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    fn block_entity_container(
        &self,
        _: &dyn Host,
        event: &mut BlockEntityContainerEvent<'_>,
    ) -> Option<ModContainer> {
        (event.block == ID).then(container)
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id == ID {
            tick(host, event);
        }
    }

    /// Prunes the blocks that stopped ticking (their chunk unloaded, or the block went).
    fn server_tick(&self, _: &mut dyn Host, event: &mut ServerTickEvent<'_>) {
        if let ServerTickPhase::ServerTickStart { world_running: true } = event.phase {
            let now = NOW.fetch_add(1, Ordering::Relaxed) + 1;
            if let Ok(mut table) = TABLE.lock() {
                table.retain(|_, entry| now - entry.seen < MAX_AGE);
                if let Ok(mut signals) = SIGNALS.lock() {
                    signals.retain(|(level, controller), _| table.contains_key(&(*level, *controller)));
                }
            }
        }
    }

    /// `ItemVaultBlock.onRemove`: a player breaking a block splits its vault; the block's own
    /// stacks drop with it (Horizon drops a container's items), and the rest stay in the blocks
    /// they were in, as in Create.
    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(part) = load_part(&*host, pos) else {
            return Flow::Continue;
        };
        if part.is_controller(pos) && part.width == 1 && part.length == 1 {
            return Flow::Continue;
        }
        let changes = solve(&*host, None, |region| {
            rules::split_removed(region, pos, part);
        });
        apply(host, None, changes);
        Flow::Continue
    }

    /// `ItemVaultBlock.onWrenched`: on a top or bottom face the vault comes apart, the block is
    /// alone again and turns to its other axis; on a side face nothing changes. Sneaking picks
    /// the block up (`IWrenchable.onSneakWrenched`).
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        if &*event.stack.item != util::WRENCH || host.block(pos) != ID {
            return Flow::Continue;
        }
        let vertical = Dir::from_normal(event.face)
            .is_some_and(|face| face.axis() == create_kinetics::Axis::Y);
        util::wrench(host, pos, |host| {
            if !vertical {
                return;
            }
            let Some(axis) = axis_of(&*host, pos) else {
                return;
            };
            let changes = solve(&*host, None, |region| {
                rules::split(region, pos);
                rules::alone(region, pos);
            });
            apply(host, None, changes);
            host.set_block_property(pos, "axis", axis.turned().name());
        })
    }

    /// `ItemVaultBlock.getStateForPlacement`: along the axis of the vault it is put against
    /// (unless sneaking), else across the way the player looks.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if &*context.item != ID {
            return None;
        }
        let mut axis = None;
        if !context.sneaking {
            let face = util::dir(context.face);
            let on = util::add(
                [context.pos.x, context.pos.y, context.pos.z],
                face.opposite().normal(),
            );
            let state = level.block_state(on);
            if is_vault(level, state) {
                axis = signal::choice(level, state, "axis", &["x", "z"]);
            }
        }
        let axis = axis.unwrap_or_else(|| {
            if matches!(looking(context.yaw, 0.0), Dir::East | Dir::West) { "x" } else { "z" }
        });
        level.state_with_property(event.state, "axis", axis).map(Some)
    }

    /// `ItemVaultBlock.hasAnalogOutputSignal`.
    fn has_analog_output(&self, level: &dyn LevelHost, event: &mut HasAnalogOutputEvent) {
        if is_vault(level, event.state) {
            event.output = true;
        }
    }

    /// `ItemVaultBlock.getAnalogOutputSignal`: the whole vault's fullness.
    fn analog_output(&self, level: &dyn LevelHost, event: &mut AnalogOutputEvent) {
        if !is_vault(level, event.state) {
            return;
        }
        let at = [event.pos.x, event.pos.y, event.pos.z];
        let dimension = signal::level_index(event.dimension);
        let Ok(table) = TABLE.lock() else { return };
        if let Some(entry) = table.get(&(dimension, at)) {
            event.output = combined(&table, dimension, entry).0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machines::diodes_signal::fake::Registry;

    fn entry(controller: Pos, width: i32, length: i32, fullness: f32, non_empty: usize) -> Entry {
        Entry { controller, width, length, axis: Axis::X, fullness, non_empty, seen: 0 }
    }

    #[test]
    fn a_comparator_beside_any_block_reads_the_whole_vault() {
        let mut table = HashMap::new();
        // Two blocks along x: the first holds 10 full stacks, the second nothing.
        table.insert((0, [0, 0, 0]), entry([0, 0, 0], 1, 2, 10.0, 10));
        table.insert((0, [1, 0, 0]), entry([0, 0, 0], 1, 2, 0.0, 0));
        let at_first = combined(&table, 0, &table[&(0, [0, 0, 0])]).0;
        let at_second = combined(&table, 0, &table[&(0, [1, 0, 0])]).0;
        assert_eq!(at_first, at_second);
        // 10 stacks of 40 slots: floor(0.25 * 14) + 1.
        assert_eq!(at_first, 4);
        // A block that has not ticked yet counts as empty, not as missing.
        table.remove(&(0, [1, 0, 0]));
        assert_eq!(combined(&table, 0, &table[&(0, [0, 0, 0])]).0, 4);
    }

    #[test]
    fn a_vault_is_told_by_its_large_and_horizontal_axis() {
        let mut level = Registry::default();
        level.block(&[("large", &["false", "true"]), ("axis", &["x", "z"])]);
        // A pillar has `axis` with y and no `large`.
        level.block(&[("axis", &["x", "y", "z"])]);
        let vault = level.find(&[("large", "true"), ("axis", "z")]);
        assert!(is_vault(&level, vault));
        let pillar = level.find(&[("axis", "y")]);
        assert!(!is_vault(&level, pillar));
    }

    #[test]
    fn the_saved_state_round_trips_and_a_controller_saves_no_controller() {
        let pos = [4, 5, 6];
        let mut part = Part::new(pos, Axis::Z);
        part.update_connectivity = false;
        part.width = 2;
        part.length = 3;
        let mut nbt = Compound::new();
        for (key, value) in part_tags(pos, &part) {
            if let Some(value) = value {
                nbt.insert(key.into(), value);
            }
        }
        assert!(read_pos(&nbt).is_none());
        assert_eq!(read_part(&nbt, pos, Axis::Z), part);
        let member = Part { controller: [0, 0, 0], ..part.clone() };
        assert_eq!(part_tags(pos, &member)[0].1, Some(Tag::IntArray(vec![0, 0, 0])));
    }
}
