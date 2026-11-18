//! The Item Vault's multi-block rules: which vault blocks join into one wider, longer vault, the
//! slots and the comparator reading of the whole, and the plan for keeping every block of a
//! vault useful to a hopper. No game types: the glue hands the rules the vault blocks around a
//! position and writes back what changed.
//!
//! Ported from Create `content/logistics/vault/ItemVaultBlock`, `ItemVaultBlockEntity`,
//! `api/connectivity/ConnectivityHandler` (`formMulti`, `tryToFormNewMulti`,
//! `tryToFormNewMultiOfWidth`, `splitMultiAndInvalidate`: the horizontal case),
//! `foundation/item/ItemHelper.calcRedstoneFromInventory` and `infrastructure/config/CLogistics`
//! (`vaultCapacity`).
//!
//! Known differences from Create:
//! - `tryReconnect` of `splitMultiAndInvalidate` is never used by Create and is left out.
//! - `ItemVaultItem.tryMultiPlace` (placing a whole layer of a wide vault at once) is not done.
//! - [`serve`] is not Create's: see it.

use std::collections::{BTreeMap, BTreeSet};

pub type Pos = [i32; 3];

/// `CLogistics.vaultCapacity`: the stacks one vault block holds.
pub const SLOTS: usize = 20;
/// `ItemVaultBlockEntity.getMaxWidth`: the widest a vault grows.
pub const MAX_WIDTH: i32 = 3;

/// `ItemVaultBlockEntity.getMaxLength(radius)`: the longest a vault of this width grows.
pub const fn max_length(width: i32) -> i32 {
    width * 3
}

/// `BlockStateProperties.HORIZONTAL_AXIS`: the axis a vault is long along.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Axis {
    X,
    Z,
}

impl Axis {
    pub const fn name(self) -> &'static str {
        match self {
            Axis::X => "x",
            Axis::Z => "z",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "x" => Some(Axis::X),
            "z" => Some(Axis::Z),
            _ => None,
        }
    }

    /// `ItemVaultBlock.rotate` once, and the Wrench on a top or a bottom face: the other axis.
    pub const fn turned(self) -> Self {
        match self {
            Axis::X => Axis::Z,
            Axis::Z => Axis::X,
        }
    }
}

/// One vault block as the multi-block rules see it (`ItemVaultBlockEntity`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    /// The controller's position: the block's own when it is the controller.
    pub controller: Pos,
    /// `radius`: how many blocks wide and tall the vault is.
    pub width: i32,
    /// `length`: how many blocks long.
    pub length: i32,
    /// `getMainConnectionAxis`: the block's `axis`.
    pub axis: Axis,
    /// `updateConnectivity`: the block still has to look for neighbours to join.
    pub update_connectivity: bool,
}

impl Part {
    /// A freshly placed block: its own controller, one block (`ItemVaultBlockEntity` constructor).
    pub fn new(pos: Pos, axis: Axis) -> Self {
        Self {
            controller: pos,
            width: 1,
            length: 1,
            axis,
            update_connectivity: true,
        }
    }

    pub fn is_controller(&self, pos: Pos) -> bool {
        self.controller == pos
    }

    /// `ItemVaultBlockEntity.notifyMultiUpdated`: the block's `large` state.
    pub fn large(&self) -> bool {
        self.width > 2
    }

    /// How many blocks the vault this controller leads has.
    pub fn blocks(&self) -> i32 {
        self.width * self.width * self.length
    }
}

/// The block at `y` along the vault's axis and `x`, `z` across it, from the controller at
/// `origin` (the offsets of `tryToFormNewMultiOfWidth` and `initCapability`).
pub fn at(origin: Pos, axis: Axis, y: i32, x: i32, z: i32) -> Pos {
    match axis {
        Axis::X => [origin[0] + y, origin[1] + x, origin[2] + z],
        Axis::Z => [origin[0] + x, origin[1] + z, origin[2] + y],
    }
}

/// Every block of the vault led by `origin`, in the order `initCapability` lays out their slots
/// (`yOffset * radius * radius + xOffset * radius + zOffset`).
pub fn members(origin: Pos, axis: Axis, width: i32, length: i32) -> Vec<Pos> {
    let mut out = Vec::new();
    for y in 0..length.max(1) {
        for x in 0..width.max(1) {
            for z in 0..width.max(1) {
                out.push(at(origin, axis, y, x, z));
            }
        }
    }
    out
}

