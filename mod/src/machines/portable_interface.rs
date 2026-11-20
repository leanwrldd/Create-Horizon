//! The Portable Storage Interface and the Portable Fluid Interface (`PortableStorageInterfaceBlock`,
//! `PortableStorageInterfaceBlockEntity`, `PortableItemInterfaceBlockEntity`,
//! `PortableFluidInterfaceBlockEntity`, `PortableStorageInterfaceRenderer`): a stationary
//! interface that a moving one on a structure links with when the structure stops facing it, and
//! that then stands for all the structure's inventories (or tanks) to funnels, chutes, pipes and
//! pumps. The link holds while contents move and lets the structure go after the idle timeout
//! (60 ticks, `CLogistics.psiTimeout`). The moving half is `portable_interface_motion.rs`, the
//! structure's storage `portable_interface_storage.rs`, and the rules (the link's state machine
//! and timeouts, the finding of the other interface, the storage maths)
//! `create_kinetics::rules::portable_interface`; this file is the stationary block.
//!
//! | Block | Behaviour |
//! |---|---|
//! | `create:portable_storage_interface` | Links with a moving one; while linked, funnels and chutes insert into and take from the structure's chests, barrels, shulker boxes and vaults |
//! | `create:portable_fluid_interface` | Links the same way; while linked, a pump or pipe fills and drains the structure's Fluid Tanks |
//!
//! Redstone power keeps an interface from linking and lets go of one that is. The middle and the top
//! come out of the base as the link forms and go back in as it lets go.
//!
//! Known differences from Create: see `rules/portable_interface.rs` and `portable_interface_motion.rs`.
//! Also, the comparator reads nothing (`getAnalogOutputSignal`), a hopper beside the interface
//! can't reach the structure's items (Horizon's hoppers see only a block's own container), the
//! Mechanical Arm has no point type for it, a Packager is not in this port, and the top's yaw about
//! its axis may differ from Create's (the partial models are turned to the facing by one rotation).

use std::collections::HashMap;
use std::sync::Mutex;

use create_kinetics::rules::bearing::up_to;
use create_kinetics::rules::portable_interface::{self as rules, Link, TIMEOUT};
use create_kinetics::{Axis, Dir, Pos, looking};
use crate::compat::{
    BlockEntityEvent, BlockEntityRenderBoundingBoxEvent, BlockEntitySubmitEvent, BlockEntityTickEvent,
    BlockEntityTicksEvent, BlockEntityUpdateTagEvent, BlockStateId, Compound, CustomBlockFluidsEvent,
    CustomFluidDrainEvent, CustomFluidFillEvent, CustomFluidTanksEvent, Flow, Hooks, Host, ItemStack,
    LevelHost, ModBlockEntity, NativeFluidAction, NativeFluidDrain, NativeFluidStack, NativeFluidTankView,
    StandaloneModelsEvent, StateForPlacementEvent, Tag, UseItemOnEvent,
};

use super::portable_interface_storage as storage;
use crate::util;

pub(crate) const ITEMS: &str = rules::ITEM_INTERFACE;
pub(crate) const FLUIDS: &str = rules::FLUID_INTERFACE;

/// `AllPartialModels.PORTABLE_STORAGE_INTERFACE_MIDDLE`, `_MIDDLE_POWERED`, `_TOP` and the fluid ones.
const MODELS: [&str; 6] = [
    "create:block/portable_storage_interface/block_middle",
    "create:block/portable_storage_interface/block_middle_powered",
    "create:block/portable_storage_interface/block_top",
    "create:block/portable_fluid_interface/block_middle",
    "create:block/portable_fluid_interface/block_middle_powered",
    "create:block/portable_fluid_interface/block_top",
];

// Saved as `PortableStorageInterfaceBlockEntity.write` names them: `Timer` and `Distance`, here with
// the game time the timer was true at (`Since`) and whether a structure holds the link (`Hold`).
const TIMER: &str = "Timer";
const SINCE: &str = "Since";
const HOLD: &str = "Hold";
const DISTANCE: &str = "Distance";
const POWERED: &str = "Powered";

