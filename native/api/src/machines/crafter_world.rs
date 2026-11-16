//! The Mechanical Crafters in the world: reading a crafter's block entity into the rules'
//! `Crafter`, gathering the crafters of a grid into a `World`, writing back what the rules
//! changed, and the inventory a grid of joined crafters is (`ConnectedInput.getItemHandler`,
//! `CombinedInvWrapper`) for hands, funnels, chutes and arms. The decisions are
//! `create_kinetics::rules::crafter`, `crafter_grid` and `crafter_recipes`; the module's hooks
//! are `crafter.rs`.
//!
//! Saved as Create saves it (`MechanicalCrafterBlockEntity.write`): `GroupedItems` (`Grid`: `x`,
//! `y`, `item`), `Phase`, `CountDown`, `Cover` and `ConnectedInput` (`Controller`, `Data`: `X`,
//! `Y`, `Z`). The one slot of the inventory is the block entity's own item slot 0 (Horizon's
//! container, so hoppers fill it). Added: `BeforeCraft` (`groupedItemsBeforeCraft`, kept by the
//! server because a client has no previous update to take it from), `Powered`
//! (`wasPoweredBefore`), and the pair `Anchor` and `Since` a client counts the animation from.

use std::collections::{BTreeMap, VecDeque};

use create_kinetics::rules::crafter::{self as rules, Input, Inputs, Pointing};
use create_kinetics::rules::crafter_grid::{Crafter, Effect, Env, Grid, Phase, Sound, World};
use create_kinetics::rules::crafter_recipes::crafting_remainder;
use create_kinetics::{Dir, Pos};
use horizon_client::mod_api::{Compound, Hooks, Host, ItemStack, ModBlockEntity, Tag};

use super::{chute_inv, crafter_recipes};
use crate::{belt, kinetic, util};

/// A module of shared code: no hooks of its own.
struct Shared;

impl Hooks for Shared {}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

pub const ID: &str = "create:mechanical_crafter";
pub const COVER: &str = "create:crafter_slot_cover";

pub const GROUPED: &str = "GroupedItems";
pub const BEFORE: &str = "BeforeCraft";
pub const PHASE: &str = "Phase";
pub const COUNT_DOWN: &str = "CountDown";
pub const COVERED: &str = "Cover";
pub const INPUT: &str = "ConnectedInput";
pub const POWERED: &str = "Powered";
pub const ANCHOR: &str = "Anchor";
pub const SINCE: &str = "Since";

/// How many crafters a gather reads at most.
const LIMIT: usize = 64;
/// `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;

fn int_of(tag: Option<&Tag>) -> i32 {
    match tag {
        Some(Tag::Int(v)) => *v,
        Some(Tag::Byte(v)) => i32::from(*v),
        _ => 0,
    }
}

fn string_of(nbt: &Compound, key: &str) -> String {
    match util::get(nbt, key) {
        Some(Tag::String(s)) => s.to_string_lossy().to_string(),
        _ => String::new(),
    }
}

// --- the block entity's NBT -----------------------------------------------------------------

/// `GroupedItems.write`: the cells under `Grid`, an empty stack as an empty compound.
pub fn grid_tag(grid: &Grid<ItemStack>) -> Tag {
    let list = grid
        .cells
        .iter()
        .map(|((x, y), stack)| {
            let mut entry = Compound::new();
            entry.insert("x".into(), Tag::Int(*x));
            entry.insert("y".into(), Tag::Int(*y));
            let item = stack.as_ref().map_or_else(|| Tag::Compound(Compound::new()), util::stack_tag);
            entry.insert("item".into(), item);
            Tag::Compound(entry)
        })
        .collect();
    let mut tag = Compound::new();
    tag.insert("Grid".into(), Tag::List(list));
    Tag::Compound(tag)
}

/// `GroupedItems.read`.
pub fn read_grid(nbt: &Compound, key: &str) -> Grid<ItemStack> {
    let mut grid = Grid::default();
    if let Some(Tag::Compound(tag)) = util::get(nbt, key)
        && let Some(Tag::List(list)) = util::get(tag, "Grid")
    {
        for entry in list {
            let cell = (int_of(entry.get("x")), int_of(entry.get("y")));
            grid.cells.insert(cell, entry.get("item").and_then(util::stack_of));
        }
    }
    grid
}

