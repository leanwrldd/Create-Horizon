//! The Mechanical Crafter's geometry and how crafters join into grids, from Create `mc1.21.1/dev`
//! at 4148fd8 (MIT, the Creators of Create): `MechanicalCrafterBlock` (`POINTING`,
//! `getTargetDirection`, `pointingFromFacing`, `isValidTarget`), `ConnectedInputHandler`
//! (`shouldConnect`, `toggleConnection`, `ConnectedInput.getInventories`) and
//! `EdgeInteractionHandler` (`getBB`). No game types: the glue reads the world and asks.
//!
//! The grid's items, the chain of crafters passing them on and the craft itself are
//! `crafter_grid.rs`; the recipes' matching is `crafter_recipes.rs`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::{Axis, Dir, Pos};

/// `CStress.setImpact(2.0)` for `AllBlocks.MECHANICAL_CRAFTER`: su per RPM.
pub const IMPACT: f32 = 2.0;

/// Catnip's `Pointing`: which way a crafter passes its items on, seen from in front of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Pointing {
    Up,
    Left,
    Down,
    Right,
}

impl Pointing {
    /// The enum's order, which `BlockState.cycle` steps through.
    pub const ALL: [Pointing; 4] = [Pointing::Up, Pointing::Left, Pointing::Down, Pointing::Right];

