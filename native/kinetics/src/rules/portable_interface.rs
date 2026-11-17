//! The Portable Storage Interface and the Portable Fluid Interface's rules: when a stationary one
//! links with one on a moving structure and for how long, how the moving one finds the stationary
//! one and holds the structure still, what the link looks like, and what the stationary one does
//! with the structure's inventories and tanks. Ported from Create (MIT, the Creators of Create),
//! `mc1.21.1/dev` at 4148fd8: `PortableStorageInterfaceBlockEntity` (`tick`, `startTransferringTo`,
//! `stopTransferring`, `canTransfer`, `isConnected`, `onContentTransferred`, `neighbourChanged`,
//! `getExtensionDistance`), `PortableItemInterfaceBlockEntity` and `PortableFluidInterfaceBlockEntity`
//! (the handlers), `PortableStorageInterfaceMovement` (`findInterface`, `tick`, `reset`,
//! `getCurrentFacingIfValid`, `getActiveAreaOffset`), `PortableStorageInterfaceBlock`
//! (`getStateForPlacement`, the comparator), `PortableStorageInterfaceRenderer`, `CLogistics.psiTimeout`,
//! `AllShapes.PORTABLE_STORAGE_INTERFACE`, `ItemHandlerHelper.insertItemStacked`,
//! `CombinedTankWrapper` and the mounted storage types (`ChestMountedStorageType`,
//! `SimpleMountedStorageType` with `create:simple_mounted_storage`, `ItemVaultMountedStorageType`,
//! `FluidTankMountedStorageType`).
//!
//! No game types: the glue is `native/api/src/machines/portable_interface*.rs`.
//!
//! Known differences from Create:
//! - Which blocks of a structure are storage is by name: the chest and trapped chest (a double
//!   chest is two halves, each its own storage as Create mounts them), the barrel, the shulker boxes,
//!   the Item Vault and the Fluid Tank (its controller). The Creative Crate, the Creative Fluid
//!   Tank, the Depot, the Dispenser and the Toolbox aren't mounted, nor are another mod's containers
//!   (`FallbackMountedStorage`).
//! - The combined inventory lists the storages by their position in the structure, as a sorted map
//!   does; Create's is a hash map's order.
//! - The structure's storage is read and changed in the controller's saved structure, so it
//!   isn't synced to a client showing the contents (the interface and the screens read nothing of
//!   it).

use crate::rules::contraption::rotate_pos;
use crate::{Axis, Dir, Pos};

pub const ITEM_INTERFACE: &str = "create:portable_storage_interface";
pub const FLUID_INTERFACE: &str = "create:portable_fluid_interface";

/// `PortableStorageInterfaceBlockEntity.ANIMATION`.
pub const ANIMATION: i32 = 4;
/// `CLogistics.psiTimeout`: the ticks an interface waits for transfers before the structure goes on.
pub const TIMEOUT: i32 = 60;
/// What a moving interface sets the stationary one's `keepAlive` to each tick.
pub const KEEP_ALIVE: i32 = 2;
/// `PortableStorageInterfaceMovement.getActiveAreaOffset`: how far in front of itself a moving
/// interface looks for the stationary one.
pub const ACTIVE_AREA: f64 = 1.85;
/// How far the interface reaches at most (`Math.min(2, distance)`).
pub const MAX_DISTANCE: f32 = 2.0;
/// `AllShapes.PORTABLE_STORAGE_INTERFACE = shape(0, 0, 0, 16, 14, 16).forDirectional()` facing up.
pub const SHAPE_HEIGHT: f32 = 14.0 / 16.0;

pub fn is_interface(block: &str) -> bool {
    block == ITEM_INTERFACE || block == FLUID_INTERFACE
}

// --- the link of a stationary interface (server) ----------------------------------------------------------

/// A stationary interface's link with a structure: `transferTimer`, `keepAlive`, `powered`,
/// `connectedEntity` (the controller the structure belongs to) and `distance`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Link {
    pub timer: i32,
    pub keep_alive: i32,
    pub powered: bool,
    pub controller: Option<Pos>,
    pub distance: f32,
}

/// What a tick of the link did that the glue must act on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Step {
    /// `stopTransferring` ran: the handlers go empty.
    pub stopped: bool,
    /// `sendData`: the clients are told.
    pub send: bool,
}

impl Default for Link {
    fn default() -> Self {
        Link { timer: 0, keep_alive: 0, powered: false, controller: None, distance: 0.0 }
    }
}

impl Link {
    /// `isConnected`: the timer is in the window where contents may move.
    pub fn is_connected(&self, timeout: i32) -> bool {
        self.timer >= ANIMATION && self.timer <= timeout + ANIMATION
    }

    /// `canTransfer`: linked with a structure and connected.
    pub fn can_transfer(&self, timeout: i32) -> bool {
        self.controller.is_some() && self.is_connected(timeout)
    }