/// How long a block that stopped ticking stays in the table.
const STALE: i64 = 400;

struct Interfaces;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Interfaces)
}

pub(crate) fn is_ours(block: &str) -> bool {
    rules::is_interface(block)
}

// --- the link table -----------------------------------------------------------------------------------------

struct Entry {
    link: Link,
    seen: i64,
}

type Key = (String, Pos);

static LINKS: Mutex<Option<HashMap<Key, Entry>>> = Mutex::new(None);

fn with_table<R>(f: impl FnOnce(&mut HashMap<Key, Entry>) -> R) -> R {
    let mut guard = LINKS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    f(guard.get_or_insert_with(HashMap::new))
}

/// Runs `f` on the stationary interface's link at `pos` (a fresh one if it has none yet). The
/// server's authority: the saved snapshot below only tells the clients.
pub(crate) fn with_link<R>(dimension: &str, pos: Pos, now: i64, f: impl FnOnce(&mut Link) -> R) -> R {
    with_table(|table| {
        let entry = table
            .entry((dimension.to_owned(), pos))
            .or_insert_with(|| Entry { link: Link::default(), seen: now });
        entry.seen = now;
        f(&mut entry.link)
    })
}

/// A copy of the link, if the interface has one.
pub(crate) fn peek(dimension: &str, pos: Pos) -> Option<Link> {
    with_table(|table| table.get(&(dimension.to_owned(), pos)).map(|entry| entry.link))
}

fn prune(now: i64) {
    with_table(|table| table.retain(|_, entry| now - entry.seen < STALE));
}

/// Writes the clients' snapshot of a link into a block entity: the timer as it is `now`, whether a
/// structure holds it and how far it reaches. Only what differs; whether anything did.
pub(crate) fn write_snapshot(entity: &mut ModBlockEntity, link: &Link, now: i64) -> bool {
    let mut changed = false;
    let mut put = |key: &str, value: Tag| {
        if util::differs(&entity.nbt, key, &value) {
            entity.nbt.insert(key.into(), value);
            changed = true;
        }
    };
    put(TIMER, Tag::Int(link.timer));
    put(SINCE, Tag::Long(now));
    put(HOLD, Tag::Byte(i8::from(link.keep_alive > 0)));
    put(POWERED, Tag::Byte(i8::from(link.powered)));
    drop(put);
    if util::float(&entity.nbt, DISTANCE) != Some(link.distance) {
        util::set_float(&mut entity.nbt, DISTANCE, link.distance);
        changed = true;
    }
    changed
}

/// [`write_snapshot`] from outside the block entity's tick, and tells the clients.
pub(crate) fn publish(host: &mut dyn Host, pos: Pos, link: &Link, now: i64) {
    let link = *link;
    let mut changed = false;
    host.change_mod_block_entity(pos, &mut |entity| {
        changed |= write_snapshot(entity, &link, now);
    });
    if changed {
        host.block_entity_sync(pos);
    }
}

// --- the structure behind a link -----------------------------------------------------------------------------

/// The controller and structure a stationary interface is linked with, if it may transfer now: it
/// is linked, connected, and the controller still runs the structure.
pub(crate) fn connected(host: &dyn Host, pos: Pos) -> Option<(Pos, super::contraption::Moving)> {
    let link = peek(host.dimension(), pos)?;
    if !link.can_transfer(TIMEOUT) {
        return None;
    }
    let controller = link.controller?;
    Some((controller, storage::load(host, controller)?))
}

/// `onContentTransferred`.
pub(crate) fn transferred(host: &mut dyn Host, pos: Pos) {
    let now = util::game_time(&*host);
    let link = with_link(host.dimension(), pos, now, |link| {
        link.content_transferred(TIMEOUT);
        *link
    });
    publish(host, pos, &link, now);
}

