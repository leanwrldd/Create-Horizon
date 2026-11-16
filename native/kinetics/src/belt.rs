//! Create's mechanical belt: the chain of belt blocks, and the items riding on it.
//!
//! Ported from Create 6.0.10 (`mc1.21.1/dev` at `4148fd8`, MIT, the Creators of Create):
//! `BeltBlock`, `BeltBlockEntity`, `BeltConnectorItem`, `transport/BeltInventory`,
//! `transport/TransportedItemStack` and `transport/ItemHandlerBeltSegment`.
//!
//! This is the straight horizontal belt. Items move along it, queue one block apart, and at the
//! end either fly off as item entities, wait at a block whose face closes the way, or are handed
//! on to a block that takes items from a belt (`Ending.INSERT`: another belt, a depot, a basin).
//! A processor two blocks above a block of the belt (the press) can hold an item in the middle
//! of that block while it works (`BeltProcessingBehaviour`). Fans, funnels, tunnels, crushers
//! and slopes are not here yet.
//!
//! An item's position counts blocks from the belt's start (its controller, `index` 0) along
//! the belt's facing: 0 is the start block's back edge, `length` the end block's front edge.

use crate::{Axis, Dir, Pos};

/// `CKinetics.maxBeltLength`'s default: the two shafts must be closer than this.
pub const MAX_LENGTH: i32 = 20;
/// `BeltInventory.tick`'s `spacing`: items keep this far apart.
pub const SPACING: f32 = 1.0;

/// `BeltPart`: which piece of the chain a belt block is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Part {
    /// The first block, where the controller lives. Has a pulley.
    Start,
    Middle,
    /// A middle block a shaft was put through: it has a pulley too.
    Pulley,
    /// The last block. Has a pulley.
    End,
}

impl Part {
    pub const ALL: [Part; 4] = [Part::Start, Part::Middle, Part::Pulley, Part::End];

    pub const fn name(self) -> &'static str {
        match self {
            Part::Start => "start",
            Part::Middle => "middle",
            Part::Pulley => "pulley",
            Part::End => "end",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == name)
    }

    /// `BeltBlockEntity.hasPulley`: every part but the middle one turns a shaft.
    pub const fn has_pulley(self) -> bool {
        !matches!(self, Part::Middle)
    }
}

/// `BeltBlock.getRotationAxis` for a horizontal belt: `facing.getClockWise().getAxis()`.
pub const fn rotation_axis(facing: Dir) -> Axis {
    match facing {
        Dir::North | Dir::South => Axis::X,
        _ => Axis::Z,
    }
}

/// `Direction.getClockWise` of a horizontal direction.
pub const fn clockwise(facing: Dir) -> Dir {
    match facing {
        Dir::North => Dir::East,
        Dir::East => Dir::South,
        Dir::South => Dir::West,
        Dir::West => Dir::North,
        other => other,
    }
}

/// `BeltBlockEntity.getBeltMovementSpeed`: blocks per tick at `rpm`.
pub fn movement_speed(rpm: f32) -> f32 {
    rpm / 480.0
}

/// `getDirectionAwareBeltMovementSpeed`: blocks per tick, positive towards the belt's end.
pub fn chain_speed(rpm: f32, facing: Dir) -> f32 {
    let mut offset = facing.sign() as f32;
    if facing.axis() == Axis::X {
        offset = -offset;
    }
    movement_speed(rpm) * offset
}

/// `getMovementFacing`: the way the belt's top moves.
pub fn movement_facing(rpm: f32, facing: Dir) -> Dir {
    let negative = (movement_speed(rpm) < 0.0) ^ (facing.axis() == Axis::X);
    let positive_dir = match facing.axis() {
        Axis::X => Dir::East,
        _ => Dir::South,
    };
    if negative {
        positive_dir.opposite()
    } else {
        positive_dir
    }
}

fn add(a: Pos, b: Pos) -> Pos {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: Pos, by: i32) -> Pos {
    [a[0] * by, a[1] * by, a[2] * by]
}

/// `BeltBlock.nextSegmentPosition` for a horizontal belt: the next block of the chain, or the
/// previous one, or none past either end.
pub fn next_segment(pos: Pos, facing: Dir, part: Part, forward: bool) -> Option<Pos> {
    if (part == Part::End && forward) || (part == Part::Start && !forward) {
        return None;
    }
    Some(add(pos, scale(facing.normal(), if forward { 1 } else { -1 })))
}

/// `BeltHelper.getPositionForOffset` for a horizontal belt.
pub fn segment_position(controller: Pos, facing: Dir, offset: i32) -> Pos {
    add(controller, scale(facing.normal(), offset))
}

/// Why two shafts can't be joined (`BeltConnectorItem.canConnect` refuses).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Not closer than [`MAX_LENGTH`].
    TooFar,
    /// The same shaft twice.
    Same,
    /// Not in a line that crosses both shafts' axis, or the axes differ.
    NotInLine,
    /// A slope, a vertical belt or a sideways one: not in this slice yet.
    NotHorizontal,
}

/// Where a belt between the shafts at `first` and `second` goes, both shafts on `axis`, as
/// `BeltConnectorItem.canConnect` and `createBelts` place it: its facing (from `first`, the
/// start, to `second`, the end) and every block of it in order. Only the geometry: the game
/// still checks the blocks between and the shafts' speeds.
pub fn connect(first: Pos, second: Pos, axis: Axis, second_axis: Axis) -> Result<(Dir, Vec<Pos>), Refusal> {
    let diff = [second[0] - first[0], second[1] - first[1], second[2] - first[2]];
    if diff == [0, 0, 0] {
        return Err(Refusal::Same);
    }
    // `Vec3i.closerThan`: distSqr < maxLength².
    let distance = diff.iter().map(|c| c * c).sum::<i32>();
    if distance >= MAX_LENGTH * MAX_LENGTH {
        return Err(Refusal::TooFar);
    }
    let [x, y, z] = diff.map(i32::abs);
    let sames = i32::from(x == y) + i32::from(y == z) + i32::from(z == x);
    if diff[axis.index()] != 0 || sames != 1 || axis != second_axis {
        return Err(Refusal::NotInLine);
    }
    if axis == Axis::Y || diff[1] != 0 {
        return Err(Refusal::NotHorizontal);
    }
    // `getFacingFromTo`: along x unless the two share an x.
    let along = if first[0] == second[0] { Axis::Z } else { Axis::X };
    let sign = diff[along.index()].signum();
    let facing = match (along, sign > 0) {
        (Axis::X, true) => Dir::East,
        (Axis::X, false) => Dir::West,
        (_, true) => Dir::South,
        (_, false) => Dir::North,
    };
    let steps = diff[along.index()].abs();
    let chain = (0..=steps)
        .map(|i| add(first, scale(facing.normal(), i)))
        .collect();
    Ok((facing, chain))
}

/// The part a block of a new belt gets (`createBelts`): `pulley` when a shaft stood there.
pub fn part_at(index: usize, length: usize, pulley: bool) -> Part {
    if index == 0 {
        Part::Start
    } else if index + 1 == length {
        Part::End
    } else if pulley {
        Part::Pulley
    } else {
        Part::Middle
    }
}

/// `BeltInventory.Ending`: what is past the belt's end, and how far before the edge items stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    /// Open: items fly off.
    Eject,
    /// A block whose face towards the belt is solid: items wait.
    Blocked,
    /// A block that takes items from a belt (`DirectBeltInputBehaviour`): items wait a quarter
    /// of a block short of it and are handed over, or stay if it will not take them.
    Insert,
}