/// The vault blocks around a position, loaded on demand and kept with their changes.
pub struct Region<L: FnMut(Pos) -> Option<Part>> {
    load: L,
    parts: BTreeMap<Pos, Option<Part>>,
    changed: BTreeSet<Pos>,
    /// A block going away right now: the game has not removed it yet, but `partAt` must not
    /// find it (`BlockEntity.isRemoved`).
    removed: Option<(Pos, Part)>,
}

impl<L: FnMut(Pos) -> Option<Part>> Region<L> {
    /// `load` answers the vault block at a position.
    pub fn new(load: L) -> Self {
        Self {
            load,
            parts: BTreeMap::new(),
            changed: BTreeSet::new(),
            removed: None,
        }
    }

    /// `partAt`: the block at `pos`, if it is a vault and not being removed.
    pub fn get(&mut self, pos: Pos) -> Option<Part> {
        if self.removed.as_ref().is_some_and(|(removed, _)| *removed == pos) {
            return None;
        }
        if !self.parts.contains_key(&pos) {
            let part = (self.load)(pos);
            self.parts.insert(pos, part);
        }
        self.parts[&pos].clone()
    }

    fn modify(&mut self, pos: Pos, change: impl FnOnce(&mut Part)) {
        if self.get(pos).is_none() {
            return;
        }
        if let Some(Some(part)) = self.parts.get_mut(&pos) {
            change(part);
            self.changed.insert(pos);
        }
    }

    /// The blocks whose state changed, with their new state.
    pub fn changes(&self) -> Vec<(Pos, Part)> {
        self.changed
            .iter()
            .filter_map(|pos| Some((*pos, self.parts.get(pos)?.clone()?)))
            .collect()
    }
}

/// `SearchCache`: for each position searched, the controller found there.
type Cache = BTreeMap<Pos, Option<Pos>>;

fn get_or_cache<L: FnMut(Pos) -> Option<Part>>(
    region: &mut Region<L>,
    cache: &mut Cache,
    pos: Pos,
) -> Option<Pos> {
    if let Some(known) = cache.get(&pos) {
        return *known;
    }
    let found = region
        .get(pos)
        .and_then(|part| region.get(part.controller).map(|_| part.controller));
    cache.insert(pos, found);
    found
}

/// `ConnectivityHandler.formMulti`: the block at `start` (a controller) looks for neighbours to
/// join, and the vaults that come out of it are the best way to group the blocks found.
pub fn form_multi<L: FnMut(Pos) -> Option<Part>>(region: &mut Region<L>, start: Pos) {
    let mut cache = Cache::new();
    let mut frontier = vec![start];
    let mut creation: Vec<(i32, Pos)> = Vec::new();
    let mut visited: BTreeSet<Pos> = BTreeSet::new();

    // A horizontal multi's search is bounded below on y by the widest vault, and not at all on
    // x and z (`minY = start.y - getMaxWidth()`).
    let min_y = start[1] - MAX_WIDTH;

    while !frontier.is_empty() {
        let part = frontier.remove(0);
        if !visited.insert(part) {
            continue;
        }
        let amount = try_to_form(region, &mut cache, part, true);
        if amount > 1 {
            creation.push((amount, part));
        }
        // `Iterate.axes`: x, y, z, one step towards the negative side each.
        for axis in 0..3 {
            let mut next = part;
            next[axis] -= 1;
            if next[1] <= min_y || visited.contains(&next) {
                continue;
            }
            if region.get(next).is_some() {
                frontier.push(next);
            }
        }
    }

    // The biggest first, the earlier found first among equals.
    creation.sort_by(|a, b| b.0.cmp(&a.0));
    visited.clear();
    for (_, pos) in creation {
        if !visited.insert(pos) {
            continue;
        }
        try_to_form(region, &mut cache, pos, false);
    }
}