/// `ConnectedInput.read`.
pub fn read_input(nbt: &Compound) -> Input {
    let Some(Tag::Compound(tag)) = util::get(nbt, INPUT) else {
        return Input::default();
    };
    let data = match util::get(tag, "Data") {
        Some(Tag::List(list)) => list
            .iter()
            .map(|e| [int_of(e.get("X")), int_of(e.get("Y")), int_of(e.get("Z"))])
            .collect(),
        _ => Vec::new(),
    };
    Input { controller: util::boolean(tag, "Controller"), data }.or_reset()
}

/// `ConnectedInput.write`.
pub fn input_tag(input: &Input) -> Tag {
    let list = input
        .data
        .iter()
        .map(|[x, y, z]| {
            let mut entry = Compound::new();
            entry.insert("X".into(), Tag::Int(*x));
            entry.insert("Y".into(), Tag::Int(*y));
            entry.insert("Z".into(), Tag::Int(*z));
            Tag::Compound(entry)
        })
        .collect();
    let mut tag = Compound::new();
    tag.insert("Controller".into(), Tag::Byte(i8::from(input.controller)));
    tag.insert("Data".into(), Tag::List(list));
    Tag::Compound(tag)
}

fn slot_of(entity: &ModBlockEntity) -> Option<ItemStack> {
    entity.items.first().cloned().flatten()
}

/// The crafter the block at `pos` and its block entity make; `None` for a block that isn't a
/// crafter.
pub fn read(host: &dyn Host, pos: Pos, entity: &ModBlockEntity) -> Option<Crafter<ItemStack>> {
    let facing = util::facing(host, pos, "facing")?;
    let pointing = Pointing::from_name(&host.block_property(pos, "pointing")?)?;
    let nbt = &entity.nbt;
    Some(Crafter {
        facing,
        pointing,
        phase: Phase::from_name(&string_of(nbt, PHASE)),
        count_down: util::int(nbt, COUNT_DOWN),
        covered: util::boolean(nbt, COVERED),
        slot: slot_of(entity),
        grid: read_grid(nbt, GROUPED),
        before_craft: read_grid(nbt, BEFORE),
        speed: kinetic::speed(entity),
        was_powered: if util::has(nbt, POWERED) { util::boolean(nbt, POWERED) } else { true },
    })
}

/// Writes what `new` has different from `old` into the entity (reaching its fields mutably
/// counts as a change, so nothing is touched for what is the same). Whether a client would see
/// something different (the phase, what is held, the cover): sync then.
pub fn write(
    entity: &mut ModBlockEntity,
    old: &Crafter<ItemStack>,
    new: &Crafter<ItemStack>,
    now: i64,
) -> bool {
    let visible = old.phase != new.phase
        || old.grid != new.grid
        || old.before_craft != new.before_craft
        || old.slot != new.slot
        || old.covered != new.covered;
    if old.phase != new.phase {
        entity.nbt.insert(PHASE.into(), Tag::String(new.phase.name().into()));
    }
    if old.count_down != new.count_down {
        entity.nbt.insert(COUNT_DOWN.into(), Tag::Int(new.count_down));
    }
    if old.covered != new.covered {
        entity.nbt.insert(COVERED.into(), Tag::Byte(i8::from(new.covered)));
    }
    if old.grid != new.grid {
        entity.nbt.insert(GROUPED.into(), grid_tag(&new.grid));
    }
    if old.before_craft != new.before_craft {
        entity.nbt.insert(BEFORE.into(), grid_tag(&new.before_craft));
    }
    if old.was_powered != new.was_powered {
        entity.nbt.insert(POWERED.into(), Tag::Byte(i8::from(new.was_powered)));
    }
    if old.slot != new.slot {
        if entity.items.is_empty() {
            entity.items.push(None);
        }
        entity.items[0] = new.slot.clone();
    }
    if visible {
        // The client counts the animation from here (`CountDown` as of game time `Since`).
        entity.nbt.insert(ANCHOR.into(), Tag::Int(new.count_down));
        entity.nbt.insert(SINCE.into(), Tag::Long(now));
    }
    visible
}

