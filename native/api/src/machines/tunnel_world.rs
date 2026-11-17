//! What the tunnels read of the world and keep in their block entities: the state of the tunnel
//! block, its neighbours, its sides and flaps, the filters, and the group of Brass Tunnels side by
//! side. Shared by `tunnel.rs`, `tunnel_distribute.rs` and `tunnel_cross.rs`; the decisions are
//! `create_kinetics::rules::tunnel` and `tunnel_brass`.
//!
//! Ported from Create 6.0.10 (MIT): `BeltTunnelBlock` (`getTunnelState`, `hasValidOutput`,
//! `canHaveWindow`, `canSurvive`), `BeltTunnelBlockEntity` (`updateTunnelConnections`, `flaps`,
//! `sides`) and `BrassTunnelBlockEntity` (`getAdjacent`, `determineIfConnected`, the sided filter).

use create_kinetics::rules::chute::filter_passes;
use create_kinetics::rules::tunnel::{self as rules, Beside, Occluder, Shape};
use create_kinetics::rules::tunnel_brass::{Mode, Output, State};
use create_kinetics::{Axis, Dir, Pos};
use horizon_client::mod_api::{Compound, Hooks, Host, ItemStack, ModBlockEntity, Tag};

use crate::{belt, int, util};

pub const ANDESITE: &str = "create:andesite_tunnel";
pub const BRASS: &str = "create:brass_tunnel";
const ANDESITE_BELT_FUNNEL: &str = "create:andesite_belt_funnel";
const BRASS_BELT_FUNNEL: &str = "create:brass_belt_funnel";

/// `BeltTunnelBlockEntity.writeFlapsAndSides`: the sides the tunnel opens on and the ones with a
/// flap, as `Direction.get3DDataValue`s.
pub const SIDES: &str = "Sides";
pub const FLAPS: &str = "Flaps";
/// Each flap's last move: `{Side, In, Tick}` (Create sends a `TunnelFlapPacket`).
pub const SWINGS: &str = "Swings";
/// `BrassTunnelBlockEntity.write`.
pub const CONNECTED_LEFT: &str = "ConnectedLeft";
pub const CONNECTED_RIGHT: &str = "ConnectedRight";
pub const STACK: &str = "StackToDistribute";
const ENTERED_FROM: &str = "StackEnteredFrom";
const PROGRESS: &str = "DistributionProgress";
const PREVIOUS_INDEX: &str = "PreviousIndex";
pub const SYNCED: &str = "SyncedOutput";
const TARGETS: &str = "Targets";
const FILTERED_TARGETS: &str = "FilteredTargets";
/// `newItemArrived`, which Create doesn't save but a tick later needs.
const NEW_ITEM: &str = "NewItem";
/// `ScrollValueBehaviour.write`: the `SelectionMode` ordinal.
pub const MODE: &str = "ScrollValue";
/// `SidedFilteringBehaviour.write`: the filter stack of each side, by the side's name.
pub const FILTERS: &str = "Filters";

/// The ticks a tunnel looks at its neighbours in.
pub const STATE_RATE: i64 = 4;

pub fn is_tunnel(block: &str) -> bool {
    block == ANDESITE || block == BRASS
}

fn is_belt_funnel(block: &str) -> bool {
    block == ANDESITE_BELT_FUNNEL || block == BRASS_BELT_FUNNEL
}

pub fn add(pos: Pos, dir: Dir) -> Pos {
    util::add(pos, dir.normal())
}

pub fn above(pos: Pos) -> Pos {
    add(pos, Dir::Up)
}

pub fn below(pos: Pos) -> Pos {
    add(pos, Dir::Down)
}

/// `Direction.getCounterClockWise`.
fn counter_clockwise(dir: Dir) -> Dir {
    dir.clockwise_about(Axis::Y)
        .clockwise_about(Axis::Y)
        .clockwise_about(Axis::Y)
}

/// The state of a tunnel block.
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub brass: bool,
    pub axis: Axis,
    pub shape: Shape,
}

pub fn view(host: &dyn Host, pos: Pos) -> Option<View> {
    let brass = match host.block(pos).as_str() {
        ANDESITE => false,
        BRASS => true,
        _ => return None,
    };
    Some(View {
        brass,
        axis: host
            .block_property(pos, "axis")
            .as_deref()
            .and_then(Axis::from_name)
            .unwrap_or(Axis::X),
        shape: host
            .block_property(pos, "shape")
            .as_deref()
            .and_then(Shape::from_name)
            .unwrap_or(Shape::Straight),
    })
}