fn try_to_form<L: FnMut(Pos) -> Option<Part>>(
    region: &mut Region<L>,
    cache: &mut Cache,
    be: Pos,
    simulate: bool,
) -> i32 {
    let mut best_width = 1;
    let mut best_amount = -1;
    match region.get(be) {
        Some(part) if part.is_controller(be) => {}
        _ => return 0,
    }
    for width in 1..=MAX_WIDTH {
        let amount = of_width(region, cache, be, width, true);
        if amount < best_amount {
            continue;
        }
        best_width = width;
        best_amount = amount;
    }
    if !simulate {
        let part = region.get(be).expect("controller");
        if part.width == best_width && part.width * part.width * part.length == best_amount {
            return best_amount;
        }
        split_with(region, cache, be);
        of_width(region, cache, be, best_width, false);
        region.modify(be, |p| {
            p.update_connectivity = false;
            p.width = best_width;
            p.length = best_amount / best_width / best_width;
        });
    }
    best_amount
}

fn of_width<L: FnMut(Pos) -> Option<Part>>(
    region: &mut Region<L>,
    cache: &mut Cache,
    origin: Pos,
    width: i32,
    simulate: bool,
) -> i32 {
    let mut amount = 0;
    let mut length = 0;
    let Some(be) = region.get(origin) else {
        return 0;
    };
    let axis = be.axis;

    'search: for y in 0..max_length(width) {
        for x in 0..width {
            for z in 0..width {
                let pos = at(origin, axis, y, x, z);
                let Some(controller_pos) = get_or_cache(region, cache, pos) else {
                    break 'search;
                };
                let Some(controller) = region.get(controller_pos) else {
                    break 'search;
                };
                let other_width = controller.width;
                if other_width > width {
                    break 'search;
                }
                if other_width == width && controller.length == max_length(width) {
                    break 'search;
                }
                if controller.axis != axis {
                    break 'search;
                }
                if controller_pos != origin {
                    // Horizontal multi, like an Item Vault.
                    if axis == Axis::Z && controller_pos[0] < origin[0] {
                        break 'search;
                    }
                    if controller_pos[1] < origin[1] {
                        break 'search;
                    }
                    if axis == Axis::X && controller_pos[2] < origin[2] {
                        break 'search;
                    }
                    if axis == Axis::Z && controller_pos[0] + other_width > origin[0] + width {
                        break 'search;
                    }
                    if controller_pos[1] + other_width > origin[1] + width {
                        break 'search;
                    }
                    if axis == Axis::X && controller_pos[2] + other_width > origin[2] + width {
                        break 'search;
                    }
                }
            }
        }
        amount += width * width;
        length += 1;
    }

    if simulate {
        return amount;
    }

    for y in 0..length {
        for x in 0..width {
            for z in 0..width {
                let pos = at(origin, axis, y, x, z);
                if region.get(pos).is_none() || pos == origin {
                    continue;
                }
                split_with(region, cache, pos);
                region.modify(pos, |p| {
                    p.controller = origin;
                    p.update_connectivity = false;
                    p.length = length;
                    p.width = width;
                });
                cache.insert(pos, Some(origin));
            }
        }
    }
    amount
}

/// `ConnectivityHandler.splitMulti`: the vault block at `pos` is going away (or changing), so
/// the whole vault it is part of comes apart into single blocks. `part` is the block as it is
/// now.
pub fn split_removed<L: FnMut(Pos) -> Option<Part>>(
    region: &mut Region<L>,
    pos: Pos,
    part: Part,
) {
    region.removed = Some((pos, part));
    let mut cache = Cache::new();
    split_with(region, &mut cache, pos);
}

/// `ConnectivityHandler.splitMulti` of a block that stays (the Wrench on a vault's top):
/// the whole vault it is part of comes apart into single blocks, itself included.
pub fn split<L: FnMut(Pos) -> Option<Part>>(region: &mut Region<L>, pos: Pos) {
    let mut cache = Cache::new();
    split_with(region, &mut cache, pos);
}

/// `ItemVaultBlockEntity.removeController(true)` for one block that stays: alone again, looking
/// for neighbours at its next tick.
pub fn alone<L: FnMut(Pos) -> Option<Part>>(region: &mut Region<L>, pos: Pos) {
    region.modify(pos, |p| {
        p.controller = pos;
        p.width = 1;
        p.length = 1;
        p.update_connectivity = true;
    });
}

