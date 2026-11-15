//! The Fluid Tank's multi-block rules: which tank blocks join into one wider, taller tank, where
//! the fluid goes when they join or split, which `top` / `bottom` / `shape` each block shows, and
//! where the fluid is drawn. No game types: the glue hands the rules the tank blocks around a
//! position and writes back what changed.
//!
//! Ported from Create `content/fluids/tank/FluidTankBlock`, `FluidTankBlockEntity`,
//! `CreativeFluidTankBlockEntity`, `FluidTankRenderer` and `api/connectivity/ConnectivityHandler`
//! (`formMulti`, `tryToFormNewMulti`, `tryToFormNewMultiOfWidth`, `splitMultiAndInvalidate`).
//!
//! Known differences from Create:
//! - Only fluids (no boiler: `BoilerData`) and no light from the fluid (`luminosity`).
//! - `tryReconnect` of `splitMultiAndInvalidate` is never used by Create and is left out.

use std::collections::{BTreeMap, BTreeSet};

pub type Pos = [i32; 3];

/// `CFluids.fluidTankCapacity`: 8 buckets a block.
pub const CAPACITY: i32 = 8 * 1000;
/// `FluidTankBlockEntity.MAX_SIZE`: the widest a tank grows.
pub const MAX_WIDTH: i32 = 3;
/// `CFluids.fluidTankMaxHeight`.
pub const MAX_HEIGHT: i32 = 32;

/// One tank block as the multi-block rules see it (`FluidTankBlockEntity`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    /// The controller's position: the block's own when it is the controller.
    pub controller: Pos,
    pub width: i32,
    pub height: i32,
    /// The fluid held (the controller's; `None` when empty) and its amount in mB.
    pub fluid: Option<String>,
    pub amount: i32,
    /// The tank's capacity in mB (`tankInventory.getCapacity`), set by [`set_tank_size`].
    pub capacity: i32,
    /// `window`: whether the controller shows windows (`FluidTankBlock.Shape`).
    pub window: bool,
    /// `CreativeSmartFluidTank`: never runs out, voids what is put in.
    pub creative: bool,
    /// `updateConnectivity`: the block still has to look for neighbours to join.
    pub update_connectivity: bool,
}

impl Part {
    /// A freshly placed block: its own controller, one block, windowed (`FluidTankBlockEntity` constructor).
    pub fn new(pos: Pos, creative: bool) -> Self {
        Self {
            controller: pos,
            width: 1,
            height: 1,
            fluid: None,
            amount: 0,
            capacity: CAPACITY,
            window: true,
            creative,
            update_connectivity: true,
        }
    }

    pub fn is_controller(&self, pos: Pos) -> bool {
        self.controller == pos
    }

    /// `FluidTankBlockEntity.getFillState`; a creative tank is always full of what it holds.
    pub fn fill_state(&self) -> f32 {
        if self.capacity <= 0 {
            0.0
        } else {
            self.amount_held() as f32 / self.capacity as f32
        }
    }

    /// `CreativeSmartFluidTank.getFluidAmount`: its whole capacity, or nothing without a fluid.
    pub fn amount_held(&self) -> i32 {
        match (&self.fluid, self.creative) {
            (Some(_), true) => self.capacity,
            (Some(_), false) => self.amount,
            (None, _) => 0,
        }
    }

    /// `FluidTank.fill(EXECUTE)`: what went in. A creative tank takes it all and keeps none.
    pub fn fill(&mut self, fluid: &str, amount: i32) -> i32 {
        if amount <= 0 {
            return 0;
        }
        if self.creative {
            return amount;
        }
        match &self.fluid {
            Some(have) if have != fluid => 0,
            _ => {
                let taken = amount.min(self.capacity - self.amount).max(0);
                if taken > 0 {
                    self.fluid = Some(fluid.to_owned());
                    self.amount += taken;
                }
                taken
            }
        }
    }

