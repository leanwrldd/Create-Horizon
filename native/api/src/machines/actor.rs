//! Actors on moving structures: the engine's one hook for blocks that act while the structure they
//! are part of moves (`MovementBehaviour`, `MovementContext`, `AbstractContraptionEntity.tickActors`,
//! `Contraption.startMoving` / `stop` / `setActorsActive`). The behaviours themselves are in
//! `actor_tools.rs` (Drill, Saw, Harvester, Plough, Roller, Deployer, Funnel) and `actor_seat.rs`;
//! the rules, with no game types, are `create_kinetics::rules::actor` and its parts.
//!
//! # The hook
//!
//! A controller (`piston.rs`, `bearing.rs`, `gantry.rs`, `pulley.rs`) calls [`tick`] each tick its
//! structure runs, with the [`Pose`] it already has for the Portable Interfaces (where the
//! structure is at any game time), and [`stop`] just before it sets the structure down. `tick`
//! works out each actor block's world position (`toGlobalVector(centre + getActiveAreaOffset)`),
//! the moment it enters a new block position (`shouldActorTrigger`) and calls
//! `visitNewPosition`, `onSpeedChanged` and `tick` on its behaviour as `tickActors` does. It answers
//! whether any actor stalls the structure (`context.stall`); the controller holds the structure
//! still the way it does for an interface (`psi_motion::set_stalled`).
//!
//! The contexts (`MovementContext`) are kept in memory per controller, not in the structure's NBT:
//! a reload starts every actor afresh (`startMoving`): a block half broken is begun again, and an
//! actor's first movement counts again. What the actors change in the structure (the items they
//! collect, a Deployer's hand) is written into the structure's saved blocks, so it survives.
//!
//! Drops go into the structure's storage first (`MovementBehaviour.collectOrDropItem`): the chests,
//! barrels, shulker boxes and vaults it carries, through the Portable Interfaces' storage reader
//! (`portable_interface_storage.rs`). What doesn't fit is thrown out at the actor.
//!
//! Known differences from Create: see `README.md` (Actors on moving structures).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use create_kinetics::rules::actor::{self as rules, Carrier, Context, Kind, Vec3};
use create_kinetics::rules::contraption::State;
use create_kinetics::rules::portable_interface::Stack;
use create_kinetics::{Dir, Pos};
use horizon_client::mod_api::{Compound, Hooks, Host, ItemStack, ModBlockEntity, RandomSource, Tag};

use super::actor_tools as tools;
use super::contraption::{self as engine, CONTRAPTION, Live, Moving};
use super::portable_interface_motion::Pose;
use super::portable_interface_storage as storage;
use crate::util;

struct Shared;

impl Hooks for Shared {}

/// A module of shared code: no hooks of its own.
pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

// --- the memory of a running structure -------------------------------------------------------------

/// One actor block of a structure.
pub(super) struct Actor {
    pub kind: Kind,
    /// The block's position in the structure.
    pub local: Pos,
    pub state: State,
    /// The block entity's saved data.
    pub data: Compound,
    pub ctx: Context,
    pub breaker: rules::Breaker,
    /// The crack's id (`BreakerId`).
    pub id: i32,
    pub extra: tools::Extra,
}

struct Run {
    actors: Vec<Actor>,
    blocks: usize,
    seen: i64,
}

type Key = (String, Pos);

static RUNS: Mutex<Option<HashMap<Key, Run>>> = Mutex::new(None);

fn with_runs<R>(f: impl FnOnce(&mut HashMap<Key, Run>) -> R) -> R {
    let mut guard = RUNS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    f(guard.get_or_insert_with(HashMap::new))
}

/// How long a run nobody ticked stays (a controller that was removed or unloaded).
const STALE: i64 = 400;

fn controls_of(moving: &Moving) -> Vec<(bool, Option<String>)> {
    moving
        .blocks
        .iter()
        .filter(|b| b.state.name == rules::CONTROLS)
        .map(|b| {
            let data = b.data.as_ref();
            let disabled = data.is_some_and(|d| util::boolean(d, "Disabled"));
            let filter = data
                .and_then(|d| util::get(d, "Filter"))
                .and_then(util::stack_of)
                .map(|stack| stack.item.to_string());
            (disabled, filter)
        })
        .collect()
}

impl Run {
    /// `Contraption.startMoving`: a context for each actor, and the actors the Controls start
    /// disabled (`disableActorOnStart`, `setActorsActive(filter, false)`).
    fn start(moving: &Moving, now: i64, controller: Pos) -> Run {
        let controls = controls_of(moving);
        let refs: Vec<(bool, Option<&str>)> = controls.iter().map(|(d, f)| (*d, f.as_deref())).collect();
        let disabled = rules::disabled_filters(&refs);
        let mut actors = Vec::new();
        for block in &moving.blocks {
            let Some(kind) = rules::kind_of(&block.state.name) else { continue };
            let mut ctx = Context::default();
            ctx.disabled = rules::is_disabled(&disabled, &block.state.name);
            let id_at = [
                controller[0].wrapping_mul(7).wrapping_add(block.local[0]),
                controller[1].wrapping_mul(7).wrapping_add(block.local[1]),
                controller[2].wrapping_mul(7).wrapping_add(block.local[2]),
            ];
            actors.push(Actor {
                kind,
                local: block.local,
                state: block.state.clone(),
                data: block.data.clone().unwrap_or_default(),
                ctx,
                breaker: rules::Breaker::default(),
                id: create_kinetics::rules::drill::breaker_id(id_at),
                extra: tools::Extra::default(),
            });
        }
        Run { actors, blocks: moving.blocks.len(), seen: now }
    }
}

