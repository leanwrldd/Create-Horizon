//! The Fluid Tank and the Creative Fluid Tank (`FluidTankBlock`, `FluidTankBlockEntity`,
//! `CreativeFluidTankBlockEntity`, `FluidTankRenderer`): a tank holding fluid that joins its
//! neighbours into one wider, taller tank. The rules (which blocks join, where the fluid goes,
//! which `top` / `bottom` / `shape` each block shows, where the fluid is drawn) are
//! `create_kinetics::rules::fluid_tank`; this is the glue.
//!
//! The fluid is held through Horizon's custom fluid storage (`custom_fluid_tanks`, `_fill` and
//! `_drain`), so a bucket (here), a pump (`pump.rs`) or another mod's pipes reach the one
//! controller whichever block of the tank they touch.
//!
//! Known differences from Create:
//! - `FluidTankItem.tryMultiPlace` (placing a whole layer of a wide tank at once) is not done:
//!   a wide tank is built block by block and joins when the blocks stand right.
//! - A tank's fluid level is drawn as saved, with no easing (`LerpedFloat`), and sent to clients
//!   at most every 8 ticks (`SYNC_RATE`).
//! - The boiler (`BoilerData`) is `steam_engine_boiler.rs`; no light from the fluid (`luminosity`), no comparator output
//!   (`getAnalogOutputSignal`), no rotation or mirroring of windows by structures.
//! - A survival player fills and empties a tank with buckets too. Create leaves that to
//!   NeoForge's bucket handling and does it only for creative players itself
//!   (`FluidTankBlock.useItemOn`).
//! - Fluid is lost when a block breaks that isn't broken by a player (an explosion): the other
//!   blocks notice a few ticks later and split, but the controller's fluid is gone.
//! - The wrench toggles the windows of the whole tank, as Create's; the click point is not
//!   needed.

use create_kinetics::rules::fluid_tank::{self as rules, CAPACITY, Part, Region};
use create_kinetics::rules::fluids as fluid_rules;
use horizon_client::mod_api::{
    BlockEntityEvent, BlockEntityRenderBoundingBoxEvent, BlockEntitySubmitEvent,
    BlockEntityTickEvent, BlockEntityTicksEvent, BlockEntityUpdateTagEvent, Compound,
    CustomBlockFluidsEvent, CustomFluidDrainEvent, CustomFluidFillEvent, CustomFluidTanksEvent,
    DestroyBlockEvent, EquipmentSlot, Flow, Hooks, Host, ItemStack, ModBlockEntity,
    NativeFluidAction, NativeFluidDrain, NativeFluidStack, NativeFluidTankView,
    RightClickBlockEvent, Tag, UseItemEvent, UseItemOnEvent,
};

use super::fluids::{CREATIVE_FLUID_TANK, FLUID_TANK};
use crate::util;

type Pos = [i32; 3];

/// Saved as Create's `FluidTankBlockEntity.write` names them.
const CONTROLLER: &str = "Controller";
const SIZE: &str = "Size";
const HEIGHT: &str = "Height";
const WINDOW: &str = "Window";
const CONTENT: &str = "TankContent";
const UNINITIALIZED: &str = "Uninitialized";
/// Not Create's: the content clients were last told, to send a change every `SYNC_RATE` ticks.
const SYNCED: &str = "Synced";
/// `FluidTankBlockEntity.SYNC_RATE`.
const SYNC_RATE: i64 = 8;
/// How often a block checks that the tank it belongs to is still whole.
const VERIFY: i64 = 20;

/// What the boiler module (`steam_engine_boiler.rs`) plugs in through [`BOILER`], so this file
/// doesn't depend on it: while a Steam Engine or Whistle is attached to a tank it is a boiler
/// (`FluidTankBlockEntity.handlerForCapability`), taking only water and keeping none.
pub(crate) struct BoilerApi {
    /// Whether the tank whose controller is at the position is an active boiler.
    pub active: fn(&dyn Host, Pos) -> bool,
    /// `BoilerFluidHandler.fill` into the controller at the position: the fluid, the amount and
    /// whether to do it (else only ask). Answers the amount taken.
    pub fill: fn(&mut dyn Host, Pos, &str, i32, bool) -> i32,
    /// Adds the boiler's saved state to a block's update tag.
    pub update_tag: fn(&ModBlockEntity, &mut Compound),
    /// `BoilerFluidHandler.getTankCapacity`.
    pub capacity: i32,
}

