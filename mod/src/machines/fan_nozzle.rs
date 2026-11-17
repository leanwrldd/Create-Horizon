//! The Nozzle in the game (`NozzleBlock`, `NozzleBlockEntity`): attached to the front of an
//! Encased Fan it turns the fan's current into a push or pull on every entity in sight, in all
//! directions. The rules are `create_kinetics::rules::fan_nozzle`.
//!
//! Known differences from Create, for the README:
//! - A nozzle with no fan behind it breaks on its next tick instead of being refused at
//!   placement (`canSurvive` can't read block entities in Horizon's placement hooks).
//! - The Wrench only picks it up (as in Create, it never turns it); sneaking picks it up.
//! - More than 256 pulled entities are removed (players aside) without the explosion.
//! - No particles; players are pushed through the client half like the fan's.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use create_kinetics::rules::{fan, fan_nozzle as nozzle};
use create_kinetics::{Dir, Pos};
use crate::compat::{
    BlockEntityEvent, BlockEntityTickEvent, BlockEntityTicksEvent, BlockStateId, Compound, Flow,
    Hooks, Host, LevelHost, StateForPlacementEvent, UseItemOnEvent,
};

use super::fan::{ID as FAN, body, meets, set_velocity};
use crate::{kinetic, util};

const ID: &str = "create:nozzle";

struct Nozzle;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Nozzle)
}

/// `NozzleBlockEntity.pushingEntities`, kept in memory per nozzle.
#[derive(Default)]
struct Pushing {
    entities: Vec<u32>,
    searched: Option<i64>,
    seen: i64,
}

type Key = (&'static str, Pos);

static NOZZLES: LazyLock<Mutex<HashMap<Key, Pushing>>> = LazyLock::new(Mutex::default);

/// Never hold the lock across a call into the game.
fn nozzles() -> std::sync::MutexGuard<'static, HashMap<Key, Pushing>> {
    NOZZLES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Hooks for Nozzle {
    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == ID).then(|| (ID.to_owned(), Compound::new()))
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
        tick(host, event.pos);
    }

    /// `NozzleBlock.getStateForPlacement`: it faces out of the face clicked.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if &*context.item != ID {
            return None;
        }
        level
            .state_with_property(event.state, "facing", util::dir(context.face).name())
            .map(Some)
    }

    /// `NozzleBlock.onWrenched` is a failure: only the sneaking Wrench does anything.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || host.block(event.pos) != ID {
            return Flow::Continue;
        }
        util::wrench(host, event.pos, |_| {})
    }
}