    /// `getSerializedName`, the state's `pointing` value.
    pub const fn name(self) -> &'static str {
        match self {
            Pointing::Up => "up",
            Pointing::Left => "left",
            Pointing::Down => "down",
            Pointing::Right => "right",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == name)
    }

    /// `getXRotation`.
    pub const fn x_rotation(self) -> i32 {
        match self {
            Pointing::Up => 0,
            Pointing::Left => 270,
            Pointing::Down => 180,
            Pointing::Right => 90,
        }
    }

    /// `state.cycle(POINTING)`: the next value in the enum's order, wrapping.
    pub fn cycle(self) -> Self {
        let i = Self::ALL.iter().position(|p| *p == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// `GroupedItems.mergeOnto`'s shift: where a crafter's cells land in its target's grid.
    pub const fn merge_offset(self) -> (i32, i32) {
        match self {
            Pointing::Left => (1, 0),
            Pointing::Right => (-1, 0),
            Pointing::Down => (0, 1),
            Pointing::Up => (0, -1),
        }
    }
}

/// Whether `facing` is a direction a crafter can face (`HORIZONTAL_FACING`).
pub const fn horizontal(facing: Dir) -> bool {
    !matches!(facing, Dir::Up | Dir::Down)
}

/// `AngleHelper.horizontalAngle`: south 0, west -90, north 180, east -270 (an x-axis direction
/// has its `toYRot` negated).
pub const fn horizontal_angle(facing: Dir) -> i32 {
    match facing {
        Dir::South => 0,
        Dir::West => -90,
        Dir::North => 180,
        Dir::East => -270,
        Dir::Up | Dir::Down => 0,
    }
}

/// `MechanicalCrafterBlock.getTargetDirection`: up, turned by `-xRotation` about z
/// (`VecHelper.rotate`), then by `horizontalAngle(facing)` about y, and snapped to a direction.
pub fn target_direction(facing: Dir, pointing: Pointing) -> Dir {
    // `VecHelper.rotate` by multiples of 90 degrees, exactly.
    let sin_cos = |deg: i32| match deg.rem_euclid(360) {
        0 => (0, 1),
        90 => (1, 0),
        180 => (0, -1),
        _ => (-1, 0),
    };
    // About z: x' = x cos - y sin, y' = y cos + x sin, from (0, 1, 0).
    let (s, c) = sin_cos(-pointing.x_rotation());
    let (x, y, z) = (-s, c, 0);
    // About y: x' = x cos + z sin, z' = z cos - x sin.
    let (s, c) = sin_cos(horizontal_angle(facing));
    let (x, z) = (x * c + z * s, z * c - x * s);
    if y > 0 {
        Dir::Up
    } else if y < 0 {
        Dir::Down
    } else if x > 0 {
        Dir::East
    } else if x < 0 {
        Dir::West
    } else if z > 0 {
        Dir::South
    } else {
        Dir::North
    }
}

/// `MechanicalCrafterBlock.pointingFromFacing`: the pointing that makes a crafter facing
/// `block_facing` pass its items towards `pointing_face`.
pub fn pointing_from_facing(pointing_face: Dir, block_facing: Dir) -> Pointing {
    let positive = block_facing.sign() > 0;
    let mut pointing = if pointing_face == Dir::Down { Pointing::Up } else { Pointing::Down };
    let pick = |positive_left: bool| match (positive, positive_left) {
        (true, true) | (false, false) => Pointing::Left,
        _ => Pointing::Right,
    };
    match pointing_face {
        Dir::East | Dir::North => pointing = pick(true),
        Dir::West | Dir::South => pointing = pick(false),
        _ => {}
    }
    pointing
}

/// What `getStateForPlacement` makes of a crafter: `facing` towards the player, or, placed on a
/// face of another crafter without sneaking, that crafter's facing; and a pointing. `clicked` is
/// the face clicked, `towards_player` the horizontal direction the player faces, opposite, and
/// `behind` the facing of the crafter the new one is placed against, if there is one.
pub fn placement(
    clicked: Dir,
    towards_player: Dir,
    sneaking: bool,
    behind: Option<Dir>,
) -> (Dir, Pointing) {
    match behind {
        Some(other) if !sneaking => (other, pointing_from_facing(clicked, other)),
        _ => {
            let pointing = if towards_player != clicked {
                pointing_from_facing(clicked, towards_player)
            } else {
                Pointing::Up
            };
            (towards_player, pointing)
        }
    }
}

/// `MechanicalCrafterBlock.isValidTarget`: whether the block a crafter points at is a crafter
/// of the same facing that doesn't point straight back (the belt is drawn between them, else an
/// arrow). `target` is its `(facing, pointing)`.
pub fn is_valid_target(facing: Dir, pointing: Pointing, target: Option<(Dir, Pointing)>) -> bool {
    match target {
        Some((other_facing, other_pointing)) => {
            other_facing == facing
                && (pointing.x_rotation() - other_pointing.x_rotation()).abs() != 180
        }
        None => false,
    }
}

/// `getCountDownSpeed`: the speed's size held between 4 and 250, or 0 when still.
pub fn count_down_speed(rpm: f32) -> i32 {
    if rpm == 0.0 { 0 } else { (rpm.abs() as i32).clamp(4, 250) }
}

// --- the picture (`MechanicalCrafterRenderer`) --------------------------------------------------------------

/// The animation's `countDown` a client works out between updates: the value `anchor` had
/// when the server last said so, run down at the crafter's count-down speed for `elapsed` ticks
/// (a fraction counts, as `+ getCountDownSpeed() * partialTicks` does).
pub fn estimated_count_down(anchor: i32, speed: i32, elapsed: f32) -> f32 {
    (anchor as f32 - speed as f32 * elapsed.max(0.0)).max(0.0)
}

/// How far an exporting crafter has slid its items towards the target, 0 to 1
/// (`(1000 - countDown + ...) / 1000`).
pub fn export_progress(count_down: f32) -> f32 {
    ((1000.0 - count_down) / 1000.0).clamp(0.0, 1.0)
}

/// How far the first half of a craft has got (the items drawn together), 0 to 1
/// (`(2000 - countDown + ...) / 1000`): 0 to 1 over the first 1000 ticks of the countdown
/// from 2000.
pub fn gather_progress(count_down: f32) -> f32 {
    ((2000.0 - count_down) / 1000.0).clamp(0.0, 1.0)
}

/// `earlyProgress` and `lateProgress` of a progress.
pub fn early_late(progress: f32) -> (f32, f32) {
    ((progress * 2.0).clamp(0.0, 1.0), (progress * 2.0 - 1.0).clamp(0.0, 1.0))
}

/// The spacing between the items of the grid while crafting: 0.5, a little more mid-way.
pub fn crafting_spacing(progress: f32) -> f32 {
    0.5 + (-4.0 * (progress - 0.5) * (progress - 0.5) + 1.0) * 0.25
}

/// `-minX + (-width + 1) / 2`, `-minY + (-height + 1) / 2`, scaled by `early`: where the grid
/// shifts to be centred, in half-blocks.
pub fn centering(min_x: i32, min_y: i32, width: i32, height: i32, early: f32) -> (f32, f32) {
    (
        (-(min_x as f32) + (-(width as f32) + 1.0) / 2.0) * early,
        (-(min_y as f32) + (-(height as f32) + 1.0) / 2.0) * early,
    )
}

/// The layer offset of an exporting crafter's items, so they don't fight the crafter ahead
/// (`value == UP ? -1 : LEFT ? 2 : RIGHT ? -2 : 1`).
pub fn export_layer(pointing: Pointing) -> i32 {
    match pointing {
        Pointing::Up => -1,
        Pointing::Left => 2,
        Pointing::Right => -2,
        Pointing::Down => 1,
    }
}

/// `(int) (getCountDownSpeed() / 128f * ticks)`: the frame of the belt's animation.
pub fn belt_frame(speed: i32, ticks: i64) -> i32 {
    (speed as f32 / 128.0 * ticks as f32) as i32
}

// --- joining crafters (`ConnectedInputHandler`) ---------------------------------------------------

fn add(a: Pos, b: Pos) -> Pos {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: Pos, b: Pos) -> Pos {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// `ConnectedInputHandler.shouldConnect`: from the crafter facing `facing`, clicked on `face`,
/// whether the neighbour towards `direction` may join it: not along the crafter's facing axis,
/// not from its front, and only a crafter facing the same way.
pub fn should_connect(facing: Dir, face: Dir, direction: Dir, neighbour: Option<Dir>) -> bool {
    direction.axis() != facing.axis() && face != facing && neighbour == Some(facing)
}

/// One crafter's `ConnectedInput`: a controller holds the offsets of every crafter of its group
/// (its own, `[0, 0, 0]`, first); the others hold only the offset to their controller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Input {
    pub controller: bool,
    pub data: Vec<Pos>,
}

impl Default for Input {
    /// `new ConnectedInput()`: alone, its own controller.
    fn default() -> Self {
        Input { controller: true, data: vec![[0; 3]] }
    }
}

impl Input {
    /// `attachTo`.
    fn attach_to(&mut self, controller: Pos, me: Pos) {
        self.controller = false;
        self.data = vec![sub(controller, me)];
    }

    /// `read`: "nbt got wiped -> reset".
    pub fn or_reset(self) -> Self {
        if self.data.is_empty() { Input::default() } else { self }
    }

    /// Where the controller of a group the crafter at `me` belongs to is
    /// (`pos.offset(data.get(0))`).
    pub fn controller_of(&self, me: Pos) -> Pos {
        add(me, self.data.first().copied().unwrap_or([0; 3]))
    }
}

/// The inputs of the crafters a joining touches, by position. A position not in it is no crafter
/// (`CrafterHelper.getCrafter` is null).
pub type Inputs = BTreeMap<Pos, Input>;

/// Every position a join of the crafters at `a` and `b` reads or changes: both, their
/// controllers and everything their controllers lead. The glue reads these crafters' inputs
/// first. `read` is the offsets a crafter at a position keeps (`None`: no crafter there).
pub fn touched(read: &dyn Fn(Pos) -> Option<Input>, a: Pos, b: Pos) -> Vec<Pos> {
    let mut out = BTreeSet::from([a, b]);
    for start in [a, b] {
        let Some(input) = read(start) else { continue };
        let controller = input.controller_of(start);
        out.insert(controller);
        if let Some(lead) = read(controller) {
            out.extend(lead.data.iter().map(|o| add(controller, *o)));
        }
    }
    out.into_iter().collect()
}

/// `toggleConnection(world, pos, pos2)`: in one group, the crafters part there; in two, the
/// second group joins the first. Does nothing unless both are crafters (in `inputs`).
pub fn toggle(inputs: &mut Inputs, pos: Pos, pos2: Pos) {
    let (Some(first), Some(second)) = (inputs.get(&pos).cloned(), inputs.get(&pos2).cloned()) else {
        return;
    };
    let controller1 = first.controller_of(pos);
    let controller2 = second.controller_of(pos2);

    if controller1 == controller2 {
        let Some(controller) = inputs.get(&controller1).cloned() else {
            return;
        };
        let mut positions: BTreeSet<Pos> =
            controller.data.iter().map(|o| add(controller1, *o)).collect();
        let mut frontier = VecDeque::from([pos2]);
        let mut split_group = Vec::new();
        positions.remove(&pos2);
        positions.remove(&pos);
        while let Some(current) = frontier.pop_front() {
            for direction in Dir::ALL {
                let next = add(current, direction.normal());
                if !positions.remove(&next) {
                    continue;
                }
                split_group.push(next);
                frontier.push_back(next);
            }
        }
        let rest: Vec<Pos> = positions.into_iter().collect();
        init_and_add_all(inputs, pos, &rest);
        init_and_add_all(inputs, pos2, &split_group);
        return;
    }

    // `crafter1.input.isController ? crafter1 : controller crafter`.
    let lead1 = if first.controller { pos } else { controller1 };
    let lead2 = if second.controller { pos2 } else { controller2 };
    if !inputs.contains_key(&lead1) || !inputs.contains_key(&lead2) {
        return;
    }
    connect_controllers(inputs, lead1, lead2);
}

/// `initAndAddAll`: `crafter` becomes the controller of a new group of itself and `positions`.
fn init_and_add_all(inputs: &mut Inputs, crafter: Pos, positions: &[Pos]) {
    let mut own = Input::default();
    for &p in positions {
        if let Some(input) = inputs.get_mut(&p) {
            input.attach_to(crafter, p);
            own.data.push(sub(p, crafter));
        }
    }
    inputs.insert(crafter, own);
}

/// `connectControllers`: everything the second controller leads, then the controller itself,
/// goes under the first.
fn connect_controllers(inputs: &mut Inputs, crafter1: Pos, crafter2: Pos) {
    let offsets = inputs.get(&crafter2).map(|i| i.data.clone()).unwrap_or_default();
    for offset in offsets {
        if offset == [0; 3] {
            continue;
        }
        let connected = add(crafter2, offset);
        let Some(input) = inputs.get_mut(&connected) else { continue };
        // `BlockPos.ZERO.subtract(input.data.get(0))`: where this crafter was, from crafter 2.
        let new = sub(connected, crafter1);
        input.attach_to(crafter1, connected);
        if let Some(lead) = inputs.get_mut(&crafter1) {
            lead.data.push(new);
        }
    }
    if let Some(input) = inputs.get_mut(&crafter2) {
        input.attach_to(crafter1, crafter2);
    }
    if let Some(lead) = inputs.get_mut(&crafter1) {
        lead.data.push(sub(crafter2, crafter1));
    }
}

/// `MechanicalCrafterBlock.onRemove`: whether two crafters beside each other belong to one
/// group (`pos + data[0]` agree), so that breaking one parts the other from it. Each is the
/// position and the input of a crafter.
pub fn same_group(a: (Pos, &Input), b: (Pos, &Input)) -> bool {
    a.1.controller_of(a.0) == b.1.controller_of(b.0)
}

/// `ConnectedInput.getInventories`: the crafters of the group a controller at `at` leads, in
/// the order their slots make one inventory: top row first, along the row as a player in front
/// of the crafters reads it. `positions` are the group's crafters (absolute); only horizontal
/// facings are valid.
pub fn inventory_order(facing: Dir, mut positions: Vec<Pos>) -> Vec<Pos> {
    let clockwise = match facing {
        Dir::North => Dir::East,
        Dir::East => Dir::South,
        Dir::South => Dir::West,
        Dir::West => Dir::North,
        other => other,
    };
    let compare = clockwise.axis();
    let modifier = facing.sign() * if compare == Axis::Z { -1 } else { 1 };
    let along = |p: &Pos| p[compare.index()];
    positions.sort_by(|p1, p2| {
        // `-Integer.compare(y1, y2)` first, then `modifier * Integer.compare(c1, c2)`.
        p2[1].cmp(&p1[1]).then_with(|| {
            let order = along(p1).cmp(&along(p2));
            if modifier < 0 { order.reverse() } else { order }
        })
    });
    positions
}

// --- a click on the front (`MechanicalCrafterBlock.useItemOn`) --------------------------------------

/// What a right click on a crafter does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Use {
    /// `PASS_TO_DEFAULT_BLOCK_INTERACTION`: the click goes on (a block may be placed).
    Pass,
    /// `ejectWholeGrid`: a click on the front of a working crafter throws everything out.
    EjectGrid,
    /// The Crafter Slot Cover goes on the slot.
    PlaceCover,
    /// The held stack goes into the grid's inventory, one item to a crafter.
    Insert,
    /// The empty hand takes the cover off.
    Uncover,
    /// The empty hand (or the same item) takes the item out of the slot.
    TakeItem,
}