/// Set once by the boiler module's `hooks()`; empty if it isn't built in.
pub(crate) static BOILER: std::sync::OnceLock<BoilerApi> = std::sync::OnceLock::new();

/// The boiler module, if the tank whose controller is at `controller` is an active boiler.
fn boiler(host: &dyn Host, controller: Pos) -> Option<&'static BoilerApi> {
    BOILER.get().filter(|api| (api.active)(host, controller))
}

struct Tanks;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Tanks)
}

fn is_tank(block: &str) -> bool {
    block == FLUID_TANK || block == CREATIVE_FLUID_TANK
}

// --- the saved state ----------------------------------------------------------------------------

fn read_pos(nbt: &Compound) -> Option<Pos> {
    match util::get(nbt, CONTROLLER) {
        Some(Tag::IntArray(v)) if v.len() == 3 => Some([v[0], v[1], v[2]]),
        _ => None,
    }
}

/// `TankContent`: `FluidStack.saveOptional`'s `id` and `amount`; nothing for an empty tank.
fn read_content(nbt: &Compound) -> (Option<String>, i32) {
    let Some(content @ Tag::Compound(_)) = util::get(nbt, CONTENT) else {
        return (None, 0);
    };
    let fluid = match content.get("id") {
        Some(Tag::String(id)) => id.to_string_lossy(),
        _ => String::new(),
    };
    let amount = match content.get("amount") {
        Some(Tag::Int(amount)) => *amount,
        _ => 0,
    };
    if fluid.is_empty() || fluid == "minecraft:empty" || amount <= 0 {
        (None, 0)
    } else {
        (Some(fluid), amount)
    }
}

fn content_tag(fluid: &Option<String>, amount: i32) -> Tag {
    let mut tag = Compound::new();
    tag.insert("id".into(), Tag::String(fluid.as_deref().unwrap_or("minecraft:empty").into()));
    tag.insert("amount".into(), Tag::Int(if fluid.is_some() { amount } else { 0 }));
    Tag::Compound(tag)
}

fn read_part(nbt: &Compound, pos: Pos, creative: bool) -> Part {
    let controller = read_pos(nbt).unwrap_or(pos);
    let width = util::int(nbt, SIZE).max(1);
    let height = util::int(nbt, HEIGHT).max(1);
    let (fluid, amount) = read_content(nbt);
    let capacity = if controller == pos { width * width * height * CAPACITY } else { CAPACITY };
    Part {
        controller,
        width,
        height,
        amount: if creative && fluid.is_some() { capacity } else { amount },
        fluid,
        capacity,
        window: !util::has(nbt, WINDOW) || util::boolean(nbt, WINDOW),
        creative,
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
        (HEIGHT, Some(Tag::Int(part.height))),
        (WINDOW, Some(Tag::Byte(i8::from(part.window)))),
        (CONTENT, Some(content_tag(&part.fluid, part.amount))),
        (UNINITIALIZED, part.update_connectivity.then_some(Tag::Byte(1))),
    ]
}

fn content_tags(part: &Part) -> Vec<(&'static str, Option<Tag>)> {
    vec![(CONTENT, Some(content_tag(&part.fluid, part.amount)))]
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
    nbt.insert(HEIGHT.into(), Tag::Int(1));
    nbt.insert(WINDOW.into(), Tag::Byte(1));
    nbt.insert(CONTENT.into(), content_tag(&None, 0));
    // `updateConnectivity`: the block looks for neighbours on its first tick.
    nbt.insert(UNINITIALIZED.into(), Tag::Byte(1));
    nbt
}

// --- the multi-block, through the game ------------------------------------------------------------

type Loader<'a> = Box<dyn FnMut(Pos) -> Option<Part> + 'a>;