impl Ending {
    pub const fn margin(self) -> f32 {
        match self {
            Ending::Eject => 0.0,
            Ending::Blocked => 0.45,
            Ending::Insert => 0.25,
        }
    }

    pub const fn id(self) -> i8 {
        match self {
            Ending::Eject => 0,
            Ending::Blocked => 1,
            Ending::Insert => 2,
        }
    }

    pub const fn from_id(id: i8) -> Self {
        match id {
            1 => Ending::Blocked,
            2 => Ending::Insert,
            _ => Ending::Eject,
        }
    }
}

/// `BeltProcessingBehaviour.ProcessingResult`: what a processor above the belt says of an item
/// at the middle of the block under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Processing {
    /// Let it go on.
    Pass,
    /// Keep it there (`locked`) until the processor lets go.
    Hold,
    /// Take it off the belt.
    Remove,
}

/// What came of handing an item to the block at the belt's end (`handleInsertion`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Insertion {
    /// Nothing was taken: the item waits.
    Nothing,
    /// Some of the stack was taken (the item's stack is now what was left).
    Part,
    /// All of it: the item is gone from the belt.
    All,
}

/// What a belt's surroundings answer while its items move: the end of the belt, and what sits
/// above it. The glue implements this over the game; a plain `|| Ending` does for the rules.
pub trait Surroundings<S> {
    /// `BeltInventory.resolveEnding`: asked once, and only when an item comes near the end.
    fn ending(&mut self) -> Ending;

    /// `BeltProcessingBehaviour.handleHeldItem` of the processor above `segment`, for an item
    /// it holds; `None` where there is none (the item is let go).
    fn held(&mut self, _segment: i32, _item: &Item<S>) -> Option<Processing> {
        None
    }

    /// `BeltProcessingBehaviour.handleReceivedItem` for an item about to cross the middle of
    /// `segment`; `None` where no processor works there (or the space is blocked).
    fn received(&mut self, _segment: i32, _item: &Item<S>) -> Option<Processing> {
        None
    }

    /// `handleInsertion` into the block past the end for an item at the end of the belt, with
    /// `canInsertFromSide` already asked. Changes the item's stack to what was left.
    fn insert(&mut self, _item: &mut Item<S>) -> Insertion {
        Insertion::Nothing
    }
}

/// A closure that only knows the ending.
struct Plain<F>(F);

impl<S, F: FnMut() -> Ending> Surroundings<S> for Plain<F> {
    fn ending(&mut self) -> Ending {
        (self.0)()
    }
}

/// `TransportedItemStack`: one stack riding the belt. `S` is the game's item stack.
#[derive(Clone, Debug, PartialEq)]
pub struct Item<S> {
    pub stack: S,
    /// `beltPosition`.
    pub pos: f32,
    /// `sideOffset`: across the belt, eased towards [`Item::target_side`].
    pub side: f32,
    /// `angle`, in degrees: how it lies, and where across the belt it settles.
    pub angle: i32,
    /// `insertedAt`: the segment it was put on.
    pub inserted_at: i32,
    /// `insertedFrom`.
    pub inserted_from: Dir,
    /// `locked`: a processor above holds it in the middle of a block.
    pub locked: bool,
}

impl<S> Item<S> {
    /// A stack dropped onto segment `segment` (`ItemHandlerBeltSegment.insertItem`), lying at
    /// `angle` (`new TransportedItemStack`: random, or 180 for an upright item).
    pub fn dropped(stack: S, segment: i32, angle: i32, positive: bool) -> Self {
        let mut item = Self {
            stack,
            pos: segment as f32 + 0.5 + if positive { -1.0 } else { 1.0 } / 16.0,
            side: 0.0,
            angle,
            inserted_at: segment,
            inserted_from: Dir::Up,
            locked: false,
        };
        item.side = item.target_side();
        item
    }

    /// `getTargetSideOffset`.
    pub fn target_side(&self) -> f32 {
        (self.angle - 180) as f32 / (360.0 * 3.0)
    }
}

/// What one tick did to a belt's items.
#[derive(Debug, PartialEq)]
pub struct Moved<S> {
    /// Stacks that went off the end (`BeltInventory.eject`), with the position they left at.
    pub ejected: Vec<Item<S>>,
    /// Anything moved, so the saved items changed.
    pub moved: bool,
    /// The list changed in a way a client can't foresee: an item left, was held or let go, was
    /// handed on, or the order turned.
    pub reordered: bool,
}

/// What `BeltInventory.handleBeltProcessingAndCheckIfRemoved` found.
enum Handled {
    Nothing,
    /// The item was locked or unlocked, or its place changed.
    Changed,
    /// A processor took the item off the belt.
    Removed,
}

/// `BeltInventory.handleBeltProcessingAndCheckIfRemoved` for one item about to move to
/// `next_offset`: a held item asks its processor to go on holding it; a free one that will
/// cross the middle of a block with a processor above it is offered to that processor.
fn handle_processing<S>(
    item: &mut Item<S>,
    next_offset: f32,
    no_movement: bool,
    positive: bool,
    env: &mut impl Surroundings<S>,
) -> Handled {
    // Continue processing if held.
    if item.locked {
        // `(int) beltPosition`.
        let segment = item.pos as i32;
        return match env.held(segment, item) {
            None | Some(Processing::Pass) => {
                item.locked = false;
                Handled::Changed
            }
            Some(Processing::Remove) => Handled::Removed,
            Some(Processing::Hold) => Handled::Nothing,
        };
    }
    if no_movement {
        return Handled::Nothing;
    }
    // See if any new belt processing catches the item.
    if item.pos > 0.5 || positive {
        let first = (item.pos + if positive { 0.5 } else { -0.5 }) as i32;
        let step = if positive { 1 } else { -1 };
        let mut segment = first;
        while if positive {
            segment as f32 + 0.5 <= next_offset
        } else {
            segment as f32 + 0.5 >= next_offset
        } {
            match env.received(segment, item) {
                Some(Processing::Remove) => return Handled::Removed,
                Some(Processing::Hold) => {
                    item.pos =
                        segment as f32 + 0.5 + if positive { 1.0 / 512.0 } else { -1.0 / 512.0 };
                    item.locked = true;
                    return Handled::Changed;
                }
                _ => {}
            }
            segment += step;
        }
    }
    Handled::Nothing
}

/// `BeltInventory`: the items of one belt, kept on its controller.
#[derive(Clone, Debug, PartialEq)]
pub struct Inventory<S> {
    /// The item furthest along the belt first.
    pub items: Vec<Item<S>>,
    /// `beltMovementPositive`: the belt last moved towards its end.
    pub positive: bool,
}

impl<S> Default for Inventory<S> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            positive: false,
        }
    }
}

impl<S> Inventory<S> {
    /// `BeltInventory.tick` on the server for a belt with nothing above it and a plain end:
    /// see [`Inventory::tick_with`].
    pub fn tick(&mut self, speed: f32, length: i32, ending: impl FnMut() -> Ending) -> Moved<S> {
        self.tick_with(speed, length, &mut Plain(ending))
    }