    /// `FluidTank.drain(EXECUTE)`: what came out. A creative tank gives and keeps all
    /// (`drain` is always simulated).
    pub fn drain(&mut self, max: i32) -> Option<(String, i32)> {
        let fluid = self.fluid.clone()?;
        let taken = max.min(self.amount_held());
        if taken <= 0 {
            return None;
        }
        if !self.creative {
            self.amount -= taken;
            if self.amount == 0 {
                self.fluid = None;
            }
        }
        Some((fluid, taken))
    }

    /// `CreativeSmartFluidTank.setContainedFluid`.
    pub fn set_contained(&mut self, fluid: Option<String>) {
        self.amount = if fluid.is_some() { self.capacity } else { 0 };
        self.fluid = fluid;
    }
}

/// `applyFluidTankSize`: the tank holds `blocks` blocks' worth; what no longer fits is drained.
pub fn set_tank_size(part: &mut Part, blocks: i32) {
    part.capacity = blocks * CAPACITY;
    if part.creative {
        if part.fluid.is_some() {
            part.amount = part.capacity;
        }
        return;
    }
    if part.amount > part.capacity {
        part.amount = part.capacity;
    }
    if part.amount == 0 {
        part.fluid = None;
    }
}

/// The tank blocks around a position, loaded on demand and kept with their changes.
pub struct Region<L: FnMut(Pos) -> Option<Part>> {
    load: L,
    parts: BTreeMap<Pos, Option<Part>>,
    changed: BTreeSet<Pos>,
    /// A block going away right now: the game has not removed it yet, but `partAt` must not
    /// find it (`BlockEntity.isRemoved`).
    removed: Option<(Pos, Part)>,
}

impl<L: FnMut(Pos) -> Option<Part>> Region<L> {
    /// `load` answers the tank block (of the one kind being joined) at a position.
    pub fn new(load: L) -> Self {
        Self {
            load,
            parts: BTreeMap::new(),
            changed: BTreeSet::new(),
            removed: None,
        }
    }