// --- gathering a grid ---------------------------------------------------------------------------

/// The crafters of a grid as the rules see them, and what they were before.
pub struct Snapshot {
    pub world: World<ItemStack>,
    original: BTreeMap<Pos, Crafter<ItemStack>>,
}

/// The crafters that touch each other from `pos` (the same facing, side by side or above and
/// below), which holds `own` (a block entity in its own tick can't be read through the host).
/// Any grid is among them: the rules follow the arrows.
pub fn gather(host: &dyn Host, pos: Pos, own: &ModBlockEntity) -> Option<Snapshot> {
    let mut found: BTreeMap<Pos, Crafter<ItemStack>> = BTreeMap::new();
    let mut queue = VecDeque::from([pos]);
    while let Some(at) = queue.pop_front() {
        if found.contains_key(&at) || found.len() >= LIMIT {
            continue;
        }
        let crafter = if at == pos {
            read(host, at, own)
        } else {
            host.mod_block_entity(at).and_then(|e| read(host, at, &e))
        };
        let Some(crafter) = crafter else { continue };
        let facing = crafter.facing;
        found.insert(at, crafter);
        for dir in Dir::ALL {
            if dir.axis() == facing.axis() {
                continue;
            }
            let next = util::add(at, dir.normal());
            if !found.contains_key(&next)
                && host.block(next) == ID
                && util::facing(host, next, "facing") == Some(facing)
            {
                queue.push_back(next);
            }
        }
    }
    found.contains_key(&pos).then(|| Snapshot { original: found.clone(), world: World::new(found) })
}

/// A snapshot of one crafter alone, for a step that needs nothing of its neighbours.
pub fn alone(pos: Pos, crafter: Crafter<ItemStack>) -> Snapshot {
    let found = BTreeMap::from([(pos, crafter)]);
    Snapshot { original: found.clone(), world: World::new(found) }
}

/// [`gather`] for a crafter that isn't in its own tick.
pub fn gather_at(host: &dyn Host, pos: Pos) -> Option<Snapshot> {
    let own = host.mod_block_entity(pos)?;
    gather(host, pos, &own)
}

/// Writes back what the rules changed. `own` is the crafter in its own tick, whose entity is
/// written directly.
pub fn commit(
    host: &mut dyn Host,
    snapshot: &Snapshot,
    mut own: Option<(Pos, &mut ModBlockEntity)>,
) {
    let now = util::game_time(&*host);
    for pos in &snapshot.world.changed {
        let (Some(old), Some(new)) = (snapshot.original.get(pos), snapshot.world.crafters.get(pos)) else {
            continue;
        };
        if old == new {
            continue;
        }
        let is_own = own.as_ref().is_some_and(|(at, _)| at == pos);
        if is_own && let Some((_, entity)) = own.as_mut() {
            // Its own tick syncs it (`own_visible`).
            write(entity, old, new, now);
            continue;
        }
        let mut visible = false;
        host.change_mod_block_entity(*pos, &mut |entity| {
            visible = write(entity, old, new, now);
        });
        if visible {
            host.block_entity_sync(*pos);
        }
    }
}

/// Whether the crafter in its own tick has something a client should see (the glue syncs it
/// from the tick).
pub fn own_visible(snapshot: &Snapshot, pos: Pos) -> bool {
    match (snapshot.original.get(&pos), snapshot.world.crafters.get(&pos)) {
        (Some(old), Some(new)) => {
            old.phase != new.phase
                || old.grid != new.grid
                || old.before_craft != new.before_craft
                || old.slot != new.slot
                || old.covered != new.covered
        }
        _ => false,
    }
}

// --- the game's answers ------------------------------------------------------------------------------

/// What the rules ask the game while a chain steps.
pub struct GameEnv<'a> {
    pub host: &'a mut dyn Host,
    /// The direction each crafter of the snapshot points (`getTargetDirection`).
    aims: BTreeMap<Pos, Dir>,
}