    /// `isTransferring`.
    pub fn is_transferring(&self) -> bool {
        self.timer > ANIMATION
    }

    /// `startTransferringTo`: links with the structure of `controller`, `distance` away. Does
    /// nothing when already linked with it. Whether it linked.
    pub fn start(&mut self, controller: Pos, distance: f32) -> bool {
        if self.controller == Some(controller) {
            return false;
        }
        self.distance = distance.min(MAX_DISTANCE);
        self.controller = Some(controller);
        // `startConnecting`.
        self.timer = 6 + ANIMATION * 2;
        true
    }

    /// `stopTransferring`.
    pub fn stop(&mut self) {
        self.controller = None;
    }

    /// `onContentTransferred`: something moved, so the structure waits another timeout.
    pub fn content_transferred(&mut self, timeout: i32) {
        self.timer = timeout + ANIMATION;
    }

    /// `neighbourChanged` and `initialize`: whether the interface became powered (a powered one
    /// lets go). Answers true when the power changed, and then whether structures near should
    /// look for it again (it was switched off).
    pub fn power(&mut self, powered_now: bool) -> Option<bool> {
        if powered_now == self.powered {
            return None;
        }
        self.powered = powered_now;
        if powered_now {
            self.stop();
        }
        Some(!powered_now)
    }

    /// One server tick of `PortableStorageInterfaceBlockEntity.tick`.
    pub fn tick(&mut self, timeout: i32) -> Step {
        let mut step = Step::default();
        if self.keep_alive > 0 {
            self.keep_alive -= 1;
            if self.keep_alive == 0 {
                self.stop();
                self.timer = ANIMATION - 1;
                step.stopped = true;
                step.send = true;
                return step;
            }
        }
        self.timer = self.timer.min(ANIMATION * 2 + timeout);
        let can_decrement = self.timer > ANIMATION || (self.timer > 0 && self.keep_alive == 0);
        if can_decrement {
            self.timer -= 1;
            if self.timer == ANIMATION - 1 {
                step.send = true;
            }
            if self.timer <= 0 || self.powered {
                self.stop();
                step.stopped = true;
            }
        }
        step
    }
}

/// The timer `now` for a link saved as `timer` at game time `since`: what a client works from
/// without ticking. While a structure holds the link (`hold`, `keepAlive` above zero) the timer runs
/// down to [`ANIMATION`] and stays; once it let go it runs down to zero.
pub fn timer_at(timer: i32, since: i64, hold: bool, now: i64) -> i32 {
    let elapsed = (now - since).clamp(0, i64::from(i32::MAX)) as i32;
    if hold {
        if timer > ANIMATION { (timer - elapsed).max(ANIMATION) } else { timer }
    } else {
        (timer - elapsed).max(0)
    }
}

/// `tick`'s `progress`: how far the interface has come out, 0 to 1.
pub fn progress(timer: i32, timeout: i32) -> f32 {
    let connected = timer >= ANIMATION && timer <= timeout + ANIMATION;
    if connected {
        1.0
    } else if timer >= timeout + ANIMATION {
        // `Mth.lerp((timer - timeUnit - animation) / animation, 1, 0)`.
        1.0 - (timer - timeout - ANIMATION) as f32 / ANIMATION as f32
    } else if timer < ANIMATION {
        timer as f32 / ANIMATION as f32
    } else {
        0.0
    }
}

/// `getExtensionDistance`: how far the middle and top have moved out, in blocks.
pub fn extension(progress: f32, distance: f32) -> f32 {
    progress * progress * distance / 2.0
}

/// `PortableStorageInterfaceRenderer.render`: how far the middle and the top are moved from the base
/// along the interface's facing (blocks), the middle half as far plus 3/8.
pub fn middle_and_top(extension: f32) -> (f32, f32) {
    (extension * 0.5 + 0.375, extension)
}

/// The comparator's reading (`getAnalogOutputSignal`): 15 while connected.
pub fn analog_output(connected: bool) -> i32 {
    if connected { 15 } else { 0 }
}

// --- placing the block ------------------------------------------------------------------------------------

/// `PortableStorageInterfaceBlock.getStateForPlacement`: `facing` is the opposite of the direction
/// the player looks (`getNearestLookingDirection`, reversed when sneaking).
pub fn placement_facing(nearest_looking: Dir, sneaking: bool) -> Dir {
    let direction = if sneaking { nearest_looking.opposite() } else { nearest_looking };
    direction.opposite()
}

// --- the moving interface -------------------------------------------------------------------------------------

/// `Direction.getNearest(x, y, z)`.
pub fn nearest_direction(v: [f64; 3]) -> Dir {
    let mut best = Dir::North;
    let mut best_dot = f64::NEG_INFINITY;
    for dir in [Dir::Down, Dir::Up, Dir::North, Dir::South, Dir::West, Dir::East] {
        let n = dir.normal();
        let dot = v[0] * f64::from(n[0]) + v[1] * f64::from(n[1]) + v[2] * f64::from(n[2]);
        if dot > best_dot {
            best_dot = dot;
            best = dir;
        }
    }
    best
}