    /// `BeltInventory.tick` on the server, for a belt `length` blocks long moving `speed`
    /// blocks a tick towards its end ([`chain_speed`]). `env` is asked for the ending only once
    /// an item gets near the end in this tick, and for the processors above and the block at
    /// the end as items reach them.
    pub fn tick_with(
        &mut self,
        speed: f32,
        length: i32,
        env: &mut impl Surroundings<S>,
    ) -> Moved<S> {
        let mut moved = Moved {
            ejected: Vec::new(),
            moved: false,
            reordered: false,
        };
        if speed == 0.0 {
            return moved;
        }
        // The belt just reversed: the item furthest along is now at the other end of the list.
        if self.positive != (speed > 0.0) {
            self.positive = !self.positive;
            self.items.reverse();
            moved.reordered = true;
        }
        let positive = self.positive;
        let mut resolved: Option<Ending> = None;
        // The position of the item just handled, the one in front; an ejected item counts too.
        let mut front: Option<f32> = None;
        let mut kept = Vec::with_capacity(self.items.len());
        for mut item in self.items.drain(..) {
            let mut movement = speed;
            // Don't move if other items are waiting in front.
            let mut no_movement = false;
            if let Some(front) = front {
                let diff = front - item.pos;
                if diff.abs() <= SPACING {
                    no_movement = true;
                }
                movement = if positive {
                    movement.min(diff - SPACING)
                } else {
                    movement.max(diff + SPACING)
                };
            }
            // Don't move beyond the edge.
            let mut to_end = if positive {
                length as f32 - item.pos
            } else {
                -item.pos
            };
            if to_end.abs() < movement.abs() + 1.0 {
                let margin = resolved.get_or_insert_with(|| env.ending()).margin();
                to_end += if positive { -margin } else { margin };
            }
            let limited = if positive {
                movement.min(to_end)
            } else {
                movement.max(to_end)
            };
            // Belt item processing.
            let next_offset = item.pos + limited;
            match handle_processing(&mut item, next_offset, no_movement, positive, env) {
                Handled::Nothing => {}
                Handled::Changed => moved.reordered = true,
                Handled::Removed => {
                    front = Some(item.pos);
                    moved.reordered = true;
                    continue;
                }
            }
            if item.locked || no_movement {
                front = Some(item.pos);
                kept.push(item);
                continue;
            }
            item.pos += limited;
            let to_middle = item.target_side() - item.side;
            item.side += (to_middle * limited.abs() * 6.0).clamp(-to_middle.abs(), to_middle.abs());
            if limited != 0.0 {
                moved.moved = true;
            }
            front = Some(item.pos);
            // Movement successful.
            if limited == movement {
                kept.push(item);
                continue;
            }
            // The end was reached.
            match resolved {
                Some(Ending::Eject) => {
                    moved.ejected.push(item);
                    moved.reordered = true;
                }
                Some(Ending::Insert) => match env.insert(&mut item) {
                    Insertion::Nothing => kept.push(item),
                    Insertion::Part => {
                        moved.reordered = true;
                        kept.push(item);
                    }
                    Insertion::All => moved.reordered = true,
                },
                // A closed end holds the item.
                _ => kept.push(item),
            }
        }
        self.items = kept;
        moved
    }

    /// `canInsertAtFromSide(segment, UP)`: an item dropped on `segment` lately is still too close.
    pub fn can_insert_at(&self, segment: i32) -> bool {
        let at = segment as f32 + 0.5;
        !self.items.iter().any(|item| {
            item.inserted_at == segment
                && item.inserted_from == Dir::Up
                && if self.positive {
                    item.pos <= at + 1.0
                } else {
                    item.pos >= at - 1.0
                }
        })
    }

    /// `canInsertAtFromSide(segment, side)` for a belt moving towards `movement`: whether a
    /// stack handed over at `segment` from `side` would still be too close to the last one that
    /// came that way. A belt can't take items from the side it moves towards.
    pub fn can_insert_at_from_side(&self, segment: i32, side: Dir, movement: Dir) -> bool {
        if movement == side.opposite() {
            return false;
        }
        let mut at = segment as f32;
        if movement != side {
            at += 0.5;
        } else if !self.positive {
            at += 1.0;
        }
        !self.items.iter().any(|item| {
            item.inserted_at == segment
                && item.inserted_from == side
                && if self.positive {
                    item.pos <= at + 1.0
                } else {
                    item.pos >= at - 1.0
                }
        })
    }

    /// `BeltInventory.insert`: keeps the list ordered from the item furthest along.
    pub fn insert(&mut self, item: Item<S>) {
        let index = self
            .items
            .iter()
            .position(|other| (other.pos < item.pos) == self.positive)
            .unwrap_or(self.items.len());
        self.items.insert(index, item);
    }

    /// Takes every item within `distance` of `position` (`applyToEachWithin` removing them, as
    /// an empty hand does at `index + .5` within .55).
    pub fn take_within(&mut self, position: f32, distance: f32) -> Vec<Item<S>> {
        let (taken, kept) = std::mem::take(&mut self.items)
            .into_iter()
            .partition(|item| (position - item.pos).abs() < distance);
        self.items = kept;
        taken
    }
}

/// Where a stack handed on to a belt lands (`BeltBlockEntity.tryInsertingFromSide`): the belt's
/// own block `index`, its direction-aware speed (blocks a tick towards its end, not zero), the
/// way it moves (`movement`) and the way the stack was moving when it came (`side`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Entry {
    /// `beltPosition`.
    pub pos: f32,
    /// `sideOffset`.
    pub side: f32,
}

/// `item_side` is the stack's own `sideOffset` (kept when it comes along the belt); `from_belt`
/// says it was carried by a belt (`prevBeltPosition != 0`) and `belt_behind` that the block behind
/// this one in the direction of movement is a belt block, so the stack comes onto the next belt
/// where the last one left it (`.26` of a block short of the start).
pub fn entry(
    index: i32,
    speed: f32,
    movement: Dir,
    side: Dir,
    item_side: f32,
    from_belt: bool,
    belt_behind: bool,
) -> Entry {
    let mut pos = index as f32 + 0.5 - speed.signum() / 16.0;
    let mut side_offset = item_side;
    if side.axis() != Axis::Y {
        if movement != side {
            // Across the belt: dropped on its near edge.
            side_offset = side.sign() as f32 * 0.675;
            if side.axis() == Axis::X {
                side_offset = -side_offset;
            }
        } else {
            // This creates a smoother transition from belt to belt.
            let extra = if from_belt && belt_behind { 0.26 } else { 0.0 };
            pos = if speed > 0.0 {
                index as f32 - extra
            } else {
                index as f32 + 1.0 + extra
            };
        }
    }
    Entry {
        pos,
        side: side_offset,
    }
}

/// Where a client draws the items `ticks` after the server last sent them: each moves at
/// `speed` until the end (less `margin`) or the item in front stops it, as the server's
/// [`Inventory::tick`] will move them while nothing new happens. `positions` are front first; an
/// item a processor holds (`locked`) stays where it is.
pub fn foresee(
    positions: &[f32],
    locked: &[bool],
    speed: f32,
    ticks: f32,
    length: i32,
    margin: f32,
) -> Vec<f32> {
    let positive = speed > 0.0;
    let end = if positive {
        length as f32 - margin
    } else {
        margin
    };
    let mut front: Option<f32> = None;
    positions
        .iter()
        .enumerate()
        .map(|(i, &pos)| {
            if locked.get(i).copied().unwrap_or(false) {
                front = Some(pos);
                return pos;
            }
            let mut limit = end;
            if let Some(front) = front {
                limit = if positive {
                    limit.min(front - SPACING)
                } else {
                    limit.max(front + SPACING)
                };
            }
            let ahead = pos + speed * ticks.max(0.0);
            let drawn = if positive {
                ahead.min(limit).max(pos)
            } else {
                ahead.max(limit).min(pos)
            };
            front = Some(drawn);
            drawn
        })
        .collect()
}