fn load_part(host: &dyn Host, kind: &str, pos: Pos) -> Option<Part> {
    let entity = host.mod_block_entity(pos)?;
    (entity.id == kind).then(|| read_part(&entity.nbt, pos, kind == CREATIVE_FLUID_TANK))
}

/// A block that changed, its new state and its controller's.
type Change = (Pos, Part, Part);

/// Runs `op` over the tank blocks of `kind` around a position and answers what it changed.
/// `own` is the block entity ticking now, which `Host::mod_block_entity` can't reach.
fn solve<'a>(
    host: &'a dyn Host,
    kind: &'a str,
    own: Option<(Pos, Part)>,
    op: impl FnOnce(&mut Region<Loader<'a>>),
) -> Vec<Change> {
    let load: Loader<'a> = Box::new(move |pos| {
        if let Some((at, part)) = &own
            && *at == pos
        {
            return Some(part.clone());
        }
        load_part(host, kind, pos)
    });
    let mut region = Region::new(load);
    op(&mut region);
    let changes = region.changes();
    changes
        .iter()
        .map(|(pos, part)| {
            let controller = if part.controller == *pos {
                part.clone()
            } else {
                changes
                    .iter()
                    .find(|(at, _)| *at == part.controller)
                    .map(|(_, c)| c.clone())
                    .or_else(|| region.get(part.controller))
                    .unwrap_or_else(|| part.clone())
            };
            (*pos, part.clone(), controller)
        })
        .collect()
}

/// `notifyMultiUpdated` / `setWindows` / `removeController`: the block's `top`, `bottom` and `shape`.
fn set_states(host: &mut dyn Host, pos: Pos, controller_pos: Pos, controller: &Part) {
    let (bottom, top, shape) =
        rules::block_state(pos, controller_pos, controller.width, controller.height, controller.window);
    for (key, value) in [
        ("bottom", if bottom { "true" } else { "false" }),
        ("top", if top { "true" } else { "false" }),
        ("shape", shape.name()),
    ] {
        if host.block_property(pos, key).as_deref() != Some(value) {
            host.set_block_property(pos, key, value);
        }
    }
}

/// Writes what `solve` found: each block's saved state and block state, and tells clients.
fn apply(
    host: &mut dyn Host,
    mut own: Option<(Pos, &mut ModBlockEntity)>,
    changes: Vec<Change>,
) {
    for (pos, part, controller) in changes {
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
        // A creative tank keeps `Window` and the rest like any other.
        let controller_pos = part.controller;
        set_states(host, pos, controller_pos, &controller);
        host.block_entity_sync(pos);
    }
}

/// The controller of the tank the block at `pos` is part of, and its state. Not for the block
/// entity ticking now.
fn controller_of(host: &dyn Host, kind: &str, pos: Pos) -> Option<(Pos, Part)> {
    let part = load_part(host, kind, pos)?;
    if part.controller == pos {
        return Some((pos, part));
    }
    let controller = load_part(host, kind, part.controller)?;
    (controller.controller == part.controller).then_some((part.controller, controller))
}

fn view(part: &Part) -> NativeFluidTankView {
    NativeFluidTankView {
        tank: 0,
        content: part.fluid.clone().filter(|_| part.amount_held() > 0).map(|fluid| NativeFluidStack {
            fluid,
            amount: part.amount_held(),
            components: Compound::new(),
        }),
        capacity: part.capacity,
        can_fill: true,
        can_drain: true,
    }
}

/// Sets what a controller holds, from outside its tick.
fn store_content(host: &mut dyn Host, pos: Pos, part: &Part) {
    let tags = content_tags(part);
    host.change_mod_block_entity(pos, &mut |entity| {
        apply_tags(entity, &tags);
    });
}

// --- the tick ---------------------------------------------------------------------------------------