// --- the structure's storage ------------------------------------------------------------------------------

/// The structure's inventories, read when an actor first needs them and written back once at the
/// end of the tick if anything moved (`context.contraption.getStorage().getAllItems()`).
pub(super) struct Store {
    live: Arc<Live>,
    moving: Option<Moving>,
    items: Option<storage::Items>,
    dirty: bool,
}

impl Store {
    fn new(live: Arc<Live>) -> Store {
        Store { live, moving: None, items: None, dirty: false }
    }

    fn items(&mut self, host: &dyn Host) -> &mut storage::Items {
        let live = &self.live;
        self.items.get_or_insert_with(|| storage::items_of(host, &live.moving))
    }

    pub fn moving_mut(&mut self) -> &mut Moving {
        self.dirty = true;
        let live = &self.live;
        self.moving.get_or_insert_with(|| live.moving.clone())
    }

    /// `ItemHandlerHelper.insertItemStacked` into the structure's items: what is left of `stack`.
    pub fn insert(&mut self, host: &dyn Host, stack: ItemStack) -> ItemStack {
        let items = self.items(host);
        if items.inventory.slots.is_empty() {
            return stack;
        }
        let rules_stack = items.from_stack(host, &stack);
        let left = items.inventory.insert_stacked(&rules_stack, false);
        let mut rest = stack;
        if left != i32::try_from(rest.count).unwrap_or(i32::MAX) {
            self.dirty = true;
        }
        rest.count = u32::try_from(left).unwrap_or(0);
        rest
    }

    /// `ItemHelper.extract(inventory, test, mode, amount, false)`: up to (or exactly, when `exact`)
    /// `amount` items that pass `test`, taken slot by slot, all of the kind of the first found.
    pub fn extract(
        &mut self,
        host: &dyn Host,
        test: &dyn Fn(&ItemStack) -> bool,
        amount: i32,
        exact: bool,
    ) -> Option<ItemStack> {
        let items = self.items(host);
        let mut wanted: Option<String> = None;
        let mut available = 0;
        for slot in &items.inventory.slots {
            let Some(stack) = &slot.stack else { continue };
            if let Some(key) = &wanted {
                if &stack.key != key {
                    continue;
                }
            } else {
                let Some(game) = items.to_stack(stack) else { continue };
                if !test(&game) {
                    continue;
                }
                wanted = Some(stack.key.clone());
            }
            available += stack.count;
        }
        let key = wanted?;
        if available <= 0 || (exact && available < amount) {
            return None;
        }
        let mut need = amount.min(available);
        let mut taken: Option<Stack> = None;
        for at in 0..items.inventory.slots.len() {
            if need <= 0 {
                break;
            }
            let matches = items.inventory.slots[at].stack.as_ref().is_some_and(|s| s.key == key);
            if !matches {
                continue;
            }
            if let Some(out) = items.inventory.extract(at, need, false) {
                need -= out.count;
                match &mut taken {
                    Some(have) => have.count += out.count,
                    None => taken = Some(out),
                }
            }
        }
        let taken = taken?;
        self.dirty = true;
        self.items.as_ref()?.to_stack(&taken)
    }

    /// Writes what changed into the cached copy the set-down places from, when the saved structure
    /// is about to go (`stop`).
    fn commit_cached(self, controller: Pos, dimension: &str) {
        if !self.dirty {
            return;
        }
        let mut moving = self.moving.unwrap_or_else(|| self.live.moving.clone());
        if let Some(items) = &self.items {
            storage::apply_items(&mut moving, items);
        }
        engine::set_live(dimension, controller, moving);
    }

    /// Writes what changed into the controller's saved structure and the cached copy the set-down
    /// places from.
    fn commit(self, controller: Pos, dimension: &str, entity: &mut ModBlockEntity) {
        if !self.dirty {
            return;
        }
        let mut moving = self.moving.unwrap_or_else(|| self.live.moving.clone());
        if let Some(items) = &self.items {
            storage::apply_items(&mut moving, items);
        }
        entity.nbt.insert(CONTRAPTION.into(), Tag::Compound(moving.to_nbt(false)));
        engine::set_live(dimension, controller, moving);
    }
}

// --- what a behaviour is given ----------------------------------------------------------------------------