fn split_with<L: FnMut(Pos) -> Option<Part>>(region: &mut Region<L>, cache: &mut Cache, pos: Pos) {
    // `be = be.getControllerBE()`: the controller of the vault the block belongs to.
    let removed = region.removed.clone();
    let (origin, be) = match removed {
        Some((removed_pos, removed_part)) if removed_pos == pos => {
            if removed_part.controller == removed_pos {
                (removed_pos, removed_part)
            } else {
                let Some(controller) = region.get(removed_part.controller) else {
                    return;
                };
                (removed_part.controller, controller)
            }
        }
        _ => {
            let Some(part) = region.get(pos) else { return };
            let Some(controller) = region.get(part.controller) else {
                return;
            };
            (part.controller, controller)
        }
    };
    if be.width == 1 && be.length == 1 {
        return;
    }
    for y in 0..be.length {
        for x in 0..be.width {
            for z in 0..be.width {
                let there = at(origin, be.axis, y, x, z);
                let Some(part) = region.get(there) else { continue };
                if part.controller != origin {
                    continue;
                }
                // `removeController(true)`: alone again, to look for neighbours.
                region.modify(there, |p| {
                    p.controller = there;
                    p.width = 1;
                    p.length = 1;
                    p.update_connectivity = true;
                });
                cache.insert(there, Some(there));
            }
        }
    }
}

/// `ItemHelper.calcRedstoneFromInventory` over a whole vault: `fullness` is the sum, over its
/// stacks, of count / min(slot limit, stack size); `non_empty` how many stacks there are and
/// `slots` how many slots the vault has.
pub fn comparator_signal(fullness: f32, non_empty: usize, slots: usize) -> i32 {
    if slots == 0 {
        return 0;
    }
    let f = fullness / slots as f32;
    (f * 14.0).floor() as i32 + i32::from(non_empty > 0)
}

/// What a hopper beside a vault block needs of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    /// It pushes items in: the block needs a free slot.
    Room,
    /// It pulls items out: the block needs something to give.
    Stock,
}