fn tick(host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
    let pos = event.pos;
    let kind = event.entity.id.clone();
    let creative = kind == CREATIVE_FLUID_TANK;
    let mut part = read_part(&event.entity.nbt, pos, creative);
    let now = util::game_time(host);

    // `updateConnectivity`: a block that just appeared, or was split off, looks for neighbours.
    if part.update_connectivity {
        part.update_connectivity = false;
        let changes = if part.is_controller(pos) {
            solve(&*host, &kind, Some((pos, part.clone())), |region| {
                rules::form_multi(region, pos);
            })
        } else {
            Vec::new()
        };
        let own_changed = changes.iter().any(|(at, _, _)| *at == pos);
        apply(host, Some((pos, &mut *event.entity)), changes);
        if !own_changed {
            apply_tags(event.entity, &part_tags(pos, &part));
        }
        return;
    }

    // Now and then, check that the tank is still whole: a block broken without a player
    // (an explosion) gave no warning.
    if (now + i64::from(pos[0] ^ pos[2]) + i64::from(pos[1])).rem_euclid(VERIFY) == 0 {
        if verify(host, &kind, pos, &part, event.entity) {
            return;
        }
    }

    // Clients are told of the fluid level at most every `SYNC_RATE` ticks.
    if part.is_controller(pos) && now.rem_euclid(SYNC_RATE) == 0 {
        let shown = content_tag(&part.fluid, part.amount_held());
        if util::differs(&event.entity.nbt, SYNCED, &shown) {
            event.entity.nbt.insert(SYNCED.into(), shown);
            host.block_entity_sync(pos);
        }
    }
}

/// Whether anything was done: a block that lost its controller stands alone, and a controller
/// that lost a block splits.
fn verify(
    host: &mut dyn Host,
    kind: &str,
    pos: Pos,
    part: &Part,
    entity: &mut ModBlockEntity,
) -> bool {
    let creative = kind == CREATIVE_FLUID_TANK;
    if !part.is_controller(pos) {
        let covered = load_part(host, kind, part.controller).is_some_and(|c| {
            c.controller == part.controller
                && (0..c.width).any(|x| (0..c.width).any(|z| (0..c.height).any(|y| {
                    [part.controller[0] + x, part.controller[1] + y, part.controller[2] + z] == pos
                })))
        });
        if covered {
            return false;
        }
        let mut alone = Part::new(pos, creative);
        alone.window = part.window;
        let changes = vec![(pos, alone.clone(), alone)];
        apply(host, Some((pos, entity)), changes);
        return true;
    }
    if part.width == 1 && part.height == 1 {
        return false;
    }
    for y in 0..part.height {
        for x in 0..part.width {
            for z in 0..part.width {
                let at = [pos[0] + x, pos[1] + y, pos[2] + z];
                if at == pos || host.block(at) == kind {
                    continue;
                }
                // A block is missing: the tank comes apart, keeping what fluid fits.
                let missing = Part { controller: pos, ..part.clone() };
                let changes = solve(&*host, kind, Some((pos, part.clone())), |region| {
                    rules::split_removed(region, at, missing);
                });
                apply(host, Some((pos, entity)), changes);
                return true;
            }
        }
    }
    false
}

/// `FluidTankBlockEntity.setWindows` for the controller ticking now (`entity`, which the world
/// can't read while it ticks): for the boiler, which takes its windows away.
pub(crate) fn set_windows(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity, window: bool) {
    let kind = entity.id.clone();
    let part = read_part(&entity.nbt, pos, kind == CREATIVE_FLUID_TANK);
    if !part.is_controller(pos) {
        return;
    }
    let changes = solve(&*host, &kind, Some((pos, part)), |region| {
        rules::set_windows(region, pos, window);
    });
    apply(host, Some((pos, entity)), changes);
}

fn update_tag(entity: &ModBlockEntity, pos: Pos) -> Compound {
    let creative = entity.id == CREATIVE_FLUID_TANK;
    let part = read_part(&entity.nbt, pos, creative);
    let mut tag = Compound::new();
    // Always sent: a client keeps the old value of a key that is left out.
    tag.insert(CONTROLLER.into(), Tag::IntArray(part.controller.to_vec()));
    tag.insert(SIZE.into(), Tag::Int(part.width));
    tag.insert(HEIGHT.into(), Tag::Int(part.height));
    tag.insert(WINDOW.into(), Tag::Byte(i8::from(part.window)));
    tag.insert(CONTENT.into(), content_tag(&part.fluid, part.amount));
    if let Some(api) = BOILER.get() {
        (api.update_tag)(entity, &mut tag);
    }
    tag
}