// --- lists of sides in the saved data ---------------------------------------------------------

fn dir_id(dir: Dir) -> i32 {
    Dir::ALL.iter().position(|d| *d == dir).unwrap_or(1) as i32
}

pub fn dirs_tag(dirs: &[Dir]) -> Tag {
    Tag::List(dirs.iter().map(|d| Tag::Int(dir_id(*d))).collect())
}

pub fn dirs_of(nbt: &Compound, key: &str) -> Vec<Dir> {
    match util::get(nbt, key) {
        Some(Tag::List(list)) => list
            .iter()
            .filter_map(|tag| match tag {
                Tag::Int(id) => Dir::ALL.get(*id as usize).copied(),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

// --- the flaps' swings ----------------------------------------------------------------------------

/// `BeltTunnelBlockEntity.flap`: the flap on `side` swings in (`inward`) or out, now.
pub fn set_swing(nbt: &mut Compound, side: Dir, inward: bool, now: i64) {
    let mut list: Vec<Tag> = match util::get(nbt, SWINGS) {
        Some(Tag::List(list)) => list.clone(),
        _ => Vec::new(),
    };
    list.retain(|tag| !matches!(tag.get("Side"), Some(Tag::Int(id)) if *id == dir_id(side)));
    let mut swing = Compound::new();
    swing.insert("Side".into(), Tag::Int(dir_id(side)));
    swing.insert("In".into(), Tag::Byte(i8::from(inward)));
    swing.insert("Tick".into(), Tag::Long(now));
    list.push(Tag::Compound(swing));
    nbt.insert(SWINGS.into(), Tag::List(list));
}

/// The last move of the flap on `side`: whether it went in, and when.
pub fn swing_of(nbt: &Compound, side: Dir) -> Option<(bool, i64)> {
    let Some(Tag::List(list)) = util::get(nbt, SWINGS) else {
        return None;
    };
    list.iter().find_map(|tag| {
        let Tag::Compound(swing) = tag else { return None };
        (int(swing, "Side") == dir_id(side)).then(|| (int(swing, "In") != 0, util::long(swing, "Tick")))
    })
}

/// A flap on the tunnel at `pos` (the tunnel over a belt block): clients are sent it.
pub fn flap(host: &mut dyn Host, pos: Pos, side: Dir, inward: bool) {
    if !is_tunnel(&host.block(pos)) {
        return;
    }
    let now = util::game_time(host);
    if host.change_mod_block_entity(pos, &mut |e| set_swing(&mut e.nbt, side, inward, now)) {
        host.block_entity_sync(pos);
    }
}

// --- the filters ----------------------------------------------------------------------------------

/// `SidedFilteringBehaviour.getFilter(side)`: the item set on that side, if any.
pub fn filter_item(nbt: &Compound, side: Dir) -> Option<ItemStack> {
    match util::get(nbt, FILTERS) {
        Some(Tag::Compound(filters)) => util::get(filters, side.name()).and_then(util::stack_of),
        _ => None,
    }
}

/// `SidedFilteringBehaviour.test(side, stack)`: a side with no filter takes anything, and so does
/// a side the tunnel doesn't open on.
pub fn filter_allows(nbt: &Compound, side: Dir, stack: &ItemStack) -> bool {
    let item = filter_item(nbt, side);
    filter_passes(item.as_ref().map(|s| &*s.item), &stack.item)
}

/// [`filter_allows`] with Filter items filtering by what they hold (`filter.rs`).
pub fn filter_allows_in(host: &dyn Host, nbt: &Compound, side: Dir, stack: &ItemStack) -> bool {
    match filter_item(nbt, side) {
        None => true,
        Some(filter) => super::filter::matches(host, &filter, stack),
    }
}

/// Sets or clears the filter on `side` (one item, as `FilteringBehaviour.setFilter` keeps).
pub fn set_filter(nbt: &mut Compound, side: Dir, item: Option<&ItemStack>) {
    let mut filters = match util::get(nbt, FILTERS) {
        Some(Tag::Compound(filters)) => filters.clone(),
        _ => Compound::new(),
    };
    match item {
        Some(stack) => {
            let mut one = stack.clone();
            one.count = 1;
            filters.insert(side.name().into(), util::stack_tag(&one));
        }
        None => {
            util::remove(&mut filters, side.name());
        }
    }
    nbt.insert(FILTERS.into(), Tag::Compound(filters));
}

// --- the distribution state ---------------------------------------------------------------------

fn pos_tag(pos: Pos, side: Dir) -> Tag {
    let mut tag = Compound::new();
    tag.insert("X".into(), Tag::Int(pos[0]));
    tag.insert("Y".into(), Tag::Int(pos[1]));
    tag.insert("Z".into(), Tag::Int(pos[2]));
    tag.insert("Face".into(), Tag::Int(dir_id(side)));
    Tag::Compound(tag)
}

fn targets_of(nbt: &Compound, key: &str) -> Vec<Output> {
    match util::get(nbt, key) {
        Some(Tag::List(list)) => list
            .iter()
            .filter_map(|tag| {
                let Tag::Compound(t) = tag else { return None };
                Some(Output {
                    tunnel: [int(t, "X"), int(t, "Y"), int(t, "Z")],
                    side: Dir::ALL.get(int(t, "Face") as usize).copied()?,
                })
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// `BrassTunnelBlockEntity.read`: the stack it holds and where it is in giving it out.
pub fn read_state(pos: Pos, nbt: &Compound) -> (State, Option<ItemStack>) {
    let stack = util::get(nbt, STACK).and_then(util::stack_of);
    let mut state = State::new(pos);
    state.count = stack.as_ref().map_or(0, |s| s.count);
    state.entered_from = util::has(nbt, ENTERED_FROM)
        .then(|| Dir::ALL.get(int(nbt, ENTERED_FROM) as usize).copied())
        .flatten();
    if util::has(nbt, PROGRESS) {
        state.progress = int(nbt, PROGRESS);
    }
    state.previous_index = int(nbt, PREVIOUS_INDEX).max(0) as usize;
    state.new_item_arrived = util::boolean(nbt, NEW_ITEM);
    state.synced_output_active = util::boolean(nbt, SYNCED);
    state.targets = [targets_of(nbt, FILTERED_TARGETS), targets_of(nbt, TARGETS)];
    (state, stack)
}

/// `BrassTunnelBlockEntity.write`.
pub fn write_state(nbt: &mut Compound, state: &State, stack: Option<&ItemStack>) {
    match stack {
        Some(stack) if state.count > 0 => {
            let mut held = stack.clone();
            held.count = state.count;
            nbt.insert(STACK.into(), util::stack_tag(&held));
        }
        _ => {
            util::remove(nbt, STACK);
        }
    }
    match state.entered_from {
        Some(dir) => {
            nbt.insert(ENTERED_FROM.into(), Tag::Int(dir_id(dir)));
        }
        None => {
            util::remove(nbt, ENTERED_FROM);
        }
    }
    nbt.insert(PROGRESS.into(), Tag::Int(state.progress));
    nbt.insert(PREVIOUS_INDEX.into(), Tag::Int(state.previous_index as i32));
    nbt.insert(NEW_ITEM.into(), Tag::Byte(i8::from(state.new_item_arrived)));
    nbt.insert(SYNCED.into(), Tag::Byte(i8::from(state.synced_output_active)));
    for (key, list) in [(FILTERED_TARGETS, &state.targets[0]), (TARGETS, &state.targets[1])] {
        nbt.insert(
            key.into(),
            Tag::List(list.iter().map(|o| pos_tag(o.tunnel, o.side)).collect()),
        );
    }
}

pub fn mode_of(nbt: &Compound) -> Mode {
    Mode::from_ordinal(int(nbt, MODE))
}

// --- the neighbours ---------------------------------------------------------------------------------

/// What stands at `pos + toward`, for the window and the flap (`canHaveWindow`, `updateTunnelConnections`).
fn beside(host: &dyn Host, pos: Pos, toward: Dir) -> Beside {
    let n = add(pos, toward);
    let block = host.block(n);
    if is_tunnel(&block) {
        return Beside::Tunnel;
    }
    if is_belt_funnel(&block)
        && host.block_property(n, "shape").as_deref() == Some("extended")
        && util::facing(host, n, "facing") == Some(toward.opposite())
    {
        return Beside::Funnel;
    }
    Beside::Other
}

/// `BeltTunnelBlock.hasValidOutput`: the block beside the belt at `belt_pos`, on `side`, takes
/// items from it.
fn has_valid_output(host: &dyn Host, belt_pos: Pos, side: Dir) -> bool {
    let n = add(belt_pos, side);
    if host.block(n) == belt::ID {
        return util::facing(host, n, "facing").is_some_and(|f| f.axis() == side.axis());
    }
    belt::can_insert_from_side(host, n, side) == Some(true)
}

/// `BeltTunnelBlock.canSurvive`: a horizontal belt below, with a casing.
pub fn can_survive(host: &dyn Host, pos: Pos) -> bool {
    let under = below(pos);
    belt::below(host, under).is_some() && host.block_property(under, "casing").as_deref() == Some("true")
}

/// `BeltTunnelBlock.getTunnelState` (with `settle` for the wrench): the axis and shape the tunnel
/// at `pos` should have; none with no belt below.
pub fn wanted(host: &dyn Host, pos: Pos, current: &View) -> Option<(Axis, Shape)> {
    let under = below(pos);
    let belt = belt::below(host, under)?;
    let axis = belt.facing.axis();
    let left = rules::left(axis);
    let on_left = has_valid_output(host, under, left);
    let on_right = has_valid_output(host, under, left.opposite());
    let forward = rules::positive(axis);
    let window = rules::can_have_window(beside(host, pos, forward), beside(host, pos, forward.opposite()));
    Some(rules::settle(current.axis, current.shape, (axis, rules::shape_for(on_left, on_right, window))))
}

/// `BrassTunnelBlockEntity.getAdjacent`: the Brass Tunnel beside this one on the left (the
/// counter-clockwise side of the axis) or the right, over a belt of the same axis.
pub fn adjacent(host: &dyn Host, pos: Pos, axis: Axis, left: bool) -> Option<Pos> {
    let base = rules::positive(axis);
    let dir = if left { counter_clockwise(base) } else { base.clockwise_about(Axis::Y) };
    let at = add(pos, dir);
    if host.block(at) != BRASS {
        return None;
    }
    if host.block_property(at, "axis").as_deref().and_then(Axis::from_name) != Some(axis) {
        return None;
    }
    host.mod_block_entity(at)?;
    Some(at)
}

/// The group a Brass Tunnel belongs to: itself, then the tunnels to its left, then those to its
/// right (`gatherValidOutputs`' order).
pub fn group(host: &dyn Host, pos: Pos, axis: Axis) -> Vec<Pos> {
    let mut members = vec![pos];
    for left in [true, false] {
        let mut at = pos;
        while let Some(next) = adjacent(host, at, axis, left) {
            if members.contains(&next) || members.len() > 128 {
                break;
            }
            members.push(next);
            at = next;
        }
    }
    members
}

/// What `updateTunnelConnections` works out for the tunnel at `pos`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connections {
    pub sides: Vec<Dir>,
    pub flaps: Vec<Dir>,
    pub left: bool,
    pub right: bool,
}

/// `BeltTunnelBlockEntity.updateTunnelConnections` and `BrassTunnelBlockEntity`'s: the sides and
/// flaps for a tunnel of this axis and shape, and whether a Brass Tunnel beside it (with flaps of
/// its own) is connected.
pub fn connections(host: &dyn Host, pos: Pos, brass: bool, axis: Axis, shape: Shape) -> Connections {
    let sides = rules::sides(axis, shape);
    let flaps = rules::flaps(&sides, |d| {
        if beside(host, pos, d) == Beside::Other {
            Occluder::None
        } else {
            Occluder::Covered
        }
    });
    let connected = |left: bool| {
        brass
            && !flaps.is_empty()
            && adjacent(host, pos, axis, left)
                .and_then(|at| host.mod_block_entity(at))
                .is_some_and(|e| !dirs_of(&e.nbt, FLAPS).is_empty())
    };
    Connections { left: connected(true), right: connected(false), sides, flaps }
}

/// `BrassTunnelBlockEntity.hasDistributionBehaviour` for the tunnel with this saved data.
pub fn distributes(nbt: &Compound, axis: Axis) -> bool {
    let connected = util::boolean(nbt, CONNECTED_LEFT) || util::boolean(nbt, CONNECTED_RIGHT);
    rules::distributes(&dirs_of(nbt, FLAPS), connected, axis)
}

/// A tunnel block entity of the world other than the one ticking (a copy).
pub fn entity_at(host: &dyn Host, pos: Pos) -> Option<ModBlockEntity> {
    host.mod_block_entity(pos).filter(|e| is_tunnel(&e.id))
}

struct Shared;

impl Hooks for Shared {}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sides_round_trip_through_nbt() {
        let mut nbt = Compound::new();
        nbt.insert(SIDES.into(), dirs_tag(&[Dir::North, Dir::East]));
        assert_eq!(dirs_of(&nbt, SIDES), vec![Dir::North, Dir::East]);
        assert!(dirs_of(&nbt, FLAPS).is_empty());
    }

    #[test]
    fn a_flap_keeps_its_last_swing_only() {
        let mut nbt = Compound::new();
        set_swing(&mut nbt, Dir::North, true, 10);
        set_swing(&mut nbt, Dir::West, false, 12);
        set_swing(&mut nbt, Dir::North, false, 20);
        assert_eq!(swing_of(&nbt, Dir::North), Some((false, 20)));
        assert_eq!(swing_of(&nbt, Dir::West), Some((false, 12)));
        assert_eq!(swing_of(&nbt, Dir::South), None);
        let Some(Tag::List(list)) = util::get(&nbt, SWINGS) else { panic!() };
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn filters_are_kept_by_side_and_match_by_item() {
        let mut nbt = Compound::new();
        let item = |id: &str| ItemStack {
            item: id.into(),
            count: 5,
            damage: 0,
            extra: Default::default(),
        };
        assert!(filter_allows(&nbt, Dir::West, &item("minecraft:stick")));
        set_filter(&mut nbt, Dir::West, Some(&item("minecraft:coal")));
        assert!(!filter_allows(&nbt, Dir::West, &item("minecraft:stick")));
        assert!(filter_allows(&nbt, Dir::West, &item("minecraft:coal")));
        // Another side has none.
        assert!(filter_allows(&nbt, Dir::East, &item("minecraft:stick")));
        assert_eq!(filter_item(&nbt, Dir::West).unwrap().count, 1);
        set_filter(&mut nbt, Dir::West, None);
        assert!(filter_allows(&nbt, Dir::West, &item("minecraft:stick")));
    }

    #[test]
    fn the_distribution_state_round_trips() {
        let pos = [3, 70, 9];
        let stack = ItemStack { item: "minecraft:coal".into(), count: 7, damage: 0, extra: Default::default() };
        let mut state = State::new(pos);
        state.count = 7;
        state.entered_from = Some(Dir::North);
        state.progress = 4;
        state.previous_index = 2;
        state.new_item_arrived = true;
        state.synced_output_active = true;
        state.targets = [
            vec![Output { tunnel: [3, 70, 10], side: Dir::East }],
            vec![Output { tunnel: pos, side: Dir::South }, Output { tunnel: [4, 70, 9], side: Dir::West }],
        ];
        let mut nbt = Compound::new();
        write_state(&mut nbt, &state, Some(&stack));
        let (read, kept) = read_state(pos, &nbt);
        assert_eq!(read, state);
        assert_eq!(kept.unwrap().count, 7);
        // Empty: no stack key, and nothing entered it.
        state.count = 0;
        state.entered_from = None;
        write_state(&mut nbt, &state, Some(&stack));
        assert!(!util::has(&nbt, STACK));
        assert!(!util::has(&nbt, ENTERED_FROM));
        assert_eq!(read_state(pos, &nbt).0.count, 0);
    }

    #[test]
    fn counter_clockwise_undoes_clockwise() {
        for d in [Dir::North, Dir::East, Dir::South, Dir::West] {
            assert_eq!(counter_clockwise(d.clockwise_about(Axis::Y)), d);
        }
        // For a belt along z the left tunnel is east of the positive end's turn.
        assert_eq!(counter_clockwise(rules::positive(Axis::Z)), Dir::East);
        assert_eq!(counter_clockwise(rules::positive(Axis::X)), Dir::North);
    }
}
