//! A Brass Tunnel giving out the stack a belt handed it (`BrassTunnelBlockEntity.tick`,
//! `insertIntoTunnel`, `gatherValidOutputs`, `grabAllStacksOfGroup`): the glue that reads the
//! world for `create_kinetics::rules::tunnel_brass` and applies what it decides.

use create_kinetics::belt::{Item, movement_speed};
use create_kinetics::rules::tunnel_brass::{
    self as rules, Gathered, Group, Member, Neighbour, Output, Sink, TIMER, Tick,
};
use create_kinetics::{Dir, Pos};
use horizon_client::mod_api::{Compound, Hooks, Host, ItemStack, ModBlockEntity, Tag};

use super::tunnel_world::{self as world, BRASS};
use crate::{belt, int, util};

struct Shared;

impl Hooks for Shared {}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

/// `ItemEntity.setDefaultPickUpDelay`.
const PICKUP_DELAY: i16 = 10;

/// A tunnel in the middle of its tick, and the world around it.
struct Brass<'a> {
    host: &'a mut dyn Host,
    pos: Pos,
    /// The tunnel's own saved data: the block entity is out of the world while it ticks.
    own: Compound,
    stack: Option<ItemStack>,
    /// Whether it holds a stack as the tick began.
    holding: bool,
    /// The tunnels of the group the last search found.
    members: Vec<Pos>,
    /// Flaps to swing, applied once the tick is over (the tunnel can't change itself meanwhile).
    flaps: Vec<(Pos, Dir, bool)>,
}

impl Brass<'_> {
    /// The saved data of the tunnel at `at`.
    fn nbt_at(&self, at: Pos) -> Option<Compound> {
        if at == self.pos {
            return Some(self.own.clone());
        }
        world::entity_at(&*self.host, at).map(|e| e.nbt.clone())
    }

    /// `BrassTunnelBlockEntity.insertIntoTunnel`: offers the stack to the side of the tunnel at
    /// `tunnel`. None when it refuses; else what was left.
    fn insert_into_tunnel(
        &mut self,
        tunnel: Pos,
        side: Dir,
        stack: ItemStack,
        simulate: bool,
    ) -> Option<ItemStack> {
        if stack.count == 0 {
            return Some(stack);
        }
        if !world::filter_allows(&self.nbt_at(tunnel)?, side, &stack) {
            return None;
        }
        let under = world::below(tunnel);
        let below = belt::below(&*self.host, under)?;
        let offset = world::add(under, side);
        match belt::can_insert_from_side(&*self.host, offset, side) {
            Some(false) => None,
            Some(true) => {
                let angle = (util::random() % 360) as i32;
                let item = Item::dropped(stack, 0, angle, true);
                let rest = belt::handle_insertion(self.host, offset, side, &item, false, simulate)?;
                if rest.count == 0 && !simulate {
                    self.flaps.push((tunnel, side, false));
                }
                Some(rest)
            }
            None => {
                // No input there: items fly off the end of the belt where nothing solid is.
                if side != below.movement || belt::face_full(&*self.host, offset, side.opposite()) {
                    return None;
                }
                if !simulate {
                    self.flaps.push((tunnel, side, true));
                    let speed = f64::from(movement_speed(below.rpm).abs().max(1.0 / 8.0));
                    let n = side.normal().map(f64::from);
                    let motion = [n[0] * speed, 1.0 / 8.0, n[2] * speed];
                    let c = util::centre(under);
                    let at = [c[0] + n[0] * 0.5, c[1] + 6.0 / 16.0, c[2] + n[2] * 0.5];
                    util::spawn_item(self.host, at, &stack, motion, PICKUP_DELAY);
                }
                let mut gone = stack;
                gone.count = 0;
                Some(gone)
            }
        }
    }

    /// What a side of a tunnel faces, as `addValidOutputsOf` asks.
    fn look(&self, tunnel: Pos, side: Dir) -> Neighbour {
        let offset = world::add(world::below(tunnel), side);
        let funnel = world::above(offset);
        let name = self.host.block(funnel);
        if matches!(name.as_str(), "create:andesite_belt_funnel" | "create:brass_belt_funnel")
            && self.host.block_property(funnel, "shape").as_deref() == Some("pulling")
            && util::facing(&*self.host, funnel, "facing") == Some(side)
        {
            return Neighbour::PullingFunnel;
        }
        match belt::can_insert_from_side(&*self.host, offset, side) {
            Some(accepts) => Neighbour::Input(accepts),
            None => Neighbour::NoInput { open: !belt::face_full(&*self.host, offset, side.opposite()) },
        }
    }

    fn loaded(&self, at: Pos) -> bool {
        self.host.chunk_is_loaded(self.host.dimension(), [at[0] >> 4, at[2] >> 4])
    }
}

impl Sink for Brass<'_> {
    fn insert(&mut self, output: &Output, count: u32, simulate: bool) -> Option<u32> {
        let mut stack = self.stack.clone()?;
        stack.count = count;
        self.insert_into_tunnel(output.tunnel, output.side, stack, simulate)
            .map(|rest| rest.count)
    }
}