// --- buckets and bottles ------------------------------------------------------------------------------

/// `FluidTankBlock.useItemOn` with `FluidHelper.tryEmptyItemIntoBE` and `tryFillItemFromBE`:
/// a full container empties into the tank, an empty one is filled from it. True when it did.
fn use_container(
    host: &mut dyn Host,
    pos: Pos,
    kind: &str,
    stack: &ItemStack,
    player_creative: bool,
) -> bool {
    let item: &str = &stack.item;
    let Some((controller, part)) = controller_of(host, kind, pos) else {
        return false;
    };
    // A boiler's handler takes water from pipes only (`BoilerData.BoilerFluidHandler`).
    if boiler(&*host, controller).is_some() {
        return false;
    }
    let tank_creative = kind == CREATIVE_FLUID_TANK;
    // A creative player or a creative tank doesn't use the item up.
    let consume = !player_creative && !tank_creative;
    if consume && stack.extra.is_some() {
        // A named or enchanted container isn't one of the plain ones.
        return false;
    }
    let centre = util::centre(pos);

    if let Some(container) = fluid_rules::emptied(item) {
        let mut after = part.clone();
        if after.fill(container.fluid, container.amount) != container.amount {
            return false;
        }
        if tank_creative {
            // `CreativeSmartFluidTank.setContainedFluid` with what the item holds.
            after.set_contained(Some(container.fluid.to_owned()));
        }
        if consume {
            if host.take_items(item, 1) != 1 {
                return false;
            }
            host.give_items(container.empty, 1);
        }
        store_content(host, controller, &after);
        let pitch = fluid_rules::tank_pitch(part.amount_held(), CAPACITY, jitter());
        host.play_sound(fluid_rules::empty_sound(container.fluid), centre, 4, 0.5, pitch);
        host.block_entity_sync(controller);
        return true;
    }

    let Some(fluid) = part.fluid.clone() else {
        return false;
    };
    let Some(container) = fluid_rules::filled(item, &fluid) else {
        return false;
    };
    if part.amount_held() < container.amount {
        return false;
    }
    let mut after = part.clone();
    after.drain(container.amount);
    if tank_creative {
        // `setContainedFluid(EMPTY)`: a creative tank loses its fluid when a container is filled.
        after.set_contained(None);
    }
    if consume && host.take_items(item, 1) != 1 {
        return false;
    }
    store_content(host, controller, &after);
    // A creative tank doesn't use the empty container up; a creative player gets nothing.
    if !player_creative {
        host.give_items(container.full, 1);
    }
    let pitch = fluid_rules::tank_pitch(part.amount_held(), CAPACITY, jitter());
    host.play_sound(fluid_rules::fill_sound(&fluid), centre, 4, 0.5, pitch);
    host.block_entity_sync(controller);
    true
}

/// The player whose click a tank took, when, and on which block. The client doesn't know the tank
/// used the click, so it goes on with the rest of it: the same click again for the off hand
/// (`right_click_block` doesn't say which hand, and the tank reads the main hand, so a filled
/// bucket would be poured straight back or a stack filled twice) and the bucket's own use
/// (`Item.use`), which would pour a source beside the tank. Both are dropped while this is fresh.
/// Create's click is consumed on both sides, so this has no Create counterpart.
static TAKEN_CLICK: std::sync::Mutex<Option<(u128, i64, Pos)>> = std::sync::Mutex::new(None);

/// Whether `uuid`'s click at `now` is the rest of a click a tank already took (within a tick: the
/// packets of one click arrive together). `at` limits it to that block.
fn click_taken(uuid: u128, now: i64, at: Option<Pos>) -> bool {
    let taken = *TAKEN_CLICK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    taken.is_some_and(|(who, when, pos)| {
        who == uuid && now - when <= 1 && at.is_none_or(|at| at == pos)
    })
}