/// `getCurrentFacingIfValid`: the direction an interface of `facing` points in after the
/// structure turned (`rotated` is the turned normal), or none while it is turned too far from a
/// right angle (more than half a block's length off).
pub fn current_facing(rotated: [f64; 3]) -> Option<Dir> {
    let nearest = nearest_direction(rotated);
    let n = nearest.normal();
    let d = (0..3).map(|i| (rotated[i] - f64::from(n[i])).powi(2)).sum::<f64>().sqrt();
    (d <= 0.5).then_some(nearest)
}

/// A vector turned `degrees` right-handed about the positive end of `axis` (the sense the bearing
/// turns its structure in, `Transform`'s quarter turns).
pub fn rotate_vec(v: [f64; 3], axis: Axis, degrees: f64) -> [f64; 3] {
    let (s, c) = degrees.to_radians().sin_cos();
    let [x, y, z] = v;
    match axis {
        Axis::X => [x, y * c - z * s, y * s + z * c],
        Axis::Y => [x * c + z * s, y, -x * s + z * c],
        Axis::Z => [x * c - y * s, x * s + y * c, z],
    }
}

/// What a stationary interface looks like to a moving one that comes by (`getStationaryInterfaceAt`):
/// its facing, when it is the same block and not powered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stationary {
    pub facing: Dir,
    pub powered: bool,
}

fn offset(pos: Pos, dir: Dir, n: i32) -> Pos {
    let d = dir.normal();
    [pos[0] + d[0] * n, pos[1] + d[1] * n, pos[2] + d[2] * n]
}

/// `getStationaryInterfaceAt`: whether the interface at a cell is one the moving one (`facing`
/// now) can link with: facing it and not powered.
pub fn links_with(stationary: Option<Stationary>, facing: Dir) -> bool {
    stationary.is_some_and(|s| s.facing == facing.opposite() && !s.powered)
}

/// `findStationaryInterface`: the first of the cell and the one beyond it (in `facing`'s
/// direction) holding an interface the moving one can link with.
pub fn find_interface(cell: Pos, facing: Dir, at: &dyn Fn(Pos) -> Option<Stationary>) -> Option<Pos> {
    (0..2).map(|i| offset(cell, facing, i)).find(|pos| links_with(at(*pos), facing))
}

/// `VecHelper.getCenterOf`.
pub fn centre(pos: Pos) -> [f64; 3] {
    [f64::from(pos[0]) + 0.5, f64::from(pos[1]) + 0.5, f64::from(pos[2]) + 0.5]
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f64>().sqrt()
}

/// `findInterface`'s distance: how far the stationary interface's centre is from the moving one
/// along the facing, plus the reach less the interface's own block (`diff.length() + 1.85 - 1`).
pub fn link_distance(stationary: Pos, position: [f64; 3], facing: Dir) -> f32 {
    let n = facing.normal();
    let diff = centre(stationary);
    let along: f64 = (0..3).map(|i| (diff[i] - position[i]) * f64::from(n[i]).abs()).sum::<f64>();
    (along.abs() + ACTIVE_AREA - 1.0) as f32
}

/// `position.closerThan(target, target.distanceTo(position + motion))`: the structure is at its
/// closest to the target, and about to move away.
pub fn at_closest(position: [f64; 3], motion: [f64; 3], target: [f64; 3]) -> bool {
    let next = [position[0] + motion[0], position[1] + motion[1], position[2] + motion[2]];
    distance(position, target) < distance(next, target)
}

/// The speed a structure follows: none while an interface holds it still (Create's
/// `Contraption.stalled`). The controller's real speed still decides assembling and setting down.
pub fn moving_speed(stalled: bool, rpm: f32) -> f32 {
    if stalled { 0.0 } else { rpm }
}

/// One moving interface's memory: the stationary one it works with (`WorkingPos`) and whether it
/// holds the structure still (`context.stall`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Actor {
    pub working: Option<Pos>,
    pub stall: bool,
}

impl Actor {
    /// `visitNewPosition` (a new grid cell): the stationary interface found there, or none.
    pub fn visit(&mut self, found: Option<Pos>) {
        self.working = found;
    }

    /// `reset`.
    pub fn reset(&mut self) {
        self.working = None;
        self.stall = false;
    }

    /// The first part of `tick` for an actor that has a working interface: whether the structure
    /// has come to its closest and stalls now.
    pub fn arrive(&mut self, position: [f64; 3], motion: [f64; 3], target: [f64; 3]) {
        if !self.stall && at_closest(position, motion, target) {
            self.stall = true;
        }
    }