impl<'a> GameEnv<'a> {
    pub fn new(host: &'a mut dyn Host, snapshot: &Snapshot) -> Self {
        let aims = snapshot
            .world
            .crafters
            .iter()
            .map(|(pos, c)| (*pos, c.target_direction()))
            .collect();
        GameEnv { host, aims }
    }

    fn aim(&self, pos: Pos) -> Option<(Pos, Dir)> {
        let dir = *self.aims.get(&pos)?;
        Some((util::add(pos, dir.normal()), dir))
    }
}

impl Env<ItemStack> for GameEnv<'_> {
    fn apply_recipe(&mut self, grid: &Grid<ItemStack>) -> Option<ItemStack> {
        crafter_recipes::apply(&*self.host, grid)
    }

    fn remainder(&mut self, stack: &ItemStack) -> Option<ItemStack> {
        crafting_remainder(&stack.item).map(|id| ItemStack::new(id, 1))
    }

    fn has_inventory(&mut self, pos: Pos) -> bool {
        self.aim(pos).is_some_and(|(target, _)| chute_inv::is_inventory(self.host, target))
    }

    fn targeting_belt(&mut self, pos: Pos) -> bool {
        self.aim(pos)
            .is_some_and(|(target, dir)| belt::can_insert_from_side(&*self.host, target, dir) == Some(true))
    }

    /// `behaviour.handleInsertion(stack, face, false)` for a belt-like input, else
    /// `inserting.insert(stack.copy())` into the inventory there.
    fn insert(&mut self, pos: Pos, stack: ItemStack) -> Option<ItemStack> {
        let (target, dir) = self.aim(pos)?;
        let rest = if belt::can_insert_from_side(&*self.host, target, dir) == Some(true) {
            let angle = (util::random() % 360) as i32;
            let item = create_kinetics::belt::Item::dropped(stack.clone(), 0, angle, true);
            belt::handle_insertion(self.host, target, dir, &item, false, false)
        } else {
            chute_inv::insert(self.host, target, dir.opposite(), &stack, false)
        };
        match rest {
            Some(rest) if rest.count > 0 => Some(rest),
            Some(_) => None,
            // Nothing there took anything.
            None => Some(stack),
        }
    }
}

/// Plays and drops what a step did (`AllSoundEvents.CRAFTER_CLICK`, `CRAFTER_CRAFT`, `dropItem`).
pub fn apply_effects(host: &mut dyn Host, world: &World<ItemStack>, effects: Vec<Effect<ItemStack>>) {
    for effect in effects {
        match effect {
            Effect::Sound(pos, Sound::Click(pitch)) => {
                let at = util::centre(pos);
                host.play_sound("minecraft:block.netherite_block.hit", at, BLOCKS, 0.25, pitch);
                host.play_sound("minecraft:block.wooden_trapdoor.open", at, BLOCKS, 0.125, pitch);
            }
            Effect::Sound(pos, Sound::Craft) => {
                host.play_sound("minecraft:entity.item.break", util::centre(pos), BLOCKS, 0.125, 0.75);
            }
            Effect::Drop(pos, stack) => {
                // `eject`: 0.75 out of the front.
                let front = world.crafters.get(&pos).map_or([0; 3], |c| c.facing.normal());
                let at = [0, 1, 2].map(|i| f64::from(pos[i]) + 0.5 + f64::from(front[i]) * 0.75);
                util::spawn_item(host, at, &stack, [0.0; 3], 10);
            }
        }
    }
}

// --- starting and ejecting a grid ------------------------------------------------------------------------

/// `checkCompletedRecipe(false)` for the crafter at `pos`, outside its own tick.
pub fn start_check(host: &mut dyn Host, pos: Pos) {
    let Some(mut snapshot) = gather_at(&*host, pos) else {
        return;
    };
    snapshot.world.check_completed_recipe(pos, false);
    commit(host, &snapshot, None);
}

/// `ejectWholeGrid` for the crafter at `pos`, outside its own tick.
pub fn eject_whole_grid(host: &mut dyn Host, pos: Pos) {
    let Some(mut snapshot) = gather_at(&*host, pos) else {
        return;
    };
    let effects = snapshot.world.eject_whole_grid(pos);
    commit(host, &snapshot, None);
    apply_effects(host, &snapshot.world, effects);
}