/// Create's vault is one inventory that every block gives and takes through; Horizon's hoppers
/// reach only the slots of the block they touch. So that a hopper on any block of a vault can
/// still fill and empty the whole vault, the block it serves moves one stack to or from the
/// rest, a few times a second: given the stacks each block holds (in the order of [`members`])
/// and the block `at` that has a hopper, the block to take a stack from and the one to put it in.
pub fn serve(counts: &[usize], at: usize, need: Need) -> Option<(usize, usize)> {
    let held = *counts.get(at)?;
    match need {
        Need::Room => {
            if held < SLOTS {
                return None;
            }
            let to = counts.iter().position(|&c| c < SLOTS)?;
            Some((at, to))
        }
        Need::Stock => {
            if held > 0 {
                return None;
            }
            let from = counts.iter().position(|&c| c > 0)?;
            Some((from, at))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    type World = Rc<RefCell<BTreeMap<Pos, Part>>>;

    fn world(positions: &[Pos], axis: Axis) -> World {
        Rc::new(RefCell::new(
            positions.iter().map(|p| (*p, Part::new(*p, axis))).collect(),
        ))
    }

    fn apply(world: &World, changes: Vec<(Pos, Part)>) {
        for (pos, part) in changes {
            world.borrow_mut().insert(pos, part);
        }
    }

    fn form(world: &World, start: Pos) {
        let w = world.clone();
        let mut region = Region::new(move |p| w.borrow().get(&p).cloned());
        form_multi(&mut region, start);
        apply(world, region.changes());
    }

    /// Joins every block that still wants to, as the blocks' own ticks do one after another.
    fn settle(world: &World) {
        loop {
            let todo: Vec<Pos> = world
                .borrow()
                .iter()
                .filter(|(_, p)| p.update_connectivity)
                .map(|(pos, _)| *pos)
                .collect();
            let Some(pos) = todo.first().copied() else { return };
            let is_controller = {
                let mut w = world.borrow_mut();
                let part = w.get_mut(&pos).unwrap();
                part.update_connectivity = false;
                part.is_controller(pos)
            };
            if is_controller {
                form(world, pos);
            }
        }
    }

    fn line(axis: Axis, count: i32) -> Vec<Pos> {
        (0..count).map(|i| at([0, 0, 0], axis, i, 0, 0)).collect()
    }

    #[test]
    fn a_lone_vault_is_one_block() {
        let w = world(&[[0, 0, 0]], Axis::X);
        settle(&w);
        let part = w.borrow()[&[0, 0, 0]].clone();
        assert_eq!((part.width, part.length, part.large()), (1, 1, false));
    }

    #[test]
    fn vaults_in_a_line_join_along_their_axis_with_the_first_as_controller() {
        let w = world(&line(Axis::X, 3), Axis::X);
        settle(&w);
        let w = w.borrow();
        let first = &w[&[0, 0, 0]];
        assert_eq!((first.width, first.length, first.blocks()), (1, 3, 3));
        assert_eq!(w[&[1, 0, 0]].controller, [0, 0, 0]);
        assert_eq!(w[&[2, 0, 0]].controller, [0, 0, 0]);
    }

    #[test]
    fn vaults_side_by_side_across_their_axis_stay_apart_when_the_axis_is_along_the_line() {
        // Two blocks along z whose axis is x: not in line, a square of width 2 needs four.
        let w = world(&[[0, 0, 0], [0, 0, 1]], Axis::X);
        settle(&w);
        let w = w.borrow();
        assert_eq!(w[&[0, 0, 0]].blocks(), 1);
        assert_eq!(w[&[0, 0, 1]].blocks(), 1);
    }

    #[test]
    fn a_length_of_vaults_along_z_joins_too() {
        let w = world(&line(Axis::Z, 3), Axis::Z);
        settle(&w);
        let w = w.borrow();
        assert_eq!(w[&[0, 0, 0]].length, 3);
        assert!(w.values().all(|p| p.controller == [0, 0, 0]));
    }

    #[test]
    fn a_two_by_two_vault_of_length_two_forms_and_a_three_by_three_is_large() {
        let mut blocks = Vec::new();
        for y in 0..2 {
            for x in 0..2 {
                for z in 0..2 {
                    blocks.push(at([0, 0, 0], Axis::X, y, x, z));
                }
            }
        }
        let w = world(&blocks, Axis::X);
        settle(&w);
        {
            let w = w.borrow();
            let c = &w[&[0, 0, 0]];
            assert_eq!((c.width, c.length, c.large()), (2, 2, false));
            assert!(blocks.iter().all(|b| w[b].controller == [0, 0, 0]));
        }
        let mut big = Vec::new();
        for y in 0..3 {
            for x in 0..3 {
                for z in 0..3 {
                    big.push(at([0, 0, 0], Axis::Z, y, x, z));
                }
            }
        }
        let w = world(&big, Axis::Z);
        settle(&w);
        let w = w.borrow();
        let c = &w[&[0, 0, 0]];
        assert_eq!((c.width, c.length, c.large(), c.blocks()), (3, 3, true, 27));
        assert!(big.iter().all(|b| w[b].controller == [0, 0, 0] && w[b].large()));
    }

    #[test]
    fn a_line_does_not_grow_past_three_times_its_width() {
        let w = world(&line(Axis::X, 5), Axis::X);
        settle(&w);
        let w = w.borrow();
        let longest = w.values().map(|p| p.length).max().unwrap();
        assert_eq!(longest, max_length(1));
        // The two left over join each other.
        assert_eq!(w.values().filter(|p| p.length == 2).count(), 2);
    }

    #[test]
    fn vaults_of_different_axes_do_not_join() {
        let w = world(&[[0, 0, 0]], Axis::X);
        w.borrow_mut().insert([1, 0, 0], Part::new([1, 0, 0], Axis::Z));
        settle(&w);
        let w = w.borrow();
        assert_eq!(w[&[0, 0, 0]].blocks(), 1);
        assert_eq!(w[&[1, 0, 0]].blocks(), 1);
    }

    #[test]
    fn a_block_placed_in_front_of_a_vault_joins_it() {
        let w = world(&line(Axis::X, 2), Axis::X);
        settle(&w);
        // One more beyond the end (a higher x).
        w.borrow_mut().insert([2, 0, 0], Part::new([2, 0, 0], Axis::X));
        settle(&w);
        {
            let w = w.borrow();
            assert_eq!(w[&[0, 0, 0]].length, 3);
            assert_eq!(w[&[2, 0, 0]].controller, [0, 0, 0]);
        }
        // And one before the start (a lower x) takes over as controller.
        w.borrow_mut().insert([-1, 0, 0], Part::new([-1, 0, 0], Axis::X));
        settle(&w);
        let w = w.borrow();
        // The vault is already 3 long: the new block can't join, so the line stays as it was.
        assert_eq!(w[&[0, 0, 0]].length, 3);
        assert_eq!(w[&[-1, 0, 0]].blocks(), 1);
    }

    #[test]
    fn a_block_before_the_start_becomes_the_controller_of_a_short_line() {
        let w = world(&line(Axis::X, 2), Axis::X);
        settle(&w);
        w.borrow_mut().insert([-1, 0, 0], Part::new([-1, 0, 0], Axis::X));
        settle(&w);
        let w = w.borrow();
        assert_eq!(w[&[-1, 0, 0]].length, 3);
        assert_eq!(w[&[0, 0, 0]].controller, [-1, 0, 0]);
        assert_eq!(w[&[1, 0, 0]].controller, [-1, 0, 0]);
    }

    #[test]
    fn breaking_a_block_splits_the_vault_into_single_blocks_that_join_again() {
        let w = world(&line(Axis::X, 3), Axis::X);
        settle(&w);
        let removed = w.borrow()[&[1, 0, 0]].clone();
        let wc = w.clone();
        let mut region = Region::new(move |p| wc.borrow().get(&p).cloned());
        split_removed(&mut region, [1, 0, 0], removed);
        apply(&w, region.changes());
        w.borrow_mut().remove(&[1, 0, 0]);
        {
            let w = w.borrow();
            assert!(w.values().all(|p| p.width == 1 && p.length == 1 && p.update_connectivity));
        }
        settle(&w);
        let w = w.borrow();
        assert_eq!(w[&[0, 0, 0]].blocks(), 1);
        assert_eq!(w[&[2, 0, 0]].blocks(), 1);
    }

    #[test]
    fn splitting_a_vault_that_stays_frees_every_block() {
        let w = world(&line(Axis::Z, 3), Axis::Z);
        settle(&w);
        let wc = w.clone();
        let mut region = Region::new(move |p| wc.borrow().get(&p).cloned());
        split(&mut region, [0, 0, 1]);
        apply(&w, region.changes());
        let w = w.borrow();
        assert!(w.values().all(|p| p.length == 1 && p.update_connectivity));
        assert_eq!(w[&[0, 0, 2]].controller, [0, 0, 2]);
    }

    #[test]
    fn members_follow_initcapabilitys_order() {
        let order = members([10, 20, 30], Axis::X, 2, 2);
        assert_eq!(
            order,
            vec![
                [10, 20, 30],
                [10, 20, 31],
                [10, 21, 30],
                [10, 21, 31],
                [11, 20, 30],
                [11, 20, 31],
                [11, 21, 30],
                [11, 21, 31],
            ]
        );
        // Along z the second layer is a step in z, and the two across are x and y.
        let along_z = members([0, 0, 0], Axis::Z, 2, 2);
        assert_eq!(along_z[0], [0, 0, 0]);
        assert_eq!(along_z[1], [0, 1, 0]);
        assert_eq!(along_z[2], [1, 0, 0]);
        assert_eq!(along_z[4], [0, 0, 1]);
    }

    #[test]
    fn the_comparator_reads_the_whole_vault() {
        // Empty: nothing.
        assert_eq!(comparator_signal(0.0, 0, 20), 0);
        // One item in one stack of one block: 1 (the `+ 1` for any stack).
        assert_eq!(comparator_signal(1.0 / 64.0, 1, 20), 1);
        // Full: 14 + 1.
        assert_eq!(comparator_signal(20.0, 20, 20), 15);
        // The same stacks in a vault of three blocks read lower.
        assert_eq!(comparator_signal(10.0, 10, 20), 8);
        assert_eq!(comparator_signal(10.0, 10, 60), 3);
        assert_eq!(comparator_signal(5.0, 5, 0), 0);
    }

    #[test]
    fn a_hopper_on_a_full_block_makes_room_and_one_on_an_empty_block_gets_stock() {
        let counts = [SLOTS, 3, 0];
        assert_eq!(serve(&counts, 0, Need::Room), Some((0, 1)));
        assert_eq!(serve(&counts, 1, Need::Room), None);
        assert_eq!(serve(&counts, 2, Need::Stock), Some((0, 2)));
        assert_eq!(serve(&counts, 1, Need::Stock), None);
        // Nowhere to go, nothing to give.
        assert_eq!(serve(&[SLOTS, SLOTS], 0, Need::Room), None);
        assert_eq!(serve(&[0, 0], 1, Need::Stock), None);
        assert_eq!(serve(&[0, 0], 5, Need::Stock), None);
    }
}