    /// The last part of `tick`: a stalled structure goes on once the link's timer is down to the
    /// animation (nothing more to exchange for the timeout).
    pub fn release(&mut self, link_timer: i32) {
        if self.stall && link_timer <= ANIMATION {
            self.stall = false;
        }
    }
}

// --- the structure's storage -------------------------------------------------------------------------------------

/// A stack as the storage maths sees it: what it is (item and components), how many and how many
/// fit in one slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stack {
    pub key: String,
    pub count: i32,
    pub max: i32,
}

/// How many slots a block of this name holds as mounted storage, or none if it isn't one
/// (`create:chest_mounted_storage`, `create:simple_mounted_storage`, `ItemVaultMountedStorageType`).
pub fn item_slots(block: &str) -> Option<usize> {
    let path = block.strip_prefix("minecraft:")?;
    if matches!(path, "chest" | "trapped_chest" | "barrel") || path == "shulker_box" || path.ends_with("_shulker_box")
    {
        return Some(27);
    }
    None
}

/// `CLogistics.vaultCapacity`: the slots of one Item Vault block.
pub const VAULT_SLOTS: usize = 20;

/// The slots a block of this name contributes to the combined inventory.
pub fn slots_of_block(block: &str) -> Option<usize> {
    if block == "create:item_vault" { Some(VAULT_SLOTS) } else { item_slots(block) }
}

/// One slot of a structure's combined inventory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slot {
    /// The block of the structure it belongs to, and its index in that block's `Items`.
    pub block: Pos,
    pub index: usize,
    pub stack: Option<Stack>,
}

/// The structure's inventories as one (`MountedStorageManager.getAllItems`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inventory {
    pub slots: Vec<Slot>,
}

/// An ItemStackHandler's per-slot limit.
pub const SLOT_LIMIT: i32 = 64;

impl Inventory {
    fn limit(stack: &Stack) -> i32 {
        stack.max.min(SLOT_LIMIT)
    }

    /// `ItemStackHandler.insertItem` into one slot: how many of `stack` stay out.
    fn insert_slot(&mut self, at: usize, stack: &Stack, left: i32, simulate: bool) -> i32 {
        let slot = &mut self.slots[at];
        match &mut slot.stack {
            Some(have) => {
                if have.key != stack.key {
                    return left;
                }
                let room = (Self::limit(have) - have.count).max(0);
                let moved = room.min(left);
                if !simulate {
                    have.count += moved;
                }
                left - moved
            }
            None => {
                let moved = Self::limit(stack).min(left);
                if !simulate && moved > 0 {
                    slot.stack = Some(Stack { key: stack.key.clone(), count: moved, max: stack.max });
                }
                left - moved
            }
        }
    }

    /// `ItemHandlerHelper.insertItemStacked`: into the slots that hold the item first, then the
    /// empty ones (a stack that doesn't stack goes only into empty ones). How many are left over.
    pub fn insert_stacked(&mut self, stack: &Stack, simulate: bool) -> i32 {
        let mut left = stack.count;
        if left <= 0 {
            return 0;
        }
        if stack.max > 1 {
            for i in 0..self.slots.len() {
                if self.slots[i].stack.as_ref().is_some_and(|have| have.key == stack.key) {
                    left = self.insert_slot(i, stack, left, simulate);
                    if left <= 0 {
                        return 0;
                    }
                }
            }
        }
        for i in 0..self.slots.len() {
            if self.slots[i].stack.is_none() {
                left = self.insert_slot(i, stack, left, simulate);
                if left <= 0 {
                    return 0;
                }
            }
        }
        left
    }

    /// `ItemStackHandler.extractItem`: up to `amount` of the slot's stack.
    pub fn extract(&mut self, at: usize, amount: i32, simulate: bool) -> Option<Stack> {
        let slot = self.slots.get_mut(at)?;
        let have = slot.stack.as_mut()?;
        let taken = amount.min(have.count);
        if taken <= 0 {
            return None;
        }
        let out = Stack { key: have.key.clone(), count: taken, max: have.max };
        if !simulate {
            have.count -= taken;
            if have.count <= 0 {
                slot.stack = None;
            }
        }
        Some(out)
    }

    /// The slots that hold something, in order: for the funnels' and chutes' own extraction.
    pub fn filled(&self) -> impl Iterator<Item = (usize, &Stack)> + '_ {
        self.slots.iter().enumerate().filter_map(|(i, s)| s.stack.as_ref().map(|stack| (i, stack)))
    }
}

/// A tank of the structure's fluid storage: a Fluid Tank's controller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageTank {
    pub block: Pos,
    pub capacity: i32,
    pub fluid: Option<String>,
    pub amount: i32,
}