/// One server tick of the nozzle at `pos` (`NozzleBlockEntity.tick` and `lazyTick`).
fn tick(host: &mut dyn Host, pos: Pos) {
    let dimension = host.dimension();
    let key = (dimension, pos);
    let Some(facing) = util::facing(host, pos, "facing") else {
        return;
    };
    // `NozzleBlock.canSurvive`: a fan behind it whose air leaves by the nozzle's side.
    let fan_pos = offset(pos, facing.opposite());
    let on_fan = host.block(fan_pos) == FAN && util::facing(host, fan_pos, "facing") == Some(facing);
    if !on_fan {
        nozzles().remove(&key);
        host.destroy_block(pos, None, false);
        return;
    }

    // `calcRange`: the fan's longest reach for its speed, when it turns (the Host can't show the
    // limited reach another block may cut it to).
    let speed = host
        .mod_block_entity(fan_pos)
        .map_or(0.0, |entity| kinetic::speed(&entity));
    let range = if speed == 0.0 { nozzle::NO_RANGE } else { fan::max_distance(speed) };
    let pushing = fan::flow_direction(speed, facing) == Some(facing);

    let mut state = nozzles().remove(&key).unwrap_or_default();
    if range == 0.0 {
        return; // `setRange(0)` clears the list.
    }
    let now = util::game_time(host);
    let centre = util::centre(pos);

    // `lazyTick`, every 5 ticks.
    if state
        .searched
        .is_none_or(|at| now - at >= i64::from(nozzle::LAZY_TICKS) || now < at)
    {
        state.searched = Some(now);
        look_for_entities(host, dimension, pos, centre, range, &mut state);
        if !pushing && state.entities.len() > nozzle::MAX_PULLED {
            // Create explodes (without damaging blocks) and discards them all.
            for id in std::mem::take(&mut state.entities) {
                if host.entity(id).is_some_and(|e| host.entity_kind(e).as_deref() != Some("minecraft:player")) {
                    host.remove_entity(dimension, id);
                }
            }
        }
    }

    // `tick`.
    let mut keep = Vec::with_capacity(state.entities.len());
    for id in std::mem::take(&mut state.entities) {
        let Some(body) = body(host, id) else {
            continue;
        };
        let diff = [
            body.position[0] - centre[0],
            body.position[1] - centre[1],
            body.position[2] - centre[2],
        ];
        let distance = (diff[0] * diff[0] + diff[1] * diff[1] + diff[2] * diff[2]).sqrt();
        if nozzle::lets_go(distance, range, body.sneaking, body.creative_flying()) {
            continue;
        }
        keep.push(id);
        let item = body.kind == "minecraft:item";
        if let Some(push) = nozzle::push(diff, range, pushing, item) {
            let velocity = [
                body.velocity[0] + push[0],
                body.velocity[1] + push[1],
                body.velocity[2] + push[2],
            ];
            set_velocity(host, &body, velocity);
        }
    }
    state.entities = keep;
    state.seen = now;
    let mut map = nozzles();
    if now % 200 == 0 {
        map.retain(|_, n| now - n.seen < 100);
    }
    map.insert(key, state);
}

fn offset(pos: Pos, direction: Dir) -> Pos {
    let n = direction.normal();
    [pos[0] + n[0], pos[1] + n[1], pos[2] + n[2]]
}

/// `NozzleBlockEntity.lazyTick`: the entities within `range` that the nozzle can see.
fn look_for_entities(
    host: &mut dyn Host,
    dimension: &str,
    pos: Pos,
    centre: [f64; 3],
    range: f32,
    state: &mut Pushing,
) {
    let half = nozzle::search_half_extent(range);
    let min = centre.map(|v| v - half);
    let max = centre.map(|v| v + half);
    let mut found = Vec::new();
    // Items first, then what else is in the box (`entities_in` goes by feet).
    for item in host.item_entities(dimension, min, max) {
        found.push(item.id);
    }
    for id in host.entities_in(dimension, min.map(|v| v - 0.5), max.map(|v| v + 0.5)) {
        if !found.contains(&id) {
            found.push(id);
        }
    }
    let mut seen = Vec::new();
    for id in found {
        let Some(body) = body(host, id) else {
            continue;
        };
        if let Some(bounds) = body.bounds {
            if !meets(bounds, min, max) {
                continue;
            }
        }
        let diff = [
            body.position[0] - centre[0],
            body.position[1] - centre[1],
            body.position[2] - centre[2],
        ];
        let distance = (diff[0] * diff[0] + diff[1] * diff[1] + diff[2] * diff[2]).sqrt();
        if nozzle::lets_go(distance, range, body.sneaking, body.creative_flying()) {
            continue;
        }
        if can_see(host, pos, centre, body.position) {
            seen.push(id);
        }
    }
    // Those it can no longer see drop out; new ones join.
    state.entities = seen;
}

/// `NozzleBlockEntity.canSee`: a ray from the entity to the nozzle's centre reaches the nozzle.
fn can_see(host: &dyn Host, pos: Pos, centre: [f64; 3], from: [f64; 3]) -> bool {
    let dir = [centre[0] - from[0], centre[1] - from[1], centre[2] - from[2]];
    let length = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    host.pick_block(from, dir, length + 1.0, false)
        .is_some_and(|hit| [hit.block_x, hit.block_y, hit.block_z] == pos)
}