/// What a hand brings to a click on a crafter.
#[derive(Clone, Copy, Debug, Default)]
pub struct UseCase {
    /// The click is on the crafter's front face.
    pub front: bool,
    /// The crafter's phase is `IDLE`.
    pub idle: bool,
    /// `stack.isEmpty() && hand == MAIN_HAND` (`isHand`).
    pub empty_main_hand: bool,
    pub wrench: bool,
    /// The stack is a Crafter Slot Cover.
    pub cover_item: bool,
    /// The stack is a Mechanical Arm (it selects the crafter instead).
    pub arm: bool,
    /// The crafter has a cover on.
    pub covered: bool,
    pub slot_empty: bool,
    /// The held stack is the same item with the same components as the slot's.
    pub same_as_slot: bool,
}

/// `MechanicalCrafterBlock.useItemOn`, branch by branch.
pub fn use_item_on(c: UseCase) -> Use {
    if c.arm || !c.front {
        return Use::Pass;
    }
    if !c.idle && !c.wrench {
        return Use::EjectGrid;
    }
    if c.idle && !c.empty_main_hand && !c.wrench {
        if c.cover_item {
            return if c.covered || !c.slot_empty { Use::Pass } else { Use::PlaceCover };
        }
        return Use::Insert;
    }
    if c.slot_empty {
        return if c.covered && !c.wrench { Use::Uncover } else { Use::Pass };
    }
    if !c.empty_main_hand && !c.same_as_slot {
        return Use::Pass;
    }
    Use::TakeItem
}