impl Group for Brass<'_> {
    fn belt_moving(&self) -> bool {
        belt::below(&*self.host, world::below(self.pos)).is_some_and(|b| b.rpm != 0.0)
    }

    fn gather(&mut self) -> Gathered {
        let Some(view) = world::view(&*self.host, self.pos) else {
            return Gathered::default();
        };
        let mut gathered = Gathered { complete: true, ..Gathered::default() };
        let members = world::group(&*self.host, self.pos, view.axis);
        for &at in &members {
            if !self.loaded(at) {
                gathered.complete = false;
                break;
            }
            let nbt = self.nbt_at(at);
            let holding = if at == self.pos {
                self.holding
            } else {
                nbt.as_ref().is_some_and(|n| util::has(n, world::STACK))
            };
            gathered.holding.push(holding);
            let (Some(nbt), Some(below)) = (nbt, belt::below(&*self.host, world::below(at))) else {
                continue;
            };
            let member = Member {
                movement: below.movement,
                stopped: below.rpm == 0.0,
                sides: world::dirs_of(&nbt, world::SIDES),
            };
            for side in rules::valid_outputs(&member, at == self.pos, &mut |d| self.look(at, d)) {
                gathered.outputs.push(Output { tunnel: at, side });
            }
        }
        self.members = members;
        gathered
    }

    fn filter_empty(&self, output: &Output) -> bool {
        self.nbt_at(output.tunnel)
            .is_none_or(|nbt| world::filter_item(&nbt, output.side).is_none())
    }

    fn exists(&self, output: &Output) -> bool {
        output.tunnel == self.pos
            || self
                .host
                .mod_block_entity(output.tunnel)
                .is_some_and(|e| e.id == BRASS)
    }
}

/// A Brass Tunnel's server tick: what it holds counts down and goes out (`BrassTunnelBlockEntity.tick`).
pub fn tick(host: &mut dyn Host, pos: Pos, entity: &mut ModBlockEntity) {
    let (mut state, stack) = world::read_state(pos, &entity.nbt);
    // Nothing held and nothing waiting on the others: nothing to do (and no write).
    if state.count == 0 && !state.synced_output_active {
        return;
    }
    let before = state.clone();
    let mode = world::mode_of(&entity.nbt);
    let mut group = Brass {
        host: &mut *host,
        pos,
        own: entity.nbt.clone(),
        stack: stack.clone(),
        holding: state.count > 0,
        members: vec![pos],
        flaps: Vec::new(),
    };
    let done: Tick = rules::tick(&mut state, mode, TIMER, &mut group, |n| {
        (util::random() % n.max(1) as u64) as usize
    });
    let Brass { members, flaps, .. } = group;

    if state != before {
        world::write_state(&mut entity.nbt, &state, stack.as_ref());
    }
    let now = util::game_time(host);
    let mut swung = false;
    for (tunnel, side, inward) in flaps {
        swung = true;
        if tunnel == pos {
            world::set_swing(&mut entity.nbt, side, inward, now);
        } else {
            world::flap(host, tunnel, side, inward);
        }
    }
    if let Some(active) = done.sync_group {
        for other in members.into_iter().filter(|m| *m != pos) {
            host.change_mod_block_entity(other, &mut |e| {
                if util::boolean(&e.nbt, world::SYNCED) != active {
                    e.nbt.insert(world::SYNCED.into(), Tag::Byte(i8::from(active)));
                }
            });
        }
    }
    if swung {
        host.block_entity_sync(pos);
    }
}

/// `BrassTunnelBlockEntity.setStackToDistribute` for the tunnel at `pos`, from the belt's side
/// `entered_from`: a stack came in.
pub fn give_stack(host: &mut dyn Host, pos: Pos, stack: &ItemStack, entered_from: Option<Dir>) {
    host.change_mod_block_entity(pos, &mut |e| {
        let (mut state, _) = world::read_state(pos, &e.nbt);
        state.set_stack(stack.count, entered_from);
        world::write_state(&mut e.nbt, &state, Some(stack));
    });
}

/// `grabAllStacksOfGroup`: the stacks every tunnel of the group at `pos` holds, the tunnel's own
/// first. Taken out of the tunnels unless `simulate`.
pub fn grab_group(host: &mut dyn Host, pos: Pos, simulate: bool) -> Vec<ItemStack> {
    let Some(view) = world::view(&*host, pos) else {
        return Vec::new();
    };
    let mut stacks = Vec::new();
    for at in world::group(&*host, pos, view.axis) {
        let Some(entity) = host.mod_block_entity(at) else {
            continue;
        };
        let (_, stack) = world::read_state(at, &entity.nbt);
        let Some(stack) = stack else {
            continue;
        };
        stacks.push(stack);
        if !simulate {
            host.change_mod_block_entity(at, &mut |e| {
                let (mut state, _) = world::read_state(at, &e.nbt);
                state.set_stack(0, None);
                world::write_state(&mut e.nbt, &state, None);
            });
        }
    }
    stacks
}

/// The number the mode is saved as, for the group to follow (`ScrollValueBehaviour.setValue`).
pub fn set_mode_group(host: &mut dyn Host, pos: Pos, mode: rules::Mode) {
    let Some(view) = world::view(&*host, pos) else {
        return;
    };
    for at in world::group(&*host, pos, view.axis) {
        host.change_mod_block_entity(at, &mut |e| {
            if int(&e.nbt, world::MODE) != mode.ordinal() {
                e.nbt.insert(world::MODE.into(), Tag::Int(mode.ordinal()));
            }
        });
    }
}