// --- the inventory of a grid ---------------------------------------------------------------------------------

/// `ConnectedInput.getInventories`: the crafters whose slots make the inventory of the grid
/// the crafter at `pos` is in, in order; none when its controller doesn't lead a group.
pub fn members(host: &dyn Host, pos: Pos) -> Vec<Pos> {
    let Some(own) = host.mod_block_entity(pos) else {
        return Vec::new();
    };
    let input = read_input(&own.nbt);
    let controller = input.controller_of(pos);
    let lead = if input.controller {
        input
    } else {
        let Some(lead) = host.mod_block_entity(controller) else {
            return Vec::new();
        };
        let lead = read_input(&lead.nbt);
        if controller == pos || !lead.controller {
            return Vec::new();
        }
        lead
    };
    let facing = util::facing(host, controller, "facing").unwrap_or(Dir::South);
    let positions: Vec<Pos> = lead
        .data
        .iter()
        .map(|offset| util::add(controller, *offset))
        .filter(|p| host.block(*p) == ID)
        .collect();
    rules::inventory_order(facing, positions)
}

/// `getSlots` of the grid's inventory.
pub fn handler_slots(host: &dyn Host, pos: Pos) -> usize {
    members(host, pos).len()
}

/// `Inventory.insertItem` through the combined handler (`ItemHandlerHelper.insertItem`): one
/// item into each crafter in turn that is idle, uncovered and empty. What is left of `stack`
/// (count 0 when all went in). The grid starts when this fills it
/// (`whenContentsChanged`).
pub fn handler_insert(host: &mut dyn Host, pos: Pos, stack: &ItemStack, simulate: bool) -> ItemStack {
    let mut rest = stack.clone();
    let mut one = stack.clone();
    one.count = 1;
    let mut last = None;
    for member in members(&*host, pos) {
        if rest.count == 0 {
            break;
        }
        let Some(entity) = host.mod_block_entity(member) else {
            continue;
        };
        let idle = Phase::from_name(&string_of(&entity.nbt, PHASE)) == Phase::Idle;
        if !idle || util::boolean(&entity.nbt, COVERED) || slot_of(&entity).is_some() {
            continue;
        }
        if !simulate {
            host.change_mod_block_entity(member, &mut |e| {
                if e.items.is_empty() {
                    e.items.push(None);
                }
                e.items[0] = Some(one.clone());
            });
            host.block_entity_sync(member);
            // `ITEM_FRAME_ADD_ITEM`, quietly.
            host.play_sound("minecraft:entity.item_frame.add_item", util::centre(member), BLOCKS, 0.25, 0.5);
            last = Some(member);
        }
        rest.count -= 1;
    }
    if let Some(member) = last {
        start_check(host, member);
    }
    rest
}

/// The arm's `extractItem(slot, amount, simulate)` on the grid (`allowExtraction` for the arm
/// only): the item of the `slot`th crafter.
pub fn handler_extract(host: &mut dyn Host, pos: Pos, slot: usize, simulate: bool) -> Option<ItemStack> {
    let member = *members(&*host, pos).get(slot)?;
    let stack = slot_of(&host.mod_block_entity(member)?)?;
    if !simulate {
        host.change_mod_block_entity(member, &mut |e| {
            if let Some(first) = e.items.first_mut() {
                *first = None;
            }
        });
        host.block_entity_sync(member);
    }
    Some(stack)
}

// --- joining ---------------------------------------------------------------------------------------------------

/// `ConnectedInputHandler.toggleConnection`: joins the crafters at `pos` and `other`, or
/// parts them if they are one group.
pub fn toggle_connection(host: &mut dyn Host, pos: Pos, other: Pos) {
    let read_one = |host: &dyn Host, p: Pos| {
        (host.block(p) == ID).then(|| host.mod_block_entity(p).map(|e| read_input(&e.nbt))).flatten()
    };
    let touched = rules::touched(&|p| read_one(&*host, p), pos, other);
    let mut inputs: Inputs = touched
        .into_iter()
        .filter_map(|p| read_one(&*host, p).map(|input| (p, input)))
        .collect();
    let before = inputs.clone();
    rules::toggle(&mut inputs, pos, other);
    for (p, input) in inputs {
        if before.get(&p) == Some(&input) {
            continue;
        }
        host.change_mod_block_entity(p, &mut |e| {
            e.nbt.insert(INPUT.into(), input_tag(&input));
        });
        host.block_entity_sync(p);
    }
}