// --- How a belt looks: `BeltRenderer`, `BeltModel` and `AllSpriteShifts` -------------------------

/// `BeltSlope`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slope {
    Horizontal,
    Upward,
    Downward,
    Vertical,
    Sideways,
}

impl Slope {
    /// `BeltRenderer`'s `diagonal`: a belt on a slope has its own model and sprite.
    pub const fn is_diagonal(self) -> bool {
        matches!(self, Slope::Upward | Slope::Downward)
    }
}

/// A sprite shift a model is drawn with: its faces textured `source` are drawn from `target`
/// instead, their UVs (in unit-sprite coordinates, 0 to 1 over the whole sprite) multiplied by
/// `scale` and then moved by `offset`.
#[derive(Clone, Debug, PartialEq)]
pub struct SpriteShift {
    pub source: String,
    pub target: String,
    pub scale: [f32; 2],
    pub offset: [f32; 2],
}

/// `DyeColor.getSerializedName`, in `DyeColor` order (what `AllSpriteShifts.populateMaps` names
/// the dyed sprites by).
pub const DYE_COLORS: [&str; 16] = [
    "white",
    "orange",
    "magenta",
    "light_blue",
    "yellow",
    "lime",
    "pink",
    "gray",
    "light_gray",
    "cyan",
    "purple",
    "blue",
    "brown",
    "green",
    "red",
    "black",
];

/// The sprite name of a `DyeColor` from its saved name (`NBTHelper.writeEnum` writes
/// `Enum.name()`, `LIGHT_BLUE`), or none for a name that is no colour.
pub fn dye_id(name: &str) -> Option<&'static str> {
    DYE_COLORS.into_iter().find(|id| id.eq_ignore_ascii_case(name))
}

/// `AllSpriteShifts.BELT`, `BELT_OFFSET` and `BELT_DIAGONAL`: the sprites the belt models name.
const BELT_SPRITE: &str = "create:block/belt";
const BELT_OFFSET_SPRITE: &str = "create:block/belt_offset";
const BELT_DIAGONAL_SPRITE: &str = "create:block/belt_diagonal";
/// What they scroll over: `belt_scroll.png` and `belt_diagonal_scroll.png`.
const BELT_SCROLL_SPRITE: &str = "create:block/belt_scroll";
const BELT_DIAGONAL_SCROLL_SPRITE: &str = "create:block/belt_diagonal_scroll";
/// The belt sprites are 16 by 16 texels and the sprites they scroll over 16 by 32: one tile that
/// repeats once, so a scroll of a tile wraps unseen.
const TILE_SIZE: [f32; 2] = [16.0, 16.0];
const SCROLL_SIZE: [f32; 2] = [16.0, 32.0];

/// `BeltRenderer.getSpriteShiftEntry`: the sprite a belt model names and the one it scrolls over
/// (`AllSpriteShifts.BELT`, `BELT_OFFSET`, `BELT_DIAGONAL` and the `DYED_*` maps). `dye` is a
/// sprite name from [`dye_id`].
pub fn belt_sprites(dye: Option<&str>, diagonal: bool, bottom: bool) -> (String, String) {
    let source = if diagonal {
        BELT_DIAGONAL_SPRITE
    } else if bottom {
        BELT_OFFSET_SPRITE
    } else {
        BELT_SPRITE
    };
    let target = match (dye, diagonal) {
        (Some(id), true) => format!("create:block/belt/{id}_diagonal_scroll"),
        (Some(id), false) => format!("create:block/belt/{id}_scroll"),
        (None, true) => BELT_DIAGONAL_SCROLL_SPRITE.to_owned(),
        (None, false) => BELT_SCROLL_SPRITE.to_owned(),
    };
    (source.to_owned(), target)
}

/// `BeltRenderer.renderSafe`'s UV shift: how far, as a share of the scroll sprite's height, the
/// belt's texture has moved at `time` (`AnimationTickHolder.getRenderTime`, ticks and the part
/// of one) for a belt at `rpm` facing `facing`. 0 up to 0.5 (3/8 on a slope).
pub fn scroll(rpm: f32, facing: Dir, slope: Slope, bottom: bool, time: f64) -> f32 {
    let diagonal = slope.is_diagonal();
    let along_x = facing.axis() == Axis::X;
    let sideways = slope == Slope::Sideways;
    let mut speed = f64::from(rpm);
    // `time = renderTick * axisDirection.getStep()`.
    let time = time * f64::from(facing.sign());
    if (diagonal && ((slope == Slope::Downward) ^ along_x))
        || (!sideways && !diagonal && along_x)
        || (sideways && facing.sign() < 0)
    {
        speed = -speed;
    }
    let multiplier = if diagonal { 3.0 / 8.0 } else { 0.5 };
    let scrolled = speed * time / (31.5 * 16.0) + if bottom { 0.5 } else { 0.0 };
    ((scrolled - scrolled.floor()) * multiplier) as f32
}

/// The sprite shift `BeltRenderer.renderSafe` draws one half of a belt block's model with, or
/// none while it draws the model as it is (`speed != 0 || be.color.isPresent()` is false).
/// `dye` is the saved colour name, `time` as for [`scroll`].
pub fn belt_shift(
    rpm: f32,
    facing: Dir,
    slope: Slope,
    bottom: bool,
    dye: Option<&str>,
    time: f64,
) -> Option<SpriteShift> {
    let dye = dye.and_then(dye_id);
    if rpm == 0.0 && dye.is_none() {
        return None;
    }
    let (source, target) = belt_sprites(dye, slope.is_diagonal(), bottom);
    Some(SpriteShift {
        source,
        target,
        // The tile shows in the top half of the sprite it scrolls over, as Create's atlas
        // coordinates put it there: `uv - original.v0 + target.v0 + scroll`.
        scale: [TILE_SIZE[0] / SCROLL_SIZE[0], TILE_SIZE[1] / SCROLL_SIZE[1]],
        offset: [0.0, scroll(rpm, facing, slope, bottom, time)],
    })
}

/// Whether a belt's drawn geometry changes with time: only a belt that moves has to be drawn
/// again every frame.
pub fn scrolls(rpm: f32) -> bool {
    rpm != 0.0
}

/// `BeltBlockEntity.CasingType`, saved as its ordinal (`Casing`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Casing {
    None,
    Andesite,
    Brass,
}

impl Casing {
    pub const fn ordinal(self) -> i32 {
        match self {
            Casing::None => 0,
            Casing::Andesite => 1,
            Casing::Brass => 2,
        }
    }

    pub const fn from_ordinal(ordinal: i32) -> Self {
        match ordinal {
            1 => Casing::Andesite,
            2 => Casing::Brass,
            _ => Casing::None,
        }
    }
}

/// `AllSpriteShifts.ANDESIDE_BELT_CASING`: the casing models name the brass casing sprite.
const BRASS_CASING_SPRITE: &str = "create:block/belt/brass_belt_casing";
const ANDESITE_CASING_SPRITE: &str = "create:block/belt/andesite_belt_casing";