    /// `partAt`: the block at `pos`, if it is one of these tanks and not being removed.
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

fn offset(origin: Pos, x: i32, y: i32, z: i32) -> Pos {
    [origin[0] + x, origin[1] + y, origin[2] + z]
}

/// `ConnectivityHandler.formMulti`: the block at `start` (a controller) looks for neighbours to
/// join, and the tanks that come out of it are the best way to group the blocks found.
pub fn form_multi<L: FnMut(Pos) -> Option<Part>>(region: &mut Region<L>, start: Pos) {
    let mut cache = Cache::new();
    let mut frontier = vec![start];
    let mut creation: Vec<(i32, Pos)> = Vec::new();
    let mut visited: BTreeSet<Pos> = BTreeSet::new();

    // The vertical search is bounded by the widest tank on x and z, and not at all on y.
    let min_x = start[0] - MAX_WIDTH;
    let min_z = start[2] - MAX_WIDTH;

    while !frontier.is_empty() {
        let part = frontier.remove(0);
        if !visited.insert(part) {
            continue;
        }
        let amount = try_to_form(region, &mut cache, part, true);
        if amount > 1 {
            creation.push((amount, part));
        }
        for axis in 0..3 {
            let mut next = part;
            next[axis] -= 1;
            if next[0] <= min_x || next[2] <= min_z || visited.contains(&next) {
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
        if part.width == best_width && part.width * part.width * part.height == best_amount {
            return best_amount;
        }
        split_with(region, cache, be, false);
        region.modify(be, |p| set_tank_size(p, best_amount));
        of_width(region, cache, be, best_width, false);
        region.modify(be, |p| {
            p.update_connectivity = false;
            p.width = best_width;
            p.height = best_amount / best_width / best_width;
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
    let mut height = 0;
    let Some(be) = region.get(origin) else {
        return 0;
    };
    let fluid = be.fluid.clone();

    'search: for y in 0..MAX_HEIGHT {
        for x in 0..width {
            for z in 0..width {
                let pos = offset(origin, x, y, z);
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
                if other_width == width && controller.height == MAX_HEIGHT {
                    break 'search;
                }
                if controller_pos != origin {
                    if controller_pos[0] < origin[0] || controller_pos[2] < origin[2] {
                        break 'search;
                    }
                    if controller_pos[0] + other_width > origin[0] + width
                        || controller_pos[2] + other_width > origin[2] + width
                    {
                        break 'search;
                    }
                }
                if let (Some(a), Some(b)) = (&fluid, &controller.fluid)
                    && a != b
                {
                    break 'search;
                }
            }
        }
        amount += width * width;
        height += 1;
    }

    if simulate {
        return amount;
    }

    for y in 0..height {
        for x in 0..width {
            for z in 0..width {
                let pos = offset(origin, x, y, z);
                let Some(part) = region.get(pos) else {
                    continue;
                };
                if pos == origin {
                    continue;
                }
                if let Some(held) = part.fluid.clone() {
                    let creative_be = be.creative;
                    region.modify(origin, |tank| {
                        // `beTank.setContainedFluid` for an empty creative tank (by what it held
                        // when the search began), then `beTank.fill`.
                        if creative_be && fluid.is_none() {
                            tank.set_contained(Some(held.clone()));
                        }
                        tank.fill(&held, part.amount_held());
                    });
                    if !part.creative {
                        region.modify(pos, |p| {
                            p.fluid = None;
                            p.amount = 0;
                        });
                    }
                }
                split_with(region, cache, pos, false);
                region.modify(pos, |p| {
                    p.controller = origin;
                    p.update_connectivity = false;
                    p.height = height;
                    p.width = width;
                });
                cache.insert(pos, Some(origin));
            }
        }
    }
    amount
}

/// `FluidTankBlockEntity.setWindows`: every block of the tank whose controller is at `controller`
/// takes the window state `window`; the changed blocks' states are then given by [`block_state`].
pub fn set_windows<L: FnMut(Pos) -> Option<Part>>(
    region: &mut Region<L>,
    controller: Pos,
    window: bool,
) {
    let Some(be) = region.get(controller) else {
        return;
    };
    for y in 0..be.height {
        for x in 0..be.width {
            for z in 0..be.width {
                region.modify(offset(controller, x, y, z), |p| p.window = window);
            }
        }
    }
}

/// `ConnectivityHandler.splitMulti`: the tank block at `pos` is going away (or changing kind),
/// so the whole tank it is part of comes apart into single blocks, each keeping what fluid fits
/// (`splitMultiAndInvalidate`). `part` is the block as it is now.
pub fn split_removed<L: FnMut(Pos) -> Option<Part>>(
    region: &mut Region<L>,
    pos: Pos,
    part: Part,
) {
    region.removed = Some((pos, part));
    let mut cache = Cache::new();
    split_with(region, &mut cache, pos, false);
}

fn split_with<L: FnMut(Pos) -> Option<Part>>(
    region: &mut Region<L>,
    cache: &mut Cache,
    pos: Pos,
    _try_reconnect: bool,
) {
    // `be = be.getControllerBE()`: the controller of the tank the block belongs to.
    let removed = region.removed.clone();
    let (origin, be, be_removed) = match removed {
        Some((removed_pos, removed_part)) if removed_pos == pos => {
            if removed_part.controller == removed_pos {
                (removed_pos, removed_part, true)
            } else {
                let Some(controller) = region.get(removed_part.controller) else {
                    return;
                };
                (removed_part.controller, controller, false)
            }
        }
        _ => {
            let Some(part) = region.get(pos) else { return };
            let Some(controller) = region.get(part.controller) else {
                return;
            };
            (part.controller, controller, false)
        }
    };
    let (height, width) = (be.height, be.width);
    if width == 1 && height == 1 {
        return;
    }

    // `toDistribute`: the controller's fluid, less what it keeps itself (a block's worth).
    let mut distribute = (be.fluid.clone(), be.amount_held());
    if distribute.0.is_some() && !be_removed {
        distribute.1 -= CAPACITY;
        if distribute.1 <= 0 {
            distribute = (None, 0);
        }
    }
    if !be_removed {
        region.modify(origin, |p| set_tank_size(p, 1));
    }

    for y in 0..height {
        for x in 0..width {
            for z in 0..width {
                let at = offset(origin, x, y, z);
                let Some(part) = region.get(at) else { continue };
                if part.controller != origin {
                    continue;
                }
                let window = be.window;
                region.modify(at, |p| {
                    // `setExtraData` then `removeController(true)`.
                    p.window = window;
                    p.controller = at;
                    p.width = 1;
                    p.height = 1;
                    p.update_connectivity = true;
                });
                if let (Some(fluid), true) = (&distribute.0, at != origin) {
                    let fluid = fluid.clone();
                    if part.creative {
                        if part.fluid.is_none() {
                            region.modify(at, |p| p.set_contained(Some(fluid.clone())));
                        }
                    } else {
                        let split = CAPACITY.min(distribute.1);
                        distribute.1 -= split;
                        region.modify(at, |p| {
                            p.fill(&fluid, split);
                        });
                    }
                }
                cache.insert(at, Some(at));
            }
        }
    }
}

/// `FluidTankBlock.Shape`, as its state value names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Plain,
    Window,
    WindowNw,
    WindowSw,
    WindowNe,
    WindowSe,
}

impl Shape {
    pub const fn name(self) -> &'static str {
        match self {
            Shape::Plain => "plain",
            Shape::Window => "window",
            Shape::WindowNw => "window_nw",
            Shape::WindowSw => "window_sw",
            Shape::WindowNe => "window_ne",
            Shape::WindowSe => "window_se",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [
            Shape::Plain,
            Shape::Window,
            Shape::WindowNw,
            Shape::WindowSw,
            Shape::WindowNe,
            Shape::WindowSe,
        ]
        .into_iter()
        .find(|s| s.name() == name)
    }

    /// `FluidTankBlock.rotate` once: a quarter turn clockwise from above.
    pub const fn rotated(self) -> Self {
        match self {
            Shape::WindowNe => Shape::WindowSe,
            Shape::WindowNw => Shape::WindowNe,
            Shape::WindowSe => Shape::WindowSw,
            Shape::WindowSw => Shape::WindowNw,
            other => other,
        }
    }
}

/// The `top`, `bottom` and `shape` the block at `pos` shows, given its controller at `controller`
/// and the controller's `width`, `height` and `window`: `notifyMultiUpdated` and `setWindows`,
/// and `removeController` for a block that stands alone.
pub fn block_state(pos: Pos, controller: Pos, width: i32, height: i32, window: bool) -> (bool, bool, Shape) {
    let bottom = controller[1] == pos[1];
    let top = controller[1] + height - 1 == pos[1];
    let (x_offset, z_offset) = (pos[0] - controller[0], pos[2] - controller[2]);
    let mut shape = Shape::Plain;
    if window {
        // Size 1: every tank has a window.
        if width == 1 {
            shape = Shape::Window;
        }
        // Size 2: every tank has a corner window.
        if width == 2 {
            shape = if x_offset == 0 {
                if z_offset == 0 { Shape::WindowNw } else { Shape::WindowSw }
            } else if z_offset == 0 {
                Shape::WindowNe
            } else {
                Shape::WindowSe
            };
        }
        // Size 3: the tanks in the middle of a side have a window.
        if width == 3 && (x_offset.abs() - z_offset.abs()).abs() == 1 {
            shape = Shape::Window;
        }
    }
    (bottom, top, shape)
}

/// `FluidTankRenderer.renderSafe`: the fluid as a box in blocks from the controller's corner,
/// `[x0, y0, z0, x1, y1, z1]`, for a tank `width` wide and `height` tall at fill `level`
/// (0 to 1). `None` when there is too little to see.
pub fn fluid_bounds(width: i32, height: i32, level: f32, lighter_than_air: bool) -> Option<[f32; 6]> {
    let cap_height = 1.0 / 4.0;
    let hull = 1.0 / 16.0 + 1.0 / 128.0;
    let min_puddle = 1.0 / 16.0;
    let total_height = height as f32 - 2.0 * cap_height - min_puddle;
    if level < 1.0 / (512.0 * total_height) {
        return None;
    }
    let clamped = (level * total_height).clamp(0.0, total_height);
    let x_min = hull;
    let x_max = x_min + width as f32 - 2.0 * hull;
    let mut y_min = total_height + cap_height + min_puddle - clamped;
    let mut y_max = y_min + clamped;
    if lighter_than_air {
        y_min += total_height - clamped;
        y_max += total_height - clamped;
    }
    // `ms.translate(0, clampedLevel - totalHeight, 0)`.
    let shift = clamped - total_height;
    Some([x_min, y_min + shift, hull, x_max, y_max + shift, hull + width as f32 - 2.0 * hull])
}

/// `ComparatorUtil.fractionToRedstoneLevel`.
pub fn comparator_level(fraction: f32) -> i32 {
    if fraction <= 0.0 { 0 } else { (1.0 + fraction * 14.0).floor() as i32 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::rc::Rc;

    type World = Rc<RefCell<BTreeMap<Pos, Part>>>;

    fn world(positions: &[Pos]) -> World {
        Rc::new(RefCell::new(
            positions.iter().map(|p| (*p, Part::new(*p, false))).collect(),
        ))
    }

    fn apply(world: &World, changes: Vec<(Pos, Part)>) {
        for (pos, part) in changes {
            world.borrow_mut().insert(pos, part);
        }
    }

    fn form(world: &World, at: Pos) {
        let w = world.clone();
        let mut region = Region::new(move |p| w.borrow().get(&p).cloned());
        form_multi(&mut region, at);
        apply(world, region.changes());
    }

    /// Joins every block that still wants to, as the blocks' own ticks do one after another.
    fn settle(world: &World) {
        loop {
            let todo: Vec<Pos> = world
                .borrow()
                .iter()
                .filter(|(pos, p)| p.update_connectivity && p.is_controller(**pos))
                .map(|(pos, _)| *pos)
                .collect();
            let Some(pos) = todo.first().copied() else { return };
            world.borrow_mut().get_mut(&pos).unwrap().update_connectivity = false;
            form(world, pos);
        }
    }

    #[test]
    fn a_lone_tank_is_one_block() {
        let w = world(&[[0, 0, 0]]);
        settle(&w);
        let part = w.borrow()[&[0, 0, 0]].clone();
        assert_eq!((part.width, part.height, part.capacity), (1, 1, CAPACITY));
    }

    #[test]
    fn tanks_stack_into_a_column_with_the_lowest_as_controller() {
        let w = world(&[[0, 0, 0], [0, 1, 0], [0, 2, 0]]);
        settle(&w);
        let w = w.borrow();
        let bottom = &w[&[0, 0, 0]];
        assert_eq!((bottom.width, bottom.height, bottom.capacity), (1, 3, 3 * CAPACITY));
        assert_eq!(w[&[0, 1, 0]].controller, [0, 0, 0]);
        assert_eq!(w[&[0, 2, 0]].controller, [0, 0, 0]);
    }

    #[test]
    fn two_by_two_tanks_join_and_are_wider_than_they_are_tall() {
        let w = world(&[[0, 0, 0], [1, 0, 0], [0, 0, 1], [1, 0, 1]]);
        settle(&w);
        let w = w.borrow();
        let controller = w.values().find(|p| p.width == 2).expect("a 2x2 tank");
        assert_eq!((controller.height, controller.capacity), (1, 4 * CAPACITY));
        let controllers = w.iter().filter(|(pos, p)| p.is_controller(**pos)).count();
        assert_eq!(controllers, 1);
    }

    #[test]
    fn a_three_by_three_by_two_tank_forms() {
        let mut blocks = Vec::new();
        for y in 0..2 {
            for x in 0..3 {
                for z in 0..3 {
                    blocks.push([x, y, z]);
                }
            }
        }
        let w = world(&blocks);
        settle(&w);
        let w = w.borrow();
        let c = &w[&[0, 0, 0]];
        assert_eq!((c.width, c.height, c.capacity), (3, 2, 18 * CAPACITY));
        assert!(blocks.iter().all(|b| w[b].controller == [0, 0, 0]));
    }

    #[test]
    fn a_column_does_not_grow_past_the_maximum_height() {
        let blocks: Vec<Pos> = (0..MAX_HEIGHT + 2).map(|y| [0, y, 0]).collect();
        let w = world(&blocks);
        settle(&w);
        let w = w.borrow();
        let tallest = w.values().map(|p| p.height).max().unwrap();
        assert_eq!(tallest, MAX_HEIGHT);
    }

    #[test]
    fn fluid_follows_the_controller_when_tanks_join() {
        let w = world(&[[0, 0, 0], [0, 1, 0]]);
        {
            let mut w = w.borrow_mut();
            let lower = w.get_mut(&[0, 0, 0]).unwrap();
            lower.update_connectivity = false;
            lower.fill("minecraft:water", 5000);
            let upper = w.get_mut(&[0, 1, 0]).unwrap();
            upper.update_connectivity = false;
            upper.fill("minecraft:water", 3000);
        }
        form(&w, [0, 0, 0]);
        let w = w.borrow();
        assert_eq!(w[&[0, 0, 0]].amount, 8000);
        assert_eq!(w[&[0, 0, 0]].capacity, 2 * CAPACITY);
        assert_eq!(w[&[0, 1, 0]].fluid, None);
    }

    #[test]
    fn tanks_holding_different_fluids_do_not_join() {
        let w = world(&[[0, 0, 0], [0, 1, 0]]);
        {
            let mut w = w.borrow_mut();
            w.get_mut(&[0, 0, 0]).unwrap().fill("minecraft:water", 100);
            w.get_mut(&[0, 1, 0]).unwrap().fill("minecraft:lava", 100);
        }
        settle(&w);
        let w = w.borrow();
        assert_eq!(w[&[0, 0, 0]].height, 1);
        assert_eq!(w[&[0, 1, 0]].controller, [0, 1, 0]);
    }

    #[test]
    fn breaking_a_tank_splits_the_column_and_hands_out_the_fluid() {
        let w = world(&[[0, 0, 0], [0, 1, 0], [0, 2, 0]]);
        settle(&w);
        w.borrow_mut().get_mut(&[0, 0, 0]).unwrap().fill("minecraft:water", 20000);
        // The middle block is broken.
        let removed = w.borrow()[&[0, 1, 0]].clone();
        let wc = w.clone();
        let mut region = Region::new(move |p| wc.borrow().get(&p).cloned());
        split_removed(&mut region, [0, 1, 0], removed);
        apply(&w, region.changes());
        w.borrow_mut().remove(&[0, 1, 0]);
        settle(&w);
        let w = w.borrow();
        assert_eq!(w[&[0, 0, 0]].height, 1);
        assert_eq!(w[&[0, 0, 0]].amount, CAPACITY);
        assert_eq!(w[&[0, 2, 0]].amount, CAPACITY);
    }

    #[test]
    fn breaking_the_controller_keeps_the_fluid_in_the_rest() {
        let w = world(&[[0, 0, 0], [0, 1, 0]]);
        settle(&w);
        w.borrow_mut().get_mut(&[0, 0, 0]).unwrap().fill("minecraft:water", 5000);
        let removed = w.borrow()[&[0, 0, 0]].clone();
        let wc = w.clone();
        let mut region = Region::new(move |p| wc.borrow().get(&p).cloned());
        split_removed(&mut region, [0, 0, 0], removed);
        apply(&w, region.changes());
        w.borrow_mut().remove(&[0, 0, 0]);
        let top = w.borrow()[&[0, 1, 0]].clone();
        assert_eq!((top.amount, top.controller, top.height), (5000, [0, 1, 0], 1));
    }

    #[test]
    fn a_creative_tank_fills_the_ones_it_joins_with_its_fluid() {
        let w = Rc::new(RefCell::new(BTreeMap::new()));
        for pos in [[0, 0, 0], [0, 1, 0]] {
            w.borrow_mut().insert(pos, Part::new(pos, true));
        }
        w.borrow_mut().get_mut(&[0, 1, 0]).unwrap().set_contained(Some("minecraft:lava".into()));
        w.borrow_mut().get_mut(&[0, 1, 0]).unwrap().update_connectivity = false;
        form(&w, [0, 0, 0]);
        let w = w.borrow();
        assert_eq!(w[&[0, 0, 0]].fluid.as_deref(), Some("minecraft:lava"));
        assert_eq!(w[&[0, 0, 0]].amount_held(), 2 * CAPACITY);
    }

    #[test]
    fn a_creative_tank_never_runs_out_and_voids_what_it_is_given() {
        let mut tank = Part::new([0, 0, 0], true);
        assert_eq!(tank.fill("minecraft:water", 500), 500);
        assert_eq!(tank.fluid, None);
        tank.set_contained(Some("minecraft:water".into()));
        assert_eq!(tank.drain(300), Some(("minecraft:water".into(), 300)));
        assert_eq!(tank.drain(300), Some(("minecraft:water".into(), 300)));
        assert_eq!(tank.amount_held(), CAPACITY);
    }

    #[test]
    fn shapes_follow_the_width() {
        let c = [0, 0, 0];
        assert_eq!(block_state(c, c, 1, 1, true), (true, true, Shape::Window));
        assert_eq!(block_state(c, c, 1, 1, false), (true, true, Shape::Plain));
        assert_eq!(block_state([1, 0, 0], c, 2, 1, true).2, Shape::WindowNe);
        assert_eq!(block_state([0, 0, 1], c, 2, 1, true).2, Shape::WindowSw);
        assert_eq!(block_state([1, 0, 1], c, 2, 1, true).2, Shape::WindowSe);
        assert_eq!(block_state(c, c, 2, 1, true).2, Shape::WindowNw);
        // 3 wide: windows in the middle of the sides, plain in the corners and the middle.
        assert_eq!(block_state([1, 0, 0], c, 3, 1, true).2, Shape::Window);
        assert_eq!(block_state([1, 0, 1], c, 3, 1, true).2, Shape::Plain);
        assert_eq!(block_state([2, 0, 2], c, 3, 1, true).2, Shape::Plain);
    }

    #[test]
    fn top_and_bottom_mark_the_ends_of_a_column() {
        let c = [5, 10, 5];
        assert_eq!(block_state(c, c, 1, 3, true), (true, false, Shape::Window));
        assert_eq!(block_state([5, 11, 5], c, 1, 3, true), (false, false, Shape::Window));
        assert_eq!(block_state([5, 12, 5], c, 1, 3, true), (false, true, Shape::Window));
    }

    #[test]
    fn the_fluid_box_sits_inside_the_caps() {
        let [x0, y0, z0, x1, y1, z1] = fluid_bounds(1, 1, 1.0, false).unwrap();
        assert!((x0 - 0.0703125).abs() < 1e-6 && (z0 - x0).abs() < 1e-6);
        assert!((x1 - (1.0 - x0)).abs() < 1e-6 && (z1 - x1).abs() < 1e-6);
        // One block: caps of a quarter above and below, a sixteenth of puddle.
        assert!((y0 - 5.0 / 16.0).abs() < 1e-6);
        assert!((y1 - 0.75).abs() < 1e-6);
        assert!(fluid_bounds(1, 1, 0.0, false).is_none());
    }

    #[test]
    fn the_comparator_reads_the_fill() {
        assert_eq!(comparator_level(0.0), 0);
        assert_eq!(comparator_level(1.0), 15);
        assert_eq!(comparator_level(0.5), 8);
    }
}