/// `MechanicalCrafterBlock.onRemove`: the crafters of the same group beside the one at `pos`
/// are parted from it, a direction at a time.
pub fn part_from_neighbours(host: &mut dyn Host, pos: Pos, facing: Dir) {
    for dir in Dir::ALL {
        if dir.axis() == facing.axis() {
            continue;
        }
        let other = util::add(pos, dir.normal());
        let (Some(a), Some(b)) = (host.mod_block_entity(pos), host.mod_block_entity(other)) else {
            continue;
        };
        if host.block(other) != ID {
            continue;
        }
        let (a, b) = (read_input(&a.nbt), read_input(&b.nbt));
        if rules::same_group((pos, &a), (other, &b)) {
            toggle_connection(host, pos, other);
        }
    }
}

// --- a Deployer's click ---------------------------------------------------------------------------------------

/// `MechanicalCrafterBlock.useItemOn` for a Deployer's fake player holding `held` (none for an
/// empty hand) on the face `face` of the crafter at `pos`. `Some(new held stack)` when the click
/// was the crafter's; `None` to carry on with the click as for any block.
pub fn deploy(
    host: &mut dyn Host,
    pos: Pos,
    face: Dir,
    held: Option<&ItemStack>,
) -> Option<Option<ItemStack>> {
    let entity = host.mod_block_entity(pos)?;
    let facing = util::facing(&*host, pos, "facing")?;
    let slot = slot_of(&entity);
    let covered = util::boolean(&entity.nbt, COVERED);
    let case = rules::UseCase {
        front: face == facing,
        idle: Phase::from_name(&string_of(&entity.nbt, PHASE)) == Phase::Idle,
        empty_main_hand: held.is_none(),
        wrench: false,
        cover_item: held.is_some_and(|h| &*h.item == COVER),
        arm: held.is_some_and(|h| &*h.item == "create:mechanical_arm"),
        covered,
        slot_empty: slot.is_none(),
        same_as_slot: matches!((held, &slot), (Some(h), Some(s)) if util::same_item(h, s)),
    };
    let shrunk = |stack: &ItemStack, by: u32| {
        let mut left = stack.clone();
        left.count = left.count.saturating_sub(by);
        (left.count > 0).then_some(left)
    };
    match rules::use_item_on(case) {
        rules::Use::Pass => None,
        rules::Use::EjectGrid => {
            eject_whole_grid(host, pos);
            Some(held.cloned())
        }
        rules::Use::PlaceCover => {
            set_cover(host, pos, true);
            Some(held.and_then(|h| shrunk(h, 1)))
        }
        rules::Use::Insert => {
            let held = held?;
            let rest = handler_insert(host, pos, held, false);
            Some(shrunk(held, held.count - rest.count))
        }
        rules::Use::Uncover => {
            set_cover(host, pos, false);
            Some(Some(ItemStack::new(COVER, 1)))
        }
        rules::Use::TakeItem => {
            take_slot(host, pos);
            Some(slot)
        }
    }
}

/// Puts the Crafter Slot Cover on or takes it off.
pub fn set_cover(host: &mut dyn Host, pos: Pos, covered: bool) {
    host.change_mod_block_entity(pos, &mut |e| {
        if util::boolean(&e.nbt, COVERED) != covered {
            e.nbt.insert(COVERED.into(), Tag::Byte(i8::from(covered)));
        }
    });
    host.block_entity_sync(pos);
}

/// Empties the slot of the crafter at `pos`.
pub fn take_slot(host: &mut dyn Host, pos: Pos) {
    host.change_mod_block_entity(pos, &mut |e| {
        if let Some(first) = e.items.first_mut() {
            *first = None;
        }
    });
    host.block_entity_sync(pos);
}