/// The world and the structure for one behaviour call.
pub(super) struct Cx<'a> {
    pub host: &'a mut dyn Host,
    pub store: &'a mut Store,
    pub pose: &'a Pose<'a>,
    pub now: i64,
    pub carrier: Carrier,
    #[allow(dead_code)]
    pub controller: Pos,
}

impl Cx<'_> {
    /// `context.rotation.apply(v)`: a vector of the structure turned as the structure is now.
    pub fn rotated(&self, v: Vec3) -> Vec3 {
        self.pose.rotate(self.now as f64, v)
    }

    /// `MovementBehaviour.collectOrDropItem`: into the structure's storage, else thrown out at the
    /// actor with the structure's motion.
    pub fn collect_or_drop(&mut self, ctx: &Context, stack: ItemStack) {
        let rest = self.store.insert(&*self.host, stack);
        if rest.count == 0 {
            return;
        }
        // Actors might void items if their position is undefined.
        let Some(at) = ctx.position else { return };
        let boost = [ctx.motion[0], ctx.motion[1] + 0.5, ctx.motion[2]];
        let k = self.host_random() * 0.3;
        util::spawn_item(self.host, at, &rest, boost.map(|c| c * k), 0);
    }

    fn host_random(&mut self) -> f64 {
        let dimension = self.host.dimension();
        self.host
            .level_random(dimension)
            .map_or(0.5, |random| f64::from(random.next_float()))
    }
}

// --- the tick -------------------------------------------------------------------------------------------------

/// One server tick of every actor on the structure of the controller at `controller`, `now` being
/// the game time. Answers whether the structure is to be held still (an actor stalls it).
pub fn tick(
    host: &mut dyn Host,
    controller: Pos,
    entity: &mut ModBlockEntity,
    pose: &Pose<'_>,
    now: i64,
    carrier: Carrier,
    on_pulley: bool,
) -> bool {
    let dimension = host.dimension();
    let Some(live) = engine::live(dimension, controller, &entity.nbt) else {
        return false;
    };
    let key: Key = (dimension.to_owned(), controller);
    let mut run = with_runs(|runs| runs.remove(&key))
        .filter(|run| run.blocks == live.moving.blocks.len())
        .unwrap_or_else(|| Run::start(&live.moving, now, controller));
    run.seen = now;
    if run.actors.is_empty() {
        with_runs(|runs| runs.insert(key, run));
        return false;
    }
    let mut store = Store::new(live);
    let t = now as f64;
    let mut stalled = false;
    for actor in &mut run.actors {
        let facing = actor.state.dir("facing");
        let offset = rules::active_area_offset(actor.kind, facing, actor.state.flag("extracting"));
        let centre = pose.centre(t, actor.local);
        let turned = pose.rotate(t, offset);
        let position = [centre[0] + turned[0], centre[1] + turned[1], centre[2] + turned[2]];
        let visited = actor.ctx.arrive(position, &|v| pose.unrotate(t, v), false);
        if !rules::is_active(actor.kind, actor.ctx.disabled, facing, actor.ctx.relative_motion, on_pulley)
            && !rules::must_tick_while_disabled(actor.kind)
        {
            continue;
        }
        let mut cx = Cx { host: &mut *host, store: &mut store, pose, now, carrier, controller };
        if let Some(cell) = visited
            && !actor.ctx.stall
        {
            tools::visit(&mut cx, actor, cell);
            actor.ctx.visited();
        }
        tools::tick(&mut cx, actor);
        stalled |= actor.ctx.stall;
    }
    store.commit(controller, dimension, entity);
    with_runs(|runs| {
        runs.insert(key, run);
        if now.rem_euclid(200) == 0 {
            runs.retain(|_, run| now - run.seen < STALE);
        }
    });
    stalled
}

/// `Contraption.stop` (`stopMoving` of every actor) for the structure of the controller at
/// `controller`, called first thing in a controller's `set_down`: cracks are cleared, a Deployer's
/// hand goes back into its block's data. Only the cached structure the set-down places from is
/// changed (`engine::set_live`), so no entity is needed. Forgets the structure's actors.
pub fn stop(host: &mut dyn Host, controller: Pos, nbt: &Compound) {
    let dimension = host.dimension();
    let key: Key = (dimension.to_owned(), controller);
    let Some(mut run) = with_runs(|runs| runs.remove(&key)) else { return };
    let Some(live) = engine::live(dimension, controller, nbt) else { return };
    let mut store = Store::new(live);
    // `stopMoving` reads no pose.
    let origin = |_: f64| [0.0; 3];
    let pose = Pose::Translating { origin: &origin };
    for actor in &mut run.actors {
        let mut cx =
            Cx { host: &mut *host, store: &mut store, pose: &pose, now: 0, carrier: Carrier::Controlled, controller };
        tools::stop(&mut cx, actor);
        actor.ctx.stop();
    }
    store.commit_cached(controller, dimension);
}

/// The nearest block direction of a vector (`Direction.getNearest`), for a facing turned by the
/// structure.
pub(super) fn nearest(v: Vec3) -> Dir {
    create_kinetics::rules::portable_interface::nearest_direction(v)
}