/// `BlockHelper.hasBlockSolidSide`, from a block's collision boxes (`[min x, y, z, max x, y, z]`
/// in the block): some box fills the whole square of the face on `side`.
pub fn face_covered(boxes: &[[f64; 6]], side: Dir) -> bool {
    let axis = side.axis().index();
    boxes.iter().any(|b| {
        let reaches = if side.sign() > 0 { b[axis + 3] >= 1.0 - 1e-6 } else { b[axis] <= 1e-6 };
        reaches
            && (0..3).filter(|i| *i != axis).all(|i| b[i] <= 1e-6 && b[i + 3] >= 1.0 - 1e-6)
    })
}

// --- the edge a Wrench click joins by (`EdgeInteractionHandler`) -------------------------------------

/// `EdgeInteractionHandler.getBB`: the strip across the edge of the block at `pos` towards
/// `direction`, 6/16 of the block wide, straddling the edge, grown by 1/256: `(min, max)`. A
/// Wrench click landing in it joins or parts the neighbour there.
pub fn edge_box(pos: Pos, direction: Dir) -> ([f64; 3], [f64; 3]) {
    let n = direction.normal();
    let margin = 10.0 / 16.0;
    let mut min = [0.0; 3];
    let mut max = [0.0; 3];
    for k in 0..3 {
        let abs = f64::from(n[k].abs()) * margin;
        // `contract(abs)` shortens the box's far side; `move(abs / 2 + n / 2)`.
        let shift = abs / 2.0 + f64::from(n[k]) / 2.0;
        min[k] = f64::from(pos[k]) + shift - 1.0 / 256.0;
        max[k] = f64::from(pos[k]) + 1.0 - abs + shift + 1.0 / 256.0;
    }
    (min, max)
}