impl StorageTank {
    fn fill(&mut self, fluid: &str, amount: i32, simulate: bool) -> i32 {
        if amount <= 0 || self.fluid.as_deref().is_some_and(|f| f != fluid) && self.amount > 0 {
            return 0;
        }
        let have = if self.fluid.is_some() { self.amount } else { 0 };
        let moved = amount.min((self.capacity - have).max(0));
        if !simulate && moved > 0 {
            self.fluid = Some(fluid.to_owned());
            self.amount = have + moved;
        }
        moved
    }

    fn drain(&mut self, fluid: Option<&str>, max: i32, simulate: bool) -> Option<(String, i32)> {
        let held = self.fluid.clone()?;
        if self.amount <= 0 || fluid.is_some_and(|f| f != held) {
            return None;
        }
        let taken = max.min(self.amount);
        if taken <= 0 {
            return None;
        }
        if !simulate {
            self.amount -= taken;
            if self.amount <= 0 {
                self.fluid = None;
                self.amount = 0;
            }
        }
        Some((held, taken))
    }
}

/// The structure's tanks as one (`MountedFluidStorageWrapper`, a `CombinedTankWrapper`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tanks {
    pub tanks: Vec<StorageTank>,
}

impl Tanks {
    /// `CombinedTankWrapper.fill`: once a tank holds the fluid, filling starts there and goes on
    /// through the ones after it; with none holding it, through all in order. How much went in.
    pub fn fill(&mut self, fluid: &str, amount: i32, simulate: bool) -> i32 {
        let mut left = amount;
        let mut filled = 0;
        if left <= 0 {
            return 0;
        }
        let mut fitting_found = false;
        'outer: for search_pass in [true, false] {
            for i in 0..self.tanks.len() {
                if search_pass && self.tanks[i].fluid.as_deref() == Some(fluid) && self.tanks[i].amount > 0 {
                    fitting_found = true;
                }
                if search_pass && !fitting_found {
                    continue;
                }
                let moved = self.tanks[i].fill(fluid, left, simulate);
                left -= moved;
                filled += moved;
                if left <= 0 {
                    break 'outer;
                }
                if fitting_found && moved != 0 {
                    break 'outer;
                }
            }
        }
        filled
    }

    /// `CombinedTankWrapper.drain`: from each tank in turn (only of `fluid` if one is named), the
    /// first fluid found. The fluid and how much.
    pub fn drain(&mut self, fluid: Option<&str>, max: i32, simulate: bool) -> Option<(String, i32)> {
        let mut left = max;
        let mut out: Option<(String, i32)> = None;
        for tank in &mut self.tanks {
            let want = match (&out, fluid) {
                (Some((got, _)), _) => Some(got.clone()),
                (None, f) => f.map(str::to_owned),
            };
            if let Some((got, amount)) = tank.drain(want.as_deref(), left, simulate) {
                left -= amount;
                match &mut out {
                    Some((_, total)) => *total += amount,
                    None => out = Some((got, amount)),
                }
            }
            if left <= 0 {
                break;
            }
        }
        out
    }

    /// `getFluidInTank` over all the tanks, for the view: the first fluid and what all the tanks
    /// of that fluid hold, and the room of every tank that takes it.
    pub fn view(&self) -> Vec<(Option<(String, i32)>, i32)> {
        self.tanks
            .iter()
            .map(|t| (t.fluid.clone().filter(|_| t.amount > 0).map(|f| (f, t.amount)), t.capacity))
            .collect()
    }
}