/// `BeltModel.getQuads`: a brass casing is the model as it is, an andesite one has every quad
/// of the brass sprite moved onto the andesite sprite (the two are the same size).
pub fn casing_shift(casing: Casing) -> Option<SpriteShift> {
    (casing == Casing::Andesite).then(|| SpriteShift {
        source: BRASS_CASING_SPRITE.to_owned(),
        target: ANDESITE_CASING_SPRITE.to_owned(),
        scale: [1.0, 1.0],
        offset: [0.0, 0.0],
    })
}

#[cfg(test)]
mod look_tests {
    use super::*;

    #[test]
    fn colours_come_from_their_saved_names() {
        assert_eq!(dye_id("LIGHT_BLUE"), Some("light_blue"));
        assert_eq!(dye_id("RED"), Some("red"));
        assert_eq!(dye_id("red"), Some("red"));
        assert_eq!(dye_id(""), None);
        assert_eq!(dye_id("TEAL"), None);
        assert_eq!(DYE_COLORS.len(), 16);
    }

    #[test]
    fn sprites_follow_the_shift_entries() {
        let pair = belt_sprites;
        assert_eq!(
            pair(None, false, false),
            ("create:block/belt".into(), "create:block/belt_scroll".into())
        );
        assert_eq!(
            pair(None, false, true),
            ("create:block/belt_offset".into(), "create:block/belt_scroll".into())
        );
        assert_eq!(
            pair(None, true, false),
            (
                "create:block/belt_diagonal".into(),
                "create:block/belt_diagonal_scroll".into()
            )
        );
        // A slope has no separate bottom model, so `bottom` doesn't matter there.
        assert_eq!(pair(None, true, true), pair(None, true, false));
        assert_eq!(
            pair(Some("lime"), false, false),
            ("create:block/belt".into(), "create:block/belt/lime_scroll".into())
        );
        assert_eq!(
            pair(Some("light_blue"), false, true),
            (
                "create:block/belt_offset".into(),
                "create:block/belt/light_blue_scroll".into()
            )
        );
        assert_eq!(
            pair(Some("red"), true, false),
            (
                "create:block/belt_diagonal".into(),
                "create:block/belt/red_diagonal_scroll".into()
            )
        );
    }

    #[test]
    fn a_still_undyed_belt_is_drawn_as_it_is() {
        assert_eq!(belt_shift(0.0, Dir::South, Slope::Horizontal, false, None, 10.0), None);
        // An unknown colour is no colour.
        assert_eq!(
            belt_shift(0.0, Dir::South, Slope::Horizontal, false, Some("TEAL"), 3.0),
            None
        );
        assert!(!scrolls(0.0));
        assert!(scrolls(-16.0));
    }

    #[test]
    fn a_still_dyed_belt_wears_its_colour_without_moving() {
        let at =
            |time| belt_shift(0.0, Dir::South, Slope::Horizontal, false, Some("RED"), time).unwrap();
        let shift = at(0.0);
        assert_eq!(shift.target, "create:block/belt/red_scroll");
        assert_eq!(shift.offset, [0.0, 0.0]);
        assert_eq!(shift, at(1234.5));
        // The bottom half starts half a tile on.
        let bottom = belt_shift(0.0, Dir::South, Slope::Horizontal, true, Some("RED"), 7.0).unwrap();
        assert_eq!(bottom.offset, [0.0, 0.25]);
    }

    #[test]
    fn the_tile_shows_in_the_top_half_of_the_scroll_sprite() {
        let shift = belt_shift(16.0, Dir::South, Slope::Horizontal, false, None, 0.0).unwrap();
        assert_eq!(shift.scale, [1.0, 0.5]);
        assert_eq!(shift.source, "create:block/belt");
        assert_eq!(shift.target, "create:block/belt_scroll");
    }

    #[test]
    fn scrolling_follows_speed_time_and_facing() {
        // 16 rpm: a tile in 31.5 ticks; the offset is half of the wrapped phase.
        let quarter = scroll(16.0, Dir::South, Slope::Horizontal, false, 31.5 / 4.0);
        assert!((quarter - 0.125).abs() < 1e-6, "{quarter}");
        // It wraps: a whole turn is back at the start.
        let wrapped = scroll(16.0, Dir::South, Slope::Horizontal, false, 31.5);
        assert!(wrapped.abs() < 1e-6 || (wrapped - 0.5).abs() < 1e-6, "{wrapped}");
        // Half a tile on for the bottom half.
        let bottom = scroll(16.0, Dir::South, Slope::Horizontal, true, 0.0);
        assert!((bottom - 0.25).abs() < 1e-6);
        // `time` runs backwards for a belt facing a negative end of its axis and the speed is
        // reversed along x: south and west scroll one way, north and east the other.
        let phase = |facing| scroll(16.0, facing, Slope::Horizontal, false, 5.0) * 2.0;
        let (south, north) = (phase(Dir::South), phase(Dir::North));
        let (east, west) = (phase(Dir::East), phase(Dir::West));
        assert!((south + north - 1.0).abs() < 1e-5);
        assert!((south - west).abs() < 1e-5);
        assert!((north - east).abs() < 1e-5);
        // A reversed shaft reverses the scroll.
        let back = scroll(-16.0, Dir::South, Slope::Horizontal, false, 5.0) * 2.0;
        assert!((south + back - 1.0).abs() < 1e-5);
    }

    #[test]
    fn a_slope_scrolls_by_three_eighths_of_a_sprite() {
        for time in [0.0, 3.0, 11.0, 29.9] {
            let up = scroll(32.0, Dir::South, Slope::Upward, false, time);
            assert!((0.0..0.375).contains(&up), "{up}");
        }
        // Down and up run opposite ways on the same facing.
        let up = scroll(32.0, Dir::South, Slope::Upward, false, 4.0) / 0.375;
        let down = scroll(32.0, Dir::South, Slope::Downward, false, 4.0) / 0.375;
        assert!((up + down - 1.0).abs() < 1e-5, "{up} {down}");
        // A slope has its own sprites.
        let shift = belt_shift(32.0, Dir::South, Slope::Upward, false, None, 0.0).unwrap();
        assert_eq!(shift.source, "create:block/belt_diagonal");
        assert_eq!(shift.target, "create:block/belt_diagonal_scroll");
    }

    #[test]
    fn a_sideways_belt_reverses_on_a_negative_facing() {
        let plus = scroll(16.0, Dir::South, Slope::Sideways, false, 6.0);
        let minus = scroll(16.0, Dir::North, Slope::Sideways, false, 6.0);
        // `time` flips with the facing and so does `speed`, so the two agree.
        assert!((plus - minus).abs() < 1e-5, "{plus} {minus}");
    }

    #[test]
    fn only_an_andesite_casing_is_shifted() {
        assert_eq!(casing_shift(Casing::None), None);
        assert_eq!(casing_shift(Casing::Brass), None);
        let shift = casing_shift(Casing::Andesite).unwrap();
        assert_eq!(shift.source, "create:block/belt/brass_belt_casing");
        assert_eq!(shift.target, "create:block/belt/andesite_belt_casing");
        assert_eq!((shift.scale, shift.offset), ([1.0, 1.0], [0.0, 0.0]));
    }