/// `level.random.nextFloat() - .5f`.
fn jitter() -> f32 {
    (util::random() % 1000) as f32 / 1000.0 - 0.5
}

// --- the hooks ------------------------------------------------------------------------------------------

impl Hooks for Tanks {
    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        is_tank(event.block).then(|| (event.block.to_owned(), new_entity()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if is_tank(event.block) {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if is_tank(&event.entity.id) {
            tick(host, event);
        }
    }

    fn block_entity_update_tag(
        &self,
        event: &mut BlockEntityUpdateTagEvent<'_>,
    ) -> Option<Compound> {
        is_tank(&event.entity.id).then(|| update_tag(event.entity, event.pos))
    }

    fn block_entity_render_bounding_box(
        &self,
        _: &dyn Host,
        event: &mut BlockEntityRenderBoundingBoxEvent<'_>,
    ) {
        if !is_tank(event.block) {
            return;
        }
        let part = read_part(&event.entity.nbt, event.pos, event.block == CREATIVE_FLUID_TANK);
        if part.is_controller(event.pos) {
            event.bounds[3] = event.bounds[3].max(part.width as f32);
            event.bounds[4] = event.bounds[4].max(part.height as f32);
            event.bounds[5] = event.bounds[5].max(part.width as f32);
        }
    }

    /// `FluidTankRenderer`: the controller draws the fluid, inside the glass.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if !is_tank(event.block) {
            return;
        }
        let part = read_part(&event.entity.nbt, event.pos, event.block == CREATIVE_FLUID_TANK);
        if !part.is_controller(event.pos) || !part.window {
            return;
        }
        let Some(fluid) = part.fluid.clone() else {
            return;
        };
        let Some(bounds) = rules::fluid_bounds(part.width, part.height, part.fill_state(), false) else {
            return;
        };
        super::fluids::draw_fluid(host, &mut *event.draw, &fluid, part.amount_held(), bounds, false);
        // Nothing here reads the partial tick or the game time.
        event.draw.keep();
    }

    fn custom_block_fluids(&self, _: &dyn Host, event: &mut CustomBlockFluidsEvent<'_>) {
        if is_tank(event.block) {
            event.custom = true;
        }
    }

    fn custom_fluid_tanks(
        &self,
        host: &dyn Host,
        event: &mut CustomFluidTanksEvent,
    ) -> Option<Vec<NativeFluidTankView>> {
        let kind = host.block(event.pos);
        if !is_tank(&kind) {
            return None;
        }
        Some(
            controller_of(host, &kind, event.pos)
                .map(|(controller, part)| match boiler(host, controller) {
                    // `BoilerFluidHandler`: no fluid in it, room for 10 buckets of water.
                    Some(api) => vec![NativeFluidTankView {
                        tank: 0,
                        content: None,
                        capacity: api.capacity,
                        can_fill: true,
                        can_drain: false,
                    }],
                    None => vec![view(&part)],
                })
                .unwrap_or_default(),
        )
    }

    /// `IFluidHandler.fill`: into the controller, whichever block of the tank is touched.
    fn custom_fluid_fill(
        &self,
        host: &mut dyn Host,
        event: &mut CustomFluidFillEvent<'_>,
    ) -> Option<i32> {
        let kind = host.block(event.pos);
        if !is_tank(&kind) {
            return None;
        }
        let resource = event.resource;
        if !resource.components.is_empty() || resource.amount <= 0 {
            return Some(0);
        }
        let Some((controller, part)) = controller_of(&*host, &kind, event.pos) else {
            return Some(0);
        };
        if let Some(api) = boiler(&*host, controller) {
            let execute = event.action == NativeFluidAction::Execute;
            return Some((api.fill)(host, controller, &resource.fluid, resource.amount, execute));
        }
        let mut after = part.clone();
        let accepted = after.fill(&resource.fluid, resource.amount);
        if event.action == NativeFluidAction::Execute && after != part {
            store_content(host, controller, &after);
        }
        Some(accepted)
    }