/// Where the eye ray `origin` + t * `dir` meets the plane of the face `face` of the block at
/// `pos` (`RaycastHelper.rayTraceRange`'s hit), or `None` if it never reaches it.
pub fn face_hit(pos: Pos, face: Dir, origin: [f64; 3], dir: [f64; 3]) -> Option<[f64; 3]> {
    let axis = face.axis().index();
    if dir[axis].abs() < 1e-9 {
        return None;
    }
    let plane = f64::from(pos[axis]) + if face.sign() > 0 { 1.0 } else { 0.0 };
    let t = (plane - origin[axis]) / dir[axis];
    if t < 0.0 {
        return None;
    }
    Some([0, 1, 2].map(|i| origin[i] + dir[i] * t))
}

/// `EdgeInteractionHandler.getActivatedDirection`: of the `candidates` (each already allowed to
/// connect), the first whose strip contains `hit`.
pub fn activated_direction(pos: Pos, candidates: &[Dir], hit: [f64; 3]) -> Option<Dir> {
    candidates.iter().copied().find(|d| {
        let (min, max) = edge_box(pos, *d);
        (0..3).all(|i| hit[i] >= min[i] && hit[i] <= max[i])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_crafter_points_by_its_facing_and_pointing() {
        // Facing south the viewer looks north: left is west.
        assert_eq!(target_direction(Dir::South, Pointing::Left), Dir::West);
        assert_eq!(target_direction(Dir::South, Pointing::Right), Dir::East);
        assert_eq!(target_direction(Dir::North, Pointing::Left), Dir::East);
        assert_eq!(target_direction(Dir::East, Pointing::Left), Dir::South);
        assert_eq!(target_direction(Dir::West, Pointing::Left), Dir::North);
        assert_eq!(target_direction(Dir::West, Pointing::Down), Dir::Down);
        assert_eq!(target_direction(Dir::North, Pointing::Up), Dir::Up);
    }

    #[test]
    fn pointing_cycles_in_the_enums_order() {
        let mut p = Pointing::Up;
        let mut seen = Vec::new();
        for _ in 0..4 {
            p = p.cycle();
            seen.push(p);
        }
        assert_eq!(seen, [Pointing::Left, Pointing::Down, Pointing::Right, Pointing::Up]);
    }

    #[test]
    fn pointing_from_facing_follows_the_axis_direction() {
        // A crafter facing south (positive) pointing to the east is Right... but the port reads
        // `positive ? LEFT : RIGHT` for east.
        assert_eq!(pointing_from_facing(Dir::East, Dir::South), Pointing::Left);
        assert_eq!(pointing_from_facing(Dir::East, Dir::North), Pointing::Right);
        assert_eq!(pointing_from_facing(Dir::West, Dir::South), Pointing::Right);
        assert_eq!(pointing_from_facing(Dir::North, Dir::East), Pointing::Left);
        assert_eq!(pointing_from_facing(Dir::South, Dir::East), Pointing::Right);
        assert_eq!(pointing_from_facing(Dir::Down, Dir::South), Pointing::Up);
        assert_eq!(pointing_from_facing(Dir::Up, Dir::South), Pointing::Down);
    }

    #[test]
    fn placement_copies_the_crafter_it_is_put_against() {
        assert_eq!(placement(Dir::East, Dir::South, false, Some(Dir::North)), (Dir::North, Pointing::Right));
        // Sneaking ignores it: faces the player, pointing towards the clicked face.
        assert_eq!(placement(Dir::East, Dir::South, true, Some(Dir::North)), (Dir::South, Pointing::Left));
        // Clicked on the side the player faces towards: points up.
        assert_eq!(placement(Dir::South, Dir::South, false, None), (Dir::South, Pointing::Up));
    }

    #[test]
    fn a_target_must_face_the_same_way_and_not_point_back() {
        assert!(is_valid_target(Dir::South, Pointing::Left, Some((Dir::South, Pointing::Left))));
        assert!(!is_valid_target(Dir::South, Pointing::Left, Some((Dir::South, Pointing::Right))));
        assert!(!is_valid_target(Dir::South, Pointing::Up, Some((Dir::North, Pointing::Up))));
        assert!(!is_valid_target(Dir::South, Pointing::Up, None));
    }

    #[test]
    fn countdown_speed_is_held_between_4_and_250() {
        assert_eq!(count_down_speed(0.0), 0);
        assert_eq!(count_down_speed(1.0), 4);
        assert_eq!(count_down_speed(-64.0), 64);
        assert_eq!(count_down_speed(256.0), 250);
    }

    fn row(n: i32) -> Inputs {
        (0..n).map(|x| ([x, 0, 0], Input::default())).collect()
    }

    #[test]
    fn crafters_join_and_part_where_clicked() {
        let mut inputs = row(3);
        toggle(&mut inputs, [0, 0, 0], [1, 0, 0]);
        toggle(&mut inputs, [1, 0, 0], [2, 0, 0]);
        let lead = inputs[&[0, 0, 0]].clone();
        assert!(lead.controller);
        let mut members: Vec<Pos> = lead.data.iter().map(|o| add([0; 3], *o)).collect();
        members.sort();
        assert_eq!(members, vec![[0, 0, 0], [1, 0, 0], [2, 0, 0]]);
        assert_eq!(inputs[&[2, 0, 0]].controller_of([2, 0, 0]), [0, 0, 0]);
        // Parting the first pair splits it: the second leads what still hangs on it.
        toggle(&mut inputs, [0, 0, 0], [1, 0, 0]);
        assert_eq!(inputs[&[0, 0, 0]], Input::default());
        let two = &inputs[&[1, 0, 0]];
        assert!(two.controller);
        assert_eq!(two.data.len(), 2);
        assert_eq!(inputs[&[2, 0, 0]].controller_of([2, 0, 0]), [1, 0, 0]);
    }

    #[test]
    fn two_groups_join_under_the_first() {
        let mut inputs = row(4);
        toggle(&mut inputs, [0, 0, 0], [1, 0, 0]);
        toggle(&mut inputs, [2, 0, 0], [3, 0, 0]);
        toggle(&mut inputs, [1, 0, 0], [2, 0, 0]);
        assert_eq!(inputs[&[0, 0, 0]].data.len(), 4);
        for x in 1..4 {
            assert_eq!(inputs[&[x, 0, 0]].controller_of([x, 0, 0]), [0, 0, 0]);
        }
    }

    #[test]
    fn breaking_a_crafter_parts_each_neighbour_of_its_group_in_turn() {
        // What `onRemove` does, a direction at a time, reading the inputs afresh each time.
        let mut inputs = row(3);
        toggle(&mut inputs, [0, 0, 0], [1, 0, 0]);
        toggle(&mut inputs, [1, 0, 0], [2, 0, 0]);
        let at = [1, 0, 0];
        for direction in [Dir::West, Dir::East] {
            let other = add(at, direction.normal());
            if same_group((at, &inputs[&at]), (other, &inputs[&other])) {
                toggle(&mut inputs, at, other);
            }
        }
        let alone = Input::default();
        assert_eq!(inputs[&[0, 0, 0]], alone);
        assert_eq!(inputs[&[2, 0, 0]], alone);
    }

    #[test]
    fn what_a_toggle_touches_is_the_two_groups() {
        let mut inputs = row(4);
        toggle(&mut inputs, [0, 0, 0], [1, 0, 0]);
        let read = |p: Pos| inputs.get(&p).cloned();
        let touched = touched(&read, [1, 0, 0], [3, 0, 0]);
        assert_eq!(touched, vec![[0, 0, 0], [1, 0, 0], [3, 0, 0]]);
    }

    #[test]
    fn a_group_reads_top_row_first_along_the_row_from_the_front() {
        // Facing south, x runs left to right for a viewer looking north... mirrored: modifier -1
        // on z comparing, +1 for x.
        let top = [0, 1, 0];
        let top2 = [1, 1, 0];
        let bottom = [0, 0, 0];
        let bottom2 = [1, 0, 0];
        let order = inventory_order(Dir::South, vec![bottom2, top2, bottom, top]);
        assert_eq!(order, vec![top, top2, bottom, bottom2]);
        // Facing north the row reads the other way.
        let order = inventory_order(Dir::North, vec![bottom2, top2, bottom, top]);
        assert_eq!(order, vec![top2, top, bottom2, bottom]);
        // Facing east a row runs along z (north to south from the front... modifier +1 * -1).
        let a = [0, 0, 0];
        let b = [0, 0, 1];
        assert_eq!(inventory_order(Dir::East, vec![b, a]), vec![b, a]);
        assert_eq!(inventory_order(Dir::West, vec![a, b]), vec![a, b]);
    }

    #[test]
    fn a_wrench_click_on_the_edge_picks_the_neighbour() {
        let (min, max) = edge_box([0, 0, 0], Dir::East);
        assert!((min[0] - (0.8125 - 1.0 / 256.0)).abs() < 1e-9);
        assert!((max[0] - (1.1875 + 1.0 / 256.0)).abs() < 1e-9);
        assert!(min[1] < 0.0 && max[1] > 1.0);
        let hit = [0.95, 0.5, 0.0];
        assert_eq!(activated_direction([0, 0, 0], &[Dir::West, Dir::East], hit), Some(Dir::East));
        assert_eq!(activated_direction([0, 0, 0], &[Dir::West, Dir::East], [0.5, 0.5, 0.0]), None);
        // The ray from in front meets the north face.
        let at = face_hit([0, 0, 0], Dir::North, [0.9, 0.5, -3.0], [0.0, 0.0, 1.0]).unwrap();
        assert_eq!(at, [0.9, 0.5, 0.0]);
        assert!(face_hit([0, 0, 0], Dir::North, [0.9, 0.5, -3.0], [0.0, 0.0, -1.0]).is_none());
    }

    fn case() -> UseCase {
        UseCase { front: true, idle: true, slot_empty: true, ..UseCase::default() }
    }

    #[test]
    fn a_click_on_the_front_does_what_the_block_says() {
        // An item goes in; a cover goes on an empty, uncovered slot.
        assert_eq!(use_item_on(case()), Use::Insert);
        assert_eq!(use_item_on(UseCase { cover_item: true, ..case() }), Use::PlaceCover);
        assert_eq!(use_item_on(UseCase { cover_item: true, covered: true, ..case() }), Use::Pass);
        assert_eq!(use_item_on(UseCase { cover_item: true, slot_empty: false, ..case() }), Use::Pass);
        // The empty hand takes the item, or the cover.
        let hand = UseCase { empty_main_hand: true, ..case() };
        assert_eq!(use_item_on(UseCase { slot_empty: false, ..hand }), Use::TakeItem);
        assert_eq!(use_item_on(UseCase { covered: true, ..hand }), Use::Uncover);
        assert_eq!(use_item_on(hand), Use::Pass);
        // A working crafter throws its grid out, whatever the hand holds; not for the wrench.
        assert_eq!(use_item_on(UseCase { idle: false, ..hand }), Use::EjectGrid);
        assert_eq!(use_item_on(UseCase { idle: false, wrench: true, ..case() }), Use::Pass);
        // Other faces and the arm pass.
        assert_eq!(use_item_on(UseCase { front: false, ..case() }), Use::Pass);
        assert_eq!(use_item_on(UseCase { arm: true, ..case() }), Use::Pass);
        // The wrench never covers or inserts.
        assert_eq!(use_item_on(UseCase { wrench: true, ..case() }), Use::Pass);
    }

    #[test]
    fn a_full_face_is_solid() {
        let full = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        let slab = [0.0, 0.0, 0.0, 1.0, 0.5, 1.0];
        assert!(face_covered(&[full], Dir::North));
        assert!(face_covered(&[slab], Dir::Down));
        assert!(!face_covered(&[slab], Dir::Up));
        assert!(!face_covered(&[slab], Dir::East));
        assert!(!face_covered(&[], Dir::Up));
    }

    #[test]
    fn the_picture_runs_on_the_count_down() {
        assert_eq!(estimated_count_down(1000, 16, 10.5), 832.0);
        assert_eq!(estimated_count_down(10, 16, 10.0), 0.0);
        assert_eq!(export_progress(1000.0), 0.0);
        assert_eq!(export_progress(250.0), 0.75);
        assert_eq!(export_progress(-5.0), 1.0);
        assert_eq!(gather_progress(2000.0), 0.0);
        assert_eq!(gather_progress(1500.0), 0.5);
        assert_eq!(early_late(0.25), (0.5, 0.0));
        assert_eq!(early_late(0.75), (1.0, 0.5));
        assert_eq!(crafting_spacing(0.0), 0.5);
        assert_eq!(crafting_spacing(0.5), 0.75);
        assert_eq!(centering(0, 0, 3, 3, 1.0), (-1.0, -1.0));
        assert_eq!(centering(-1, 0, 2, 1, 0.5), (0.25, 0.0));
        assert_eq!(export_layer(Pointing::Left), 2);
        assert_eq!(belt_frame(64, 8), 4);
    }
}