/// The turned position of a structure's block in the world at a quarter turn: for tests of the
/// bearing's sense.
pub fn quarter_turned(local: Pos, axis: Axis, quarters: i32) -> Pos {
    rotate_pos(local, axis, quarters)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(key: &str, count: i32, max: i32) -> Stack {
        Stack { key: key.to_owned(), count, max }
    }

    fn inventory(slots: usize) -> Inventory {
        Inventory {
            slots: (0..slots).map(|i| Slot { block: [0, 0, 0], index: i, stack: None }).collect(),
        }
    }

    #[test]
    fn a_new_link_connects_after_the_animation_and_waits_for_the_timeout_after_a_transfer() {
        let mut link = Link::default();
        assert!(link.start([1, 2, 3], 5.0));
        assert_eq!(link.distance, MAX_DISTANCE);
        assert_eq!(link.timer, 14);
        // Linking again with the same structure does nothing.
        assert!(!link.start([1, 2, 3], 1.0));
        assert_eq!(link.distance, MAX_DISTANCE);
        // The structure holds it (`keepAlive` each tick); the timer runs down to the animation
        // and stays there.
        for _ in 0..30 {
            link.keep_alive = KEEP_ALIVE;
            link.tick(TIMEOUT);
        }
        assert_eq!(link.timer, ANIMATION);
        assert!(link.can_transfer(TIMEOUT));
        // Something moves: a whole timeout again, then down to the animation.
        link.content_transferred(TIMEOUT);
        assert_eq!(link.timer, 64);
        for _ in 0..60 {
            link.keep_alive = KEEP_ALIVE;
            link.tick(TIMEOUT);
        }
        assert_eq!(link.timer, ANIMATION);
    }

    #[test]
    fn a_link_nobody_holds_lets_go_and_retracts() {
        let mut link = Link::default();
        link.start([0, 0, 0], 2.0);
        link.keep_alive = 2;
        // Two ticks with nobody refreshing the hold.
        let first = link.tick(TIMEOUT);
        assert!(!first.stopped);
        let second = link.tick(TIMEOUT);
        assert!(second.stopped && second.send);
        assert!(link.controller.is_none());
        assert_eq!(link.timer, ANIMATION - 1);
        // Now it runs down to nothing.
        for _ in 0..3 {
            link.tick(TIMEOUT);
        }
        assert_eq!(link.timer, 0);
    }

    #[test]
    fn a_link_with_nothing_to_hold_it_ends_when_its_timer_does_and_a_powered_one_lets_go() {
        let mut link = Link::default();
        link.start([0, 0, 0], 2.0);
        for _ in 0..20 {
            link.tick(TIMEOUT);
        }
        assert!(link.controller.is_none());
        assert_eq!(link.timer, 0);
        let mut link = Link::default();
        link.start([0, 0, 0], 2.0);
        assert_eq!(link.power(true), Some(false));
        assert!(link.controller.is_none());
        assert_eq!(link.power(true), None);
        assert_eq!(link.power(false), Some(true));
    }

    #[test]
    fn the_connected_window_is_the_animation_to_the_timeout() {
        let mut link = Link::default();
        link.timer = 3;
        assert!(!link.is_connected(TIMEOUT));
        link.timer = 4;
        assert!(link.is_connected(TIMEOUT));
        link.timer = 64;
        assert!(link.is_connected(TIMEOUT));
        link.timer = 65;
        assert!(!link.is_connected(TIMEOUT));
        assert!(link.is_transferring());
        assert_eq!(analog_output(true), 15);
        assert_eq!(analog_output(false), 0);
    }

    #[test]
    fn a_client_follows_the_timer_from_a_snapshot() {
        assert_eq!(timer_at(14, 100, true, 105), 9);
        assert_eq!(timer_at(14, 100, true, 200), 4);
        assert_eq!(timer_at(3, 100, true, 200), 3);
        assert_eq!(timer_at(3, 100, false, 102), 1);
        assert_eq!(timer_at(3, 100, false, 200), 0);
        assert_eq!(timer_at(64, 10, true, 10), 64);
    }

    #[test]
    fn the_interface_comes_out_as_it_connects_and_goes_back_as_it_lets_go() {
        // Connected from the first tick (a new link's timer of 14 is inside the window), as in Create.
        assert_eq!(progress(14, TIMEOUT), 1.0);
        assert_eq!(progress(4, TIMEOUT), 1.0);
        // Letting go: below the animation it goes back in.
        assert_eq!(progress(3, TIMEOUT), 0.75);
        assert_eq!(progress(0, TIMEOUT), 0.0);
        // A transfer's timer of 64 is the connected window's top; above it (a long timeout's
        // start) the interface is drawn going in.
        assert_eq!(progress(64, TIMEOUT), 1.0);
        assert_eq!(progress(65, TIMEOUT), 0.75);
        assert_eq!(progress(68, TIMEOUT), 0.0);
        assert!((extension(1.0, 2.0) - 1.0).abs() < 1e-6);
        assert!((extension(0.5, 2.0) - 0.25).abs() < 1e-6);
        let (middle, top) = middle_and_top(1.0);
        assert!((middle - 0.875).abs() < 1e-6 && (top - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_placed_interface_faces_the_player_and_a_sneaking_one_faces_away() {
        // Looking down at a floor: it faces up.
        assert_eq!(placement_facing(Dir::Down, false), Dir::Up);
        assert_eq!(placement_facing(Dir::Down, true), Dir::Down);
        assert_eq!(placement_facing(Dir::North, false), Dir::South);
    }

    #[test]
    fn the_nearest_direction_takes_the_largest_component_and_a_turned_interface_needs_a_right_angle() {
        assert_eq!(nearest_direction([0.1, 0.9, 0.2]), Dir::Up);
        assert_eq!(nearest_direction([-0.9, 0.1, 0.0]), Dir::West);
        assert_eq!(current_facing([0.0, 1.0, 0.0]), Some(Dir::Up));
        // A 20 degrees turn is still close enough; 45 is not.
        let near = rotate_vec([0.0, 1.0, 0.0], Axis::X, 20.0);
        assert_eq!(current_facing(near), Some(Dir::Up));
        let far = rotate_vec([0.0, 1.0, 0.0], Axis::X, 45.0);
        assert_eq!(current_facing(far), None);
    }

    #[test]
    fn a_vector_turns_the_way_the_structure_turns_at_right_angles() {
        for axis in Axis::ALL {
            for local in [[1, 0, 0], [0, 1, 0], [0, 0, 1], [2, -1, 3]] {
                for quarters in 0..4 {
                    let turned = rotate_vec(
                        [f64::from(local[0]), f64::from(local[1]), f64::from(local[2])],
                        axis,
                        f64::from(90 * quarters),
                    );
                    let by_quarters = quarter_turned(local, axis, quarters);
                    for i in 0..3 {
                        assert!(
                            (turned[i] - f64::from(by_quarters[i])).abs() < 1e-9,
                            "{axis:?} {local:?} {quarters}: {turned:?} vs {by_quarters:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_moving_interface_finds_a_stationary_one_facing_it_in_its_cell_or_the_next() {
        let at = |pos: Pos| (pos == [5, 0, 0]).then_some(Stationary { facing: Dir::West, powered: false });
        // The moving one faces east: it looks at its cell and the one east of it.
        assert_eq!(find_interface([4, 0, 0], Dir::East, &at), Some([5, 0, 0]));
        assert_eq!(find_interface([5, 0, 0], Dir::East, &at), Some([5, 0, 0]));
        assert_eq!(find_interface([3, 0, 0], Dir::East, &at), None);
        // Facing the same way as the stationary one, or powered, it doesn't link.
        let same = |pos: Pos| (pos == [5, 0, 0]).then_some(Stationary { facing: Dir::East, powered: false });
        assert_eq!(find_interface([4, 0, 0], Dir::East, &same), None);
        let powered = |pos: Pos| (pos == [5, 0, 0]).then_some(Stationary { facing: Dir::West, powered: true });
        assert_eq!(find_interface([4, 0, 0], Dir::East, &powered), None);
    }

    #[test]
    fn the_link_distance_is_the_gap_along_the_facing_plus_the_reach_less_a_block() {
        // The interface's centre 1.5 along the facing from the moving one's: 1.5 + 0.85.
        let distance = link_distance([5, 0, 0], [4.0, 0.5, 0.5], Dir::East);
        assert!((distance - (1.5 + 0.85)).abs() < 1e-5);
        // Sideways offsets don't count.
        let sideways = link_distance([5, 0, 0], [4.0, 0.5, 3.5], Dir::East);
        assert!((sideways - distance).abs() < 1e-5);
    }

    #[test]
    fn a_structure_stalls_at_its_closest_and_goes_on_when_the_timer_is_down() {
        let target = centre([5, 0, 0]);
        let mut actor = Actor::default();
        actor.visit(Some([5, 0, 0]));
        // Still approaching: no stall.
        actor.arrive([3.5, 0.5, 0.5], [0.5, 0.0, 0.0], target);
        assert!(!actor.stall);
        // At the target now, the next step would move away: stall.
        actor.arrive([5.5, 0.5, 0.5], [0.5, 0.0, 0.0], target);
        assert!(actor.stall);
        // A fresh link's timer is above the animation: it stays stalled.
        actor.release(14);
        assert!(actor.stall);
        actor.release(4);
        assert!(!actor.stall);
        // A stalled structure has no motion, so it doesn't stall again.
        actor.arrive([5.5, 0.5, 0.5], [0.0, 0.0, 0.0], target);
        assert!(!actor.stall);
        actor.reset();
        assert_eq!(actor, Actor::default());
    }

    #[test]
    fn a_held_structure_follows_no_speed() {
        assert_eq!(moving_speed(true, 64.0), 0.0);
        assert_eq!(moving_speed(false, 64.0), 64.0);
        assert_eq!(moving_speed(true, -64.0), 0.0);
    }

    #[test]
    fn mounted_storage_is_chests_barrels_shulker_boxes_and_vaults() {
        assert_eq!(slots_of_block("minecraft:chest"), Some(27));
        assert_eq!(slots_of_block("minecraft:trapped_chest"), Some(27));
        assert_eq!(slots_of_block("minecraft:barrel"), Some(27));
        assert_eq!(slots_of_block("minecraft:red_shulker_box"), Some(27));
        assert_eq!(slots_of_block("minecraft:shulker_box"), Some(27));
        assert_eq!(slots_of_block("create:item_vault"), Some(20));
        assert_eq!(slots_of_block("minecraft:ender_chest"), None);
        assert_eq!(slots_of_block("minecraft:hopper"), None);
        assert_eq!(slots_of_block("create:creative_crate"), None);
    }

    #[test]
    fn an_insert_tops_up_equal_stacks_first_then_fills_empty_slots() {
        let mut inventory = inventory(4);
        inventory.slots[2].stack = Some(stack("a", 60, 64));
        assert_eq!(inventory.insert_stacked(&stack("a", 10, 64), false), 0);
        assert_eq!(inventory.slots[2].stack.as_ref().map(|s| s.count), Some(64));
        assert_eq!(inventory.slots[0].stack.as_ref().map(|s| s.count), Some(6));
        // A different item goes to an empty slot, and what can't fit is returned.
        assert_eq!(inventory.insert_stacked(&stack("b", 300, 64), false), 300 - 64 * 2);
        // Full now: a simulation says so and changes nothing.
        let before = inventory.clone();
        assert_eq!(inventory.insert_stacked(&stack("c", 5, 64), true), 5);
        assert_eq!(inventory, before);
        // With room it says that too.
        inventory.slots[3].stack = None;
        assert_eq!(inventory.insert_stacked(&stack("c", 5, 64), true), 0);
        assert!(inventory.slots[3].stack.is_none());
    }

    #[test]
    fn an_item_that_doesnt_stack_goes_one_to_a_slot() {
        let mut inventory = inventory(3);
        assert_eq!(inventory.insert_stacked(&stack("sword", 1, 1), false), 0);
        assert_eq!(inventory.insert_stacked(&stack("sword", 1, 1), false), 0);
        assert_eq!(inventory.insert_stacked(&stack("sword", 1, 1), false), 0);
        assert_eq!(inventory.insert_stacked(&stack("sword", 1, 1), false), 1);
        // A slot's limit is 64 whatever the item allows.
        let mut big = inventory_of_one();
        assert_eq!(big.insert_stacked(&stack("plate", 200, 99), false), 200 - 64);
    }

    fn inventory_of_one() -> Inventory {
        inventory(1)
    }

    #[test]
    fn extraction_takes_what_is_there_and_empties_the_slot() {
        let mut inventory = inventory(2);
        inventory.slots[1].stack = Some(stack("a", 10, 64));
        assert_eq!(inventory.extract(0, 5, false), None);
        assert_eq!(inventory.extract(1, 4, true), Some(stack("a", 4, 64)));
        assert_eq!(inventory.slots[1].stack.as_ref().map(|s| s.count), Some(10));
        assert_eq!(inventory.extract(1, 64, false), Some(stack("a", 10, 64)));
        assert!(inventory.slots[1].stack.is_none());
        assert_eq!(inventory.filled().count(), 0);
    }

    fn tank(block: Pos, capacity: i32, fluid: Option<&str>, amount: i32) -> StorageTank {
        StorageTank { block, capacity, fluid: fluid.map(str::to_owned), amount }
    }

    #[test]
    fn fluid_fills_the_tank_already_holding_it_and_then_the_ones_after() {
        let mut tanks = Tanks {
            tanks: vec![
                tank([0, 0, 0], 8000, None, 0),
                tank([1, 0, 0], 8000, Some("minecraft:water"), 7000),
                tank([2, 0, 0], 8000, None, 0),
            ],
        };
        // The second holds water, so filling starts there; once a tank that holds it took some
        // the fill ends (`fittingHandlerFound && filledIntoCurrent != 0`).
        assert_eq!(tanks.fill("minecraft:water", 3000, false), 1000);
        assert_eq!(tanks.tanks[0].amount, 0);
        assert_eq!(tanks.tanks[1].amount, 8000);
        assert_eq!(tanks.tanks[2].amount, 0);
        // Full, it takes nothing and the fill goes on to the next.
        assert_eq!(tanks.fill("minecraft:water", 2000, false), 2000);
        assert_eq!(tanks.tanks[2].amount, 2000);
        // Another fluid has no holder: the first tank that takes it.
        assert_eq!(tanks.fill("minecraft:lava", 500, false), 500);
        assert_eq!(tanks.tanks[0].fluid.as_deref(), Some("minecraft:lava"));
        // A simulation changes nothing.
        let before = tanks.clone();
        assert_eq!(tanks.fill("minecraft:water", 100, true), 100);
        assert_eq!(tanks, before);
    }

    #[test]
    fn fluid_drains_from_the_tanks_in_order_one_fluid_only() {
        let mut tanks = Tanks {
            tanks: vec![
                tank([0, 0, 0], 8000, Some("minecraft:lava"), 300),
                tank([1, 0, 0], 8000, Some("minecraft:water"), 1000),
                tank([2, 0, 0], 8000, Some("minecraft:lava"), 400),
            ],
        };
        // The first fluid found, from every tank holding it.
        assert_eq!(tanks.drain(None, 500, false), Some(("minecraft:lava".to_owned(), 500)));
        assert_eq!(tanks.tanks[0].fluid, None);
        assert_eq!(tanks.tanks[2].amount, 200);
        // A named fluid skips the others.
        assert_eq!(tanks.drain(Some("minecraft:water"), 400, false), Some(("minecraft:water".to_owned(), 400)));
        assert_eq!(tanks.drain(Some("minecraft:honey"), 400, true), None);
        assert_eq!(tanks.view().len(), 3);
    }
}