// --- items: what funnels and chutes reach ------------------------------------------------------------------------

/// The stacks of the structure's combined inventory by slot, while the interface may transfer
/// (`getStackInSlot` of the handler; an unlinked one holds nothing).
pub(crate) fn items_view(host: &dyn Host, pos: Pos) -> Option<Vec<(usize, ItemStack)>> {
    if host.block(pos) != ITEMS {
        return None;
    }
    let (_, moving) = connected(host, pos)?;
    let items = storage::items_of(host, &moving);
    Some(
        items
            .inventory
            .filled()
            .filter_map(|(slot, stack)| Some((slot, items.to_stack(stack)?)))
            .collect(),
    )
}

/// `InterfaceItemHandler.extractItem`: up to `amount` from the combined slot.
pub(crate) fn extract_slot(host: &mut dyn Host, pos: Pos, slot: usize, amount: u32) -> Option<ItemStack> {
    let (controller, mut moving) = connected(&*host, pos)?;
    let mut items = storage::items_of(&*host, &moving);
    let taken = items.inventory.extract(slot, i32::try_from(amount).ok()?, false)?;
    let out = items.to_stack(&taken)?;
    storage::apply_items(&mut moving, &items);
    storage::save(host, controller, moving);
    transferred(host, pos);
    Some(out)
}

/// `InterfaceItemHandler.insertItem` over the whole inventory (`ItemHandlerHelper.insertItemStacked`):
/// what is left of `stack`. A simulation changes nothing.
pub(crate) fn insert(host: &mut dyn Host, pos: Pos, stack: &ItemStack, simulate: bool) -> ItemStack {
    let Some((controller, mut moving)) = connected(&*host, pos) else {
        return stack.clone();
    };
    let mut items = storage::items_of(&*host, &moving);
    let wanted = items.from_stack(&*host, stack);
    let left = items.inventory.insert_stacked(&wanted, simulate);
    let mut rest = stack.clone();
    rest.count = u32::try_from(left.max(0)).unwrap_or(0);
    if !simulate && rest.count != stack.count {
        storage::apply_items(&mut moving, &items);
        storage::save(host, controller, moving);
        transferred(host, pos);
    }
    rest
}

// --- fluids: what pumps and pipes reach ----------------------------------------------------------------------------

fn fluid_views(host: &dyn Host, pos: Pos) -> Vec<NativeFluidTankView> {
    let transfer = connected(host, pos);
    let Some((_, moving)) = transfer else {
        return Vec::new();
    };
    storage::tanks_of(&moving)
        .view()
        .into_iter()
        .enumerate()
        .map(|(tank, (content, capacity))| NativeFluidTankView {
            tank,
            content: content.map(|(fluid, amount)| NativeFluidStack { fluid, amount, components: Compound::new() }),
            capacity,
            can_fill: true,
            can_drain: true,
        })
        .collect()
}

fn fluid_fill(host: &mut dyn Host, pos: Pos, fluid: &str, amount: i32, simulate: bool) -> i32 {
    let Some((controller, mut moving)) = connected(&*host, pos) else {
        return 0;
    };
    let mut tanks = storage::tanks_of(&moving);
    let filled = tanks.fill(fluid, amount, simulate);
    if !simulate && filled > 0 {
        storage::apply_tanks(&mut moving, &tanks);
        storage::save(host, controller, moving);
        transferred(host, pos);
    }
    filled
}

fn fluid_drain(host: &mut dyn Host, pos: Pos, only: Option<&str>, max: i32, simulate: bool) -> Option<(String, i32)> {
    let (controller, mut moving) = connected(&*host, pos)?;
    let mut tanks = storage::tanks_of(&moving);
    let drained = tanks.drain(only, max, simulate)?;
    if !simulate {
        storage::apply_tanks(&mut moving, &tanks);
        storage::save(host, controller, moving);
        transferred(host, pos);
    }
    Some(drained)
}