    #[test]
    fn casings_are_saved_as_their_ordinal() {
        for casing in [Casing::None, Casing::Andesite, Casing::Brass] {
            assert_eq!(Casing::from_ordinal(casing.ordinal()), casing);
        }
        assert_eq!(Casing::from_ordinal(7), Casing::None);
        assert_eq!(Casing::from_ordinal(-1), Casing::None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn belt(positions: &[f32], positive: bool) -> Inventory<u32> {
        Inventory {
            items: positions
                .iter()
                .enumerate()
                .map(|(i, &pos)| Item {
                    stack: i as u32,
                    pos,
                    side: 0.0,
                    angle: 180,
                    inserted_at: 0,
                    inserted_from: Dir::Up,
                    locked: false,
                })
                .collect(),
            positive,
        }
    }

    fn positions(inventory: &Inventory<u32>) -> Vec<f32> {
        inventory.items.iter().map(|i| i.pos).collect()
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn speeds_and_facings_follow_create() {
        // 480 RPM would be a block a tick.
        assert_eq!(movement_speed(48.0), 0.1);
        // A belt facing south moves towards its end when the speed is positive; one facing
        // east the other way round, as `getDirectionAwareBeltMovementSpeed` flips x.
        assert_eq!(chain_speed(48.0, Dir::South), 0.1);
        assert_eq!(chain_speed(48.0, Dir::North), -0.1);
        assert_eq!(chain_speed(48.0, Dir::East), -0.1);
        assert_eq!(chain_speed(48.0, Dir::West), 0.1);
        // The top moves the way the chain speed says.
        for facing in [Dir::North, Dir::South, Dir::East, Dir::West] {
            for rpm in [32.0, -32.0] {
                let towards_end = chain_speed(rpm, facing) > 0.0;
                let expected = if towards_end { facing } else { facing.opposite() };
                assert_eq!(movement_facing(rpm, facing), expected, "{facing:?} {rpm}");
            }
        }
        assert_eq!(rotation_axis(Dir::North), Axis::X);
        assert_eq!(rotation_axis(Dir::East), Axis::Z);
    }

    #[test]
    fn a_stack_rides_to_the_end_and_flies_off() {
        let mut inventory = Inventory::default();
        inventory.insert(Item::dropped(7u32, 0, 180, true));
        let speed = chain_speed(256.0, Dir::South);
        let mut ejected = None;
        for _ in 0..100 {
            let moved = inventory.tick(speed, 3, || Ending::Eject);
            if let Some(item) = moved.ejected.into_iter().next() {
                assert!(moved.reordered);
                ejected = Some(item);
                break;
            }
        }
        let item = ejected.expect("the stack leaves the belt");
        assert_eq!(item.stack, 7);
        // It left at the end's front edge.
        assert!(close(item.pos, 3.0), "{}", item.pos);
        assert!(inventory.items.is_empty());
    }

    #[test]
    fn a_closed_end_holds_the_stack_and_queues_the_next() {
        let mut inventory = belt(&[1.5, 0.5], true);
        let speed = chain_speed(256.0, Dir::South);
        for _ in 0..200 {
            let moved = inventory.tick(speed, 4, || Ending::Blocked);
            assert!(moved.ejected.is_empty());
        }
        let at = positions(&inventory);
        assert!(close(at[0], 4.0 - 0.45), "{at:?}");
        assert!(close(at[1], 4.0 - 0.45 - SPACING), "{at:?}");
        // Opened again: the front one goes, then the one behind.
        let mut gone = 0;
        for _ in 0..200 {
            gone += inventory.tick(speed, 4, || Ending::Eject).ejected.len();
        }
        assert_eq!(gone, 2);
    }

    #[test]
    fn a_stopped_belt_moves_nothing() {
        let mut inventory = belt(&[1.0], true);
        let moved = inventory.tick(0.0, 4, || unreachable!());
        assert!(!moved.moved && !moved.reordered);
        assert_eq!(positions(&inventory), vec![1.0]);
    }

    #[test]
    fn the_end_is_only_asked_for_near_it() {
        let mut inventory = belt(&[0.5], true);
        inventory.tick(0.1, 10, || panic!("far from the end"));
        let mut asked = false;
        let mut near = belt(&[9.5], true);
        near.tick(0.1, 10, || {
            asked = true;
            Ending::Eject
        });
        assert!(asked);
    }

    #[test]
    fn reversing_turns_the_order_and_runs_back() {
        let mut inventory = belt(&[2.5, 0.5], true);
        let moved = inventory.tick(-0.1, 4, || Ending::Blocked);
        assert!(moved.reordered);
        assert!(!inventory.positive);
        // The one nearest the start leads now, and stops short of the closed start.
        assert!(close(inventory.items[0].pos, 0.45));
        assert!(close(inventory.items[1].pos, 2.4));
        for _ in 0..100 {
            inventory.tick(-0.1, 4, || Ending::Blocked);
        }
        let at = positions(&inventory);
        assert!(close(at[0], 0.45) && close(at[1], 1.45), "{at:?}");
    }

    #[test]
    fn a_drop_waits_for_the_last_drop_on_that_segment_to_move_on() {
        let mut inventory = belt(&[], true);
        assert!(inventory.can_insert_at(1));
        inventory.insert(Item::dropped(1u32, 1, 180, true));
        assert!(!inventory.can_insert_at(1));
        // Others segments are free.
        assert!(inventory.can_insert_at(0));
        assert!(inventory.can_insert_at(2));
        inventory.items[0].pos = 2.6;
        assert!(inventory.can_insert_at(1));
    }

    #[test]
    fn inserting_keeps_the_front_first() {
        let mut forward = belt(&[3.0, 1.0], true);
        forward.insert(Item::dropped(9, 2, 180, true));
        assert_eq!(forward.items.iter().map(|i| i.stack).collect::<Vec<_>>(), vec![0, 9, 1]);
        let mut back = belt(&[1.0, 3.0], false);
        back.insert(Item::dropped(9, 2, 180, false));
        assert_eq!(back.items.iter().map(|i| i.stack).collect::<Vec<_>>(), vec![0, 9, 1]);
    }

    #[test]
    fn a_hand_takes_what_is_on_its_segment() {
        let mut inventory = belt(&[2.5, 1.6, 0.4], true);
        let taken = inventory.take_within(1.5, 0.55);
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].pos, 1.6);
        assert_eq!(positions(&inventory), vec![2.5, 0.4]);
    }

    #[test]
    fn a_drop_settles_across_the_belt_by_its_angle() {
        let item = Item::dropped((), 0, 0, true);
        assert!(close(item.side, -1.0 / 6.0));
        assert!(close(item.pos, 0.5 - 1.0 / 16.0));
        assert!(close(Item::dropped((), 0, 0, false).pos, 0.5 + 1.0 / 16.0));
    }

    #[test]
    fn the_client_foresees_what_the_server_does() {
        let speed = chain_speed(128.0, Dir::South);
        let mut inventory = belt(&[3.2, 2.5, 0.5], true);
        let start = positions(&inventory);
        for ticks in 1..=40 {
            inventory.tick(speed, 5, || Ending::Blocked);
            let drawn = foresee(&start, &[false; 3], speed, ticks as f32, 5, Ending::Blocked.margin());
            for (a, b) in drawn.iter().zip(positions(&inventory)) {
                assert!((a - b).abs() < 1e-3, "tick {ticks}: {drawn:?} vs {:?}", positions(&inventory));
            }
        }
    }

    #[test]
    fn connecting_two_shafts_in_a_line() {
        let (facing, chain) = connect([0, 0, 0], [3, 0, 0], Axis::Z, Axis::Z).unwrap();
        assert_eq!(facing, Dir::East);
        assert_eq!(chain, vec![[0, 0, 0], [1, 0, 0], [2, 0, 0], [3, 0, 0]]);
        let (facing, chain) = connect([0, 0, 0], [0, 0, -2], Axis::X, Axis::X).unwrap();
        assert_eq!(facing, Dir::North);
        assert_eq!(chain.len(), 3);
        // Along the shafts' own axis, or with different axes, there is no belt.
        assert_eq!(connect([0, 0, 0], [3, 0, 0], Axis::X, Axis::X), Err(Refusal::NotInLine));
        assert_eq!(connect([0, 0, 0], [3, 0, 0], Axis::Z, Axis::Y), Err(Refusal::NotInLine));
        assert_eq!(connect([0, 0, 0], [3, 0, 1], Axis::Y, Axis::Y), Err(Refusal::NotInLine));
        assert_eq!(connect([0, 0, 0], [0, 0, 0], Axis::Z, Axis::Z), Err(Refusal::Same));
        // Closer than 20: a belt is at most 20 blocks long.
        assert!(connect([0, 0, 0], [19, 0, 0], Axis::Z, Axis::Z).is_ok());
        assert_eq!(connect([0, 0, 0], [20, 0, 0], Axis::Z, Axis::Z), Err(Refusal::TooFar));
        // Slopes and vertical belts come later.
        assert_eq!(connect([0, 0, 0], [2, 2, 0], Axis::Z, Axis::Z), Err(Refusal::NotHorizontal));
        assert_eq!(connect([0, 0, 0], [0, 3, 0], Axis::Z, Axis::Z), Err(Refusal::NotHorizontal));
    }

    #[test]
    fn a_belt_turns_as_one_and_drives_shafts_at_its_pulleys() {
        use crate::{Kind, Node, solve};
        use std::collections::BTreeMap;
        let belt = |part| Node::passive(Kind::Belt(Dir::East, part));
        let mut motor = Node::passive(Kind::Motor(Dir::South));
        motor.generated = 16.0;
        motor.capacity = 16384.0;
        let nodes: BTreeMap<Pos, Node> = [
            ([0, 0, -1], motor),
            ([0, 0, 0], belt(Part::Start)),
            ([1, 0, 0], belt(Part::Middle)),
            ([2, 0, 0], belt(Part::End)),
            // Beside the middle block: no pulley there.
            ([1, 0, 1], Node::passive(Kind::Shaft(Axis::Z))),
            // Beside the end's pulley.
            ([2, 0, 1], Node::passive(Kind::Shaft(Axis::Z))),
            // Another belt starting right after the end is a belt of its own.
            ([3, 0, 0], belt(Part::Start)),
            ([4, 0, 0], belt(Part::End)),
        ]
        .into_iter()
        .collect();
        let solution = solve(&nodes);
        assert!(solution.breaks.is_empty());
        let speeds = solution.speeds;
        for pos in [[0, 0, 0], [1, 0, 0], [2, 0, 0], [2, 0, 1]] {
            assert_eq!(speeds[&pos], 16.0, "{pos:?}");
        }
        for pos in [[1, 0, 1], [3, 0, 0], [4, 0, 0]] {
            assert_eq!(speeds[&pos], 0.0, "{pos:?}");
        }
    }

    /// A belt's surroundings for the tests: a processor above one segment and a block at the end.
    struct Around {
        ending: Ending,
        /// The segment with a processor above it and what it says of an arriving item.
        processor: Option<(i32, Processing)>,
        /// What the processor says once an item is held.
        holding: Processing,
        offered: Vec<i32>,
        takes: Insertion,
        taken: u32,
    }

    impl Around {
        fn new(ending: Ending) -> Self {
            Around {
                ending,
                processor: None,
                holding: Processing::Hold,
                offered: Vec::new(),
                takes: Insertion::Nothing,
                taken: 0,
            }
        }
    }

    impl Surroundings<u32> for Around {
        fn ending(&mut self) -> Ending {
            self.ending
        }

        fn held(&mut self, segment: i32, _: &Item<u32>) -> Option<Processing> {
            self.processor.filter(|p| p.0 == segment).map(|_| self.holding)
        }

        fn received(&mut self, segment: i32, _: &Item<u32>) -> Option<Processing> {
            self.offered.push(segment);
            self.processor.filter(|p| p.0 == segment).map(|p| p.1)
        }

        fn insert(&mut self, item: &mut Item<u32>) -> Insertion {
            if self.takes != Insertion::Nothing {
                self.taken += 1;
            }
            if self.takes == Insertion::Part {
                item.stack += 100;
            }
            self.takes
        }
    }

    #[test]
    fn an_item_stops_a_quarter_block_short_of_a_block_that_takes_it_and_is_handed_over() {
        assert_eq!(Ending::Insert.margin(), 0.25);
        assert_eq!(Ending::from_id(Ending::Insert.id()), Ending::Insert);
        let speed = chain_speed(256.0, Dir::South);
        // Refused: it waits at the margin, however long, and the next waits a block behind.
        let mut inventory = belt(&[1.5, 0.5], true);
        let mut env = Around::new(Ending::Insert);
        for _ in 0..200 {
            let moved = inventory.tick_with(speed, 3, &mut env);
            assert!(moved.ejected.is_empty());
        }
        let at = positions(&inventory);
        assert!(close(at[0], 3.0 - 0.25), "{at:?}");
        assert!(close(at[1], 3.0 - 0.25 - SPACING), "{at:?}");
        assert_eq!(env.taken, 0);
        // Taken whole: the front one is gone, and the one behind moves up and goes too.
        env.takes = Insertion::All;
        let mut reordered = false;
        for _ in 0..200 {
            reordered |= inventory.tick_with(speed, 3, &mut env).reordered;
        }
        assert!(reordered);
        assert!(inventory.items.is_empty());
        assert_eq!(env.taken, 2);
    }

    #[test]
    fn a_part_taken_leaves_the_rest_waiting() {
        let speed = chain_speed(256.0, Dir::South);
        let mut inventory = belt(&[2.7], true);
        let mut env = Around::new(Ending::Insert);
        env.takes = Insertion::Part;
        let moved = inventory.tick_with(speed, 3, &mut env);
        assert!(moved.reordered);
        // The glue changed the stack to what was left, and it stays on the belt.
        assert_eq!(inventory.items.len(), 1);
        assert_eq!(inventory.items[0].stack, 100);
    }

    #[test]
    fn a_processor_holds_an_item_in_the_middle_of_its_block_and_lets_it_go() {
        let speed = chain_speed(64.0, Dir::South);
        // The processor above segment 1.
        let mut env = Around::new(Ending::Eject);
        env.processor = Some((1, Processing::Hold));
        let mut inventory = belt(&[2.0, 0.5], true);
        // The first item is past the processor, the second comes to it.
        let mut caught = false;
        for _ in 0..60 {
            let moved = inventory.tick_with(speed, 14, &mut env);
            if inventory.items[1].locked {
                caught = true;
                assert!(moved.reordered);
                break;
            }
        }
        assert!(caught, "the processor never caught the item");
        assert!(close(inventory.items[1].pos, 1.5 + 1.0 / 512.0), "{}", inventory.items[1].pos);
        // Held: it stays however long, and the one in front keeps going.
        for _ in 0..30 {
            inventory.tick_with(speed, 14, &mut env);
        }
        assert!(inventory.items[1].locked);
        assert!(close(inventory.items[1].pos, 1.5 + 1.0 / 512.0));
        assert!(inventory.items[0].pos > 3.0);
        // Let go, it runs on and is not offered to the same processor again.
        env.holding = Processing::Pass;
        env.offered.clear();
        let moved = inventory.tick_with(speed, 14, &mut env);
        assert!(moved.reordered);
        assert!(!inventory.items[1].locked);
        for _ in 0..40 {
            inventory.tick_with(speed, 14, &mut env);
        }
        assert!(inventory.items[1].pos > 2.5);
        assert!(!env.offered.contains(&1), "{:?}", env.offered);
    }

    #[test]
    fn a_held_item_whose_processor_is_gone_is_let_go() {
        let speed = chain_speed(64.0, Dir::South);
        let mut inventory = belt(&[1.502], true);
        inventory.items[0].locked = true;
        let mut env = Around::new(Ending::Eject);
        let moved = inventory.tick_with(speed, 6, &mut env);
        assert!(moved.reordered);
        assert!(!inventory.items[0].locked);
    }

    #[test]
    fn a_processor_that_takes_the_item_off_empties_the_belt_of_it() {
        let speed = chain_speed(64.0, Dir::South);
        let mut inventory = belt(&[1.45], true);
        let mut env = Around::new(Ending::Eject);
        env.processor = Some((1, Processing::Remove));
        let moved = inventory.tick_with(speed, 6, &mut env);
        assert!(moved.reordered);
        assert!(inventory.items.is_empty());
    }

    #[test]
    fn items_on_a_belt_running_the_other_way_are_caught_at_the_segment_they_reach() {
        let speed = chain_speed(64.0, Dir::North);
        assert!(speed < 0.0);
        let mut env = Around::new(Ending::Eject);
        env.processor = Some((1, Processing::Hold));
        let mut inventory = belt(&[2.45], false);
        for _ in 0..40 {
            inventory.tick_with(speed, 6, &mut env);
            if inventory.items[0].locked {
                break;
            }
        }
        assert!(inventory.items[0].locked);
        assert!(close(inventory.items[0].pos, 1.5 - 1.0 / 512.0), "{}", inventory.items[0].pos);
    }

    #[test]
    fn a_stack_behind_a_held_one_queues_a_block_back() {
        let speed = chain_speed(64.0, Dir::South);
        let mut env = Around::new(Ending::Eject);
        env.processor = Some((2, Processing::Hold));
        let mut inventory = belt(&[1.8, 0.5], true);
        for _ in 0..100 {
            inventory.tick_with(speed, 8, &mut env);
        }
        let at = positions(&inventory);
        assert!(inventory.items[0].locked);
        assert!(close(at[0], 2.5 + 1.0 / 512.0), "{at:?}");
        assert!(close(at[1], at[0] - SPACING), "{at:?}");
    }

    #[test]
    fn the_client_leaves_a_held_item_where_it_is() {
        let speed = chain_speed(128.0, Dir::South);
        let drawn = foresee(&[2.0, 1.0], &[true, false], speed, 40.0, 8, 0.0);
        assert_eq!(drawn[0], 2.0);
        // The one behind runs up to a block short of it and stops.
        assert!(close(drawn[1], 1.0), "{drawn:?}");
        let free = foresee(&[2.0, 0.2], &[false, false], speed, 40.0, 8, 0.0);
        assert!(free[0] > 7.9 && free[1] < free[0] - SPACING + 1e-3, "{free:?}");
    }

    #[test]
    fn a_stack_handed_on_lands_as_create_s_try_inserting_from_side_puts_it() {
        // In line, from a belt behind: a quarter block short of the start (.26 of a block).
        let e = entry(0, 0.1, Dir::South, Dir::South, 0.05, true, true);
        assert!(close(e.pos, -0.26));
        assert_eq!(e.side, 0.05);
        // Moving the other way round the belt, the stack starts past the far end of the block.
        let e = entry(3, -0.1, Dir::North, Dir::North, 0.0, true, true);
        assert!(close(e.pos, 4.26));
        // Not from a belt, or nothing behind: right at the edge.
        let e = entry(2, 0.1, Dir::South, Dir::South, 0.0, false, true);
        assert!(close(e.pos, 2.0));
        let e = entry(2, 0.1, Dir::South, Dir::South, 0.0, true, false);
        assert!(close(e.pos, 2.0));
        // From the side: onto the middle of the block, on the near edge. Coming east lands on the
        // west edge (x is flipped), coming south on the north edge.
        let e = entry(1, 0.1, Dir::South, Dir::East, 0.0, true, false);
        assert!(close(e.pos, 1.5 - 1.0 / 16.0));
        assert!(close(e.side, -0.675));
        let e = entry(1, 0.1, Dir::East, Dir::South, 0.0, true, false);
        assert!(close(e.side, 0.675));
        let e = entry(1, -0.1, Dir::East, Dir::South, 0.0, true, false);
        assert!(close(e.pos, 1.5 + 1.0 / 16.0));
        // From above: dropped in the middle with its own side offset.
        let e = entry(1, 0.1, Dir::South, Dir::Up, 0.2, false, false);
        assert!(close(e.pos, 1.5 - 1.0 / 16.0));
        assert!(close(e.side, 0.2));
    }

    #[test]
    fn a_belt_takes_nothing_from_the_side_it_moves_towards_and_spaces_what_comes() {
        let mut inventory = belt(&[], true);
        // A belt moving south can't be fed from the south: that would be coming from ahead.
        assert!(!inventory.can_insert_at_from_side(1, Dir::North, Dir::South));
        assert!(inventory.can_insert_at_from_side(1, Dir::South, Dir::South));
        assert!(inventory.can_insert_at_from_side(1, Dir::East, Dir::South));
        let mut item = Item::dropped(1u32, 1, 180, true);
        item.inserted_from = Dir::South;
        item.pos = 0.9;
        inventory.insert(item);
        // In line: the last one is too close until it is past the segment's start plus a block.
        assert!(!inventory.can_insert_at_from_side(1, Dir::South, Dir::South));
        inventory.items[0].pos = 2.1;
        assert!(inventory.can_insert_at_from_side(1, Dir::South, Dir::South));
        // Another side is not held up by it.
        assert!(inventory.can_insert_at_from_side(1, Dir::East, Dir::South));
    }

    #[test]
    fn parts_and_chain_steps() {
        assert_eq!(part_at(0, 4, true), Part::Start);
        assert_eq!(part_at(3, 4, true), Part::End);
        assert_eq!(part_at(1, 4, false), Part::Middle);
        assert_eq!(part_at(2, 4, true), Part::Pulley);
        assert_eq!(next_segment([0, 0, 0], Dir::East, Part::Start, true), Some([1, 0, 0]));
        assert_eq!(next_segment([0, 0, 0], Dir::East, Part::Start, false), None);
        assert_eq!(next_segment([0, 0, 0], Dir::East, Part::End, true), None);
        assert_eq!(segment_position([5, 1, 5], Dir::North, 3), [5, 1, 2]);
        for part in Part::ALL {
            assert_eq!(Part::from_name(part.name()), Some(part));
        }
    }
}