    /// `IFluidHandler.drain`: from the controller; a creative tank gives and keeps all.
    fn custom_fluid_drain(
        &self,
        host: &mut dyn Host,
        event: &mut CustomFluidDrainEvent<'_>,
    ) -> Option<NativeFluidStack> {
        let kind = host.block(event.pos);
        if !is_tank(&kind) {
            return None;
        }
        let (controller, part) = controller_of(&*host, &kind, event.pos)?;
        if boiler(&*host, controller).is_some() {
            return None;
        }
        let max = match event.request {
            NativeFluidDrain::Any(max) => *max,
            NativeFluidDrain::Exact(stack) => {
                if !stack.components.is_empty() || part.fluid.as_deref() != Some(stack.fluid.as_str()) {
                    return None;
                }
                stack.amount
            }
            _ => return None,
        };
        let mut after = part.clone();
        let (fluid, amount) = after.drain(max)?;
        if event.action == NativeFluidAction::Execute && after != part {
            store_content(host, controller, &after);
        }
        Some(NativeFluidStack {
            fluid,
            amount,
            components: Compound::new(),
        })
    }

    /// `FluidTankBlock.onRemove`: a player breaking a block splits its tank first, so the fluid
    /// is shared out among the blocks left.
    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        let pos = event.pos;
        let kind = host.block(pos);
        if !is_tank(&kind) {
            return Flow::Continue;
        }
        let Some(part) = load_part(&*host, &kind, pos) else {
            return Flow::Continue;
        };
        if part.is_controller(pos) && part.width == 1 && part.height == 1 {
            return Flow::Continue;
        }
        let changes = solve(&*host, &kind, None, |region| {
            rules::split_removed(region, pos, part);
        });
        apply(host, None, changes);
        Flow::Continue
    }

    /// Buckets and bottles empty into or fill from the tank (`FluidTankBlock.useItemOn`).
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        let kind = host.block(pos);
        if !is_tank(&kind) {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        let uuid = player.uuid;
        let Some(stack) = host.player_equipment(uuid, EquipmentSlot::MainHand) else {
            return Flow::Continue;
        };
        // Sneaking with something in hand skips the block's own use.
        if player.sneaking {
            return Flow::Continue;
        }
        let item: &str = &stack.item;
        if !fluid_rules::is_container(item) {
            return Flow::Continue;
        }
        let now = util::game_time(&*host);
        if click_taken(uuid, now, Some(pos)) {
            // The off hand's turn of a click the main hand's turn already used.
            return Flow::Cancel;
        }
        use_container(host, pos, &kind, &stack, player.game_mode == util::CREATIVE);
        *TAKEN_CLICK.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some((uuid, now, pos));
        // `GenericItemEmptying.canItemBeEmptied || canItemBeFilled`: the click is the tank's.
        Flow::Cancel
    }

    /// The item use the client sends after a click a tank took (see `TAKEN_CLICK`).
    fn use_item(&self, host: &mut dyn Host, event: &mut UseItemEvent<'_>) -> Flow {
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if !click_taken(player.uuid, util::game_time(&*host), None) {
            return Flow::Continue;
        }
        let item: &str = &event.stack.item;
        if fluid_rules::is_container(item) || fluid_rules::emptied(item).is_some() {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }

    /// The Wrench: sneak-click picks the tank up; a click toggles the windows of the whole tank
    /// (`FluidTankBlock.onWrenched`).
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH {
            return Flow::Continue;
        }
        let pos = event.pos;
        let kind = host.block(pos);
        if !is_tank(&kind) {
            return Flow::Continue;
        }
        util::wrench(host, pos, |host| {
            let Some((controller, part)) = controller_of(&*host, &kind, pos) else {
                return;
            };
            // `FluidTankBlockEntity.toggleWindows`: not on a boiler.
            if boiler(&*host, controller).is_some() {
                return;
            }
            let window = !part.window;
            let changes = solve(&*host, &kind, None, |region| {
                rules::set_windows(region, controller, window);
            });
            apply(host, None, changes);
        })
    }
}