// --- the hooks -------------------------------------------------------------------------------------------------------

fn fresh_entity() -> Compound {
    let mut nbt = Compound::new();
    nbt.insert(TIMER.into(), Tag::Int(0));
    nbt.insert(SINCE.into(), Tag::Long(0));
    nbt.insert(HOLD.into(), Tag::Byte(0));
    nbt.insert(POWERED.into(), Tag::Byte(0));
    util::set_float(&mut nbt, DISTANCE, 0.0);
    nbt
}

impl Hooks for Interfaces {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        for model in MODELS {
            event.models.push(model.to_owned());
        }
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        is_ours(event.block).then(|| (event.block.to_owned(), fresh_entity()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if is_ours(event.block) {
            event.ticks = true;
        }
    }

    /// `PortableStorageInterfaceBlockEntity.tick` and `neighbourChanged` (power, from the neighbour event: `signals`).
    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if !is_ours(&event.entity.id) {
            return;
        }
        let pos = event.pos;
        let dimension = host.dimension();
        let now = util::game_time(&*host);
        let powered = crate::signals::read(host, &mut event.entity.nbt, pos) > 0;
        let (step, power, link) = with_link(dimension, pos, now, |link| {
            let power = link.power(powered);
            let step = link.tick(TIMEOUT);
            (step, power, *link)
        });
        if step.send || step.stopped || power.is_some() {
            if write_snapshot(event.entity, &link, now) {
                host.block_entity_sync(pos);
            }
        }
        if now.rem_euclid(200) == 0 {
            prune(now);
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        let entity = event.entity;
        if !is_ours(&entity.id) {
            return None;
        }
        let mut tag = Compound::new();
        tag.insert(TIMER.into(), Tag::Int(util::int(&entity.nbt, TIMER)));
        tag.insert(SINCE.into(), Tag::Long(util::long(&entity.nbt, SINCE)));
        tag.insert(HOLD.into(), Tag::Byte(i8::from(util::boolean(&entity.nbt, HOLD))));
        tag.insert(POWERED.into(), Tag::Byte(i8::from(util::boolean(&entity.nbt, POWERED))));
        util::set_float(&mut tag, DISTANCE, util::float(&entity.nbt, DISTANCE).unwrap_or(0.0));
        Some(tag)
    }

    /// `PortableStorageInterfaceRenderer`: the middle and the top, out along the facing by how far
    /// the link has come.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if !is_ours(event.block) {
            return;
        }
        let Some(facing) = host.block_state_property(event.state, "facing").and_then(Dir::from_name) else {
            return;
        };
        let nbt = &event.entity.nbt;
        let (timer, since, hold) = (util::int(nbt, TIMER), util::long(nbt, SINCE), util::boolean(nbt, HOLD));
        let distance = util::float(nbt, DISTANCE).unwrap_or(0.0);
        let draw = &mut *event.draw;
        let now = draw.game_time;
        let current = rules::timer_at(timer, since, hold, now);
        let extension = rules::extension(rules::progress(current, TIMEOUT), distance);
        let lit = current >= rules::ANIMATION && current <= TIMEOUT + rules::ANIMATION;
        draw_parts(host, draw, event.block, facing, lit, extension);
        // Nothing here moves while there is no link and no timer running.
        if timer == 0 && !hold {
            draw.keep();
        }
    }

    fn block_entity_render_bounding_box(&self, _: &dyn Host, event: &mut BlockEntityRenderBoundingBoxEvent<'_>) {
        if is_ours(event.block) {
            // `createRenderBoundingBox`: `inflate(2)`.
            for i in 0..3 {
                event.bounds[i] -= 2.0;
                event.bounds[i + 3] += 2.0;
            }
        }
    }

    /// `PortableStorageInterfaceBlock.getStateForPlacement`.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if !is_ours(&context.item) {
            return None;
        }
        let facing = rules::placement_facing(looking(context.yaw, context.pitch), context.sneaking);
        level.state_with_property(event.state, "facing", facing.name()).map(Some)
    }

    /// The Wrench turns the block (`WrenchableDirectionalBlock`) and picks it up.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || !is_ours(&host.block(event.pos)) {
            return Flow::Continue;
        }
        let (pos, clicked) = (event.pos, Dir::from_normal(event.face));
        util::wrench(host, pos, |host| {
            let (Some(facing), Some(clicked)) = (util::facing(host, pos, "facing"), clicked) else {
                return;
            };
            let next = if facing.axis() == clicked.axis() {
                facing.opposite()
            } else {
                create_kinetics::rules::drill::clockwise(facing, clicked.axis())
            };
            if next != facing {
                host.set_block_property(pos, "facing", next.name());
            }
        })
    }

    // --- the fluid interface's handler ---

    fn custom_block_fluids(&self, _: &dyn Host, event: &mut CustomBlockFluidsEvent<'_>) {
        if event.block == FLUIDS {
            event.custom = true;
        }
    }

    fn custom_fluid_tanks(&self, host: &dyn Host, event: &mut CustomFluidTanksEvent) -> Option<Vec<NativeFluidTankView>> {
        (host.block(event.pos) == FLUIDS).then(|| fluid_views(host, event.pos))
    }

    fn custom_fluid_fill(&self, host: &mut dyn Host, event: &mut CustomFluidFillEvent<'_>) -> Option<i32> {
        if host.block(event.pos) != FLUIDS {
            return None;
        }
        let resource = event.resource;
        if !resource.components.is_empty() || resource.amount <= 0 {
            return Some(0);
        }
        let simulate = event.action != NativeFluidAction::Execute;
        Some(fluid_fill(host, event.pos, &resource.fluid, resource.amount, simulate))
    }

    fn custom_fluid_drain(&self, host: &mut dyn Host, event: &mut CustomFluidDrainEvent<'_>) -> Option<NativeFluidStack> {
        if host.block(event.pos) != FLUIDS {
            return None;
        }
        let (max, only) = match event.request {
            NativeFluidDrain::Any(max) => (*max, None),
            NativeFluidDrain::Exact(stack) => {
                if !stack.components.is_empty() {
                    return None;
                }
                (stack.amount, Some(stack.fluid.clone()))
            }
            _ => return None,
        };
        let simulate = event.action != NativeFluidAction::Execute;
        let (fluid, amount) = fluid_drain(host, event.pos, only.as_deref(), max, simulate)?;
        Some(NativeFluidStack { fluid, amount, components: Compound::new() })
    }
}

// --- drawing ---------------------------------------------------------------------------------------------------------

/// `PortableStorageInterfaceRenderer.render`: the middle (lit while connected) and the top, turned to
/// the facing and moved along it. Under the draw's pose; the base is in the chunk mesh.
pub(crate) fn draw_parts(
    host: &dyn Host,
    draw: &mut crate::compat::BlockEntityDraw,
    block: &str,
    facing: Dir,
    lit: bool,
    extension: f32,
) {
    let family = if block == FLUIDS { "portable_fluid_interface" } else { "portable_storage_interface" };
    let middle = if lit {
        format!("create:block/{family}/block_middle_powered")
    } else {
        format!("create:block/{family}/block_middle")
    };
    let top = format!("create:block/{family}/block_top");
    let (middle_up, top_up) = rules::middle_and_top(extension);
    let (axis, degrees): (Axis, f32) = up_to(facing);
    for (model, up) in [(middle, middle_up), (top, top_up)] {
        draw.push();
        if degrees != 0.0 {
            draw.rotate_about(axis.unit(), degrees, [0.5; 3]);
        }
        draw.translate([0.0, up, 0.0]);
        host.draw_model(draw, &model, 0xFF_FFFF);
        draw.pop();
    }
}
