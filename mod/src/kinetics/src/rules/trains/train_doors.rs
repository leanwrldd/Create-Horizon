//! Train doors: `DoorControl` and `DoorControlBehaviour` (content/decoration/slidingDoor/), the station's door controls
//! (`StationBlockEntity.doorControls`, `StationEditPacket`), `SlidingDoorMovementBehaviour` (the doors of a carriage open
//! at a station) and `DoorMovingInteraction` (a click on such a door in a carriage). No game types.
//!
//! The doors are Create's five `SlidingDoorBlock`s (`andesite_door`, `brass_door`, `copper_door`, `train_door`,
//! `framed_glass_door`): they all register the same movement behaviour and interaction in `BuilderTransformers.slidingDoor`.
//!
//! Differences from Create:
//! - `SlidingDoorBlockEntity` (the sliding animation, `VISIBLE`) isn't here: the port's doors are vanilla-style doors that
//!   swing, so the door's state is just `open`; the close sound that Create plays when the client-side animation has settled
//!   is played when the door closes.
//! - The elevator contact's `DoorControlBehaviour` (an elevator cabin's doors) is not wired (`getElevatorDoorControl`).
//! - Catnip's `NBTHelper.writeEnum` / `readEnum` are not in the Java tree: the enum is saved as its constant's name
//!   (`"ALL"`, `"NORTH"`, ...), and an unknown or missing value reads as `ALL`, the first constant.

use crate::rules::contraption::State;
use crate::{Dir, Pos};

/// `DoorControl`: which of a carriage's doors a station opens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DoorControl {
    #[default]
    All,
    North,
    East,
    South,
    West,
    None,
}

impl DoorControl {
    pub const ALL: [DoorControl; 6] =
        [DoorControl::All, DoorControl::North, DoorControl::East, DoorControl::South, DoorControl::West, DoorControl::None];

    /// `Lang.asId(name())`: `all`, `north`, ...
    pub fn id(self) -> &'static str {
        match self {
            DoorControl::All => "all",
            DoorControl::North => "north",
            DoorControl::East => "east",
            DoorControl::South => "south",
            DoorControl::West => "west",
            DoorControl::None => "none",
        }
    }

    /// The enum constant's name, as it is saved.
    pub fn constant(self) -> &'static str {
        match self {
            DoorControl::All => "ALL",
            DoorControl::North => "NORTH",
            DoorControl::East => "EAST",
            DoorControl::South => "SOUTH",
            DoorControl::West => "WEST",
            DoorControl::None => "NONE",
        }
    }

    /// `NBTHelper.readEnum`: the constant named, or the first one.
    pub fn read(saved: Option<&str>) -> DoorControl {
        saved.and_then(|s| DoorControl::ALL.into_iter().find(|c| c.constant() == s)).unwrap_or_default()
    }

    /// The word a command gives (`all`, `north`, ..., any case).
    pub fn parse(word: &str) -> Option<DoorControl> {
        let word = word.trim();
        DoorControl::ALL.into_iter().find(|c| c.id().eq_ignore_ascii_case(word))
    }

    /// `create.contraption.door_control.<id>` (the long name on the screen).
    pub fn long(self) -> &'static str {
        match self {
            DoorControl::All => "Open All Doors",
            DoorControl::North => "North Side Only",
            DoorControl::East => "East Side Only",
            DoorControl::South => "South Side Only",
            DoorControl::West => "West Side Only",
            DoorControl::None => "Keep Doors Closed",
        }
    }

    /// `create.contraption.door_control.<id>.short`.
    pub fn short(self) -> &'static str {
        match self {
            DoorControl::All => "Open All",
            DoorControl::North => "North",
            DoorControl::East => "East",
            DoorControl::South => "South",
            DoorControl::West => "West",
            DoorControl::None => "None",
        }
    }

    /// `DoorControl.matches(doorDirection)`.
    pub fn matches(self, door: Dir) -> bool {
        match self {
            DoorControl::All => true,
            DoorControl::North => door == Dir::North,
            DoorControl::East => door == Dir::East,
            DoorControl::South => door == Dir::South,
            DoorControl::West => door == Dir::West,
            DoorControl::None => false,
        }
    }
}

/// The five `SlidingDoorBlock`s.
pub const SLIDING_DOORS: [&str; 5] =
    ["create:andesite_door", "create:brass_door", "create:copper_door", "create:train_door", "create:framed_glass_door"];

pub fn is_sliding_door(name: &str) -> bool {
    SLIDING_DOORS.contains(&name)
}

// --- SlidingDoorMovementBehaviour ----------------------------------------------------------------------------------

/// `Direction.getNearest` (ties go to the first of down, up, north, south, west, east; north when all are zero).
pub fn nearest(v: [f64; 3]) -> Dir {
    crate::rules::portable_interface::nearest_direction(v)
}

/// `Contraption.bounds`: the union of the unit cells of the anchor and every block (`bounds.minmax(new AABB(localPos))`).
/// Its centre is what `getDoorFacing` measures from.
pub fn bounds_centre(blocks: impl IntoIterator<Item = Pos>) -> [f64; 3] {
    let mut lo = [0, 0, 0];
    let mut hi = [1, 1, 1];
    for p in blocks {
        for i in 0..3 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i] + 1);
        }
    }
    [0, 1, 2].map(|i| (f64::from(lo[i]) + f64::from(hi[i])) / 2.0)
}

/// `SlidingDoorMovementBehaviour.getDoorFacing`: which way the door faces in the world, from its state's `facing`,
/// where it sits in the structure (`centre` is [`bounds_centre`]) and how the structure is turned (`rotate` turns a
/// direction vector as `context.rotation` does).
pub fn door_facing(state_facing: Dir, local: Pos, centre: [f64; 3], rotate: &dyn Fn([f64; 3]) -> [f64; 3]) -> Dir {
    // `Direction.get(AxisDirection.POSITIVE, axis)`.
    let mut original = match state_facing.axis() {
        crate::Axis::X => Dir::East,
        crate::Axis::Y => Dir::Up,
        crate::Axis::Z => Dir::South,
    };
    let n = state_facing.normal();
    let diff = [0, 1, 2].map(|i| f64::from(local[i]) + 0.5 + f64::from(n[i]) * -0.45 - centre[i]);
    let along = match original.axis() {
        crate::Axis::X => diff[0],
        crate::Axis::Y => diff[1],
        crate::Axis::Z => diff[2],
    };
    if along < 0.0 {
        original = original.opposite();
    }
    let o = original.normal();
    nearest(rotate([f64::from(o[0]), f64::from(o[1]), f64::from(o[2])]))
}

/// A train this slow (blocks a tick) lets its doors open (`context.motion.length() < 1 / 128f`).
pub const OPEN_BELOW_SPEED: f64 = 1.0 / 128.0;

/// `SlidingDoorMovementBehaviour.shouldOpen` for a door on a carriage. `speed` is `|train.speed|` while the
/// carriage moves (`shouldActorTrigger` scales the motion to it), 0 when it doesn't; `disabled` is
/// `context.disabled`; `control` is the mode of the station the train is at (`None` when the train is at none, or
/// its block entity is gone: the door stays shut).
pub fn should_open(disabled: bool, speed: f64, control: Option<DoorControl>, door: Dir) -> bool {
    if disabled || speed >= OPEN_BELOW_SPEED {
        return false;
    }
    match control {
        None => false,
        Some(DoorControl::All) => true,
        Some(DoorControl::None) => false,
        Some(mode) => mode.matches(door),
    }
}

/// What a door's actor remembers (`context.data.Open`, `context.firstMovement`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DoorData {
    pub open: Option<bool>,
    pub first_movement: bool,
}

impl Default for DoorData {
    fn default() -> Self {
        DoorData { open: None, first_movement: true }
    }
}

impl DoorData {
    /// `shouldUpdate(context, shouldOpen)`.
    pub fn should_update(&mut self, should_open: bool) -> bool {
        if self.first_movement && should_open {
            return false;
        }
        let Some(was_open) = self.open else {
            self.open = Some(should_open);
            return true;
        };
        self.open = Some(should_open);
        was_open != should_open
    }

    /// The end of the actor's tick (`context.firstMovement = false`).
    pub fn ticked(&mut self) {
        self.first_movement = false;
    }
}

/// `state.cycle(DoorBlock.OPEN)`.
fn cycle_open(state: &State) -> State {
    let mut out = state.clone();
    out.set("open", if state.flag("open") { "false" } else { "true" });
    out
}

/// What `tickOpen` does when a door has to change.
#[derive(Clone, Debug, PartialEq)]
pub struct Toggle {
    /// `toggleDoor`: the door's new state and its other half's, with local positions.
    pub changes: Vec<(Pos, State)>,
    /// The door in the world, one block from the carriage's door along its facing, is set to this (`db.setOpen(null, ..)`)
    /// when it is a door facing the same axis.
    pub world_door: (Dir, bool),
    /// `SoundEvents.IRON_DOOR_OPEN`, volume .125, played when opening; the close sound is played when it shuts.
    pub sound: &'static str,
}

/// `tickOpen`'s change for a door whose state is `state` at `local` (`other` is the block above a lower half or below an
/// upper half). `None` when nothing changes (`shouldUpdate` said no, the door is already as it should be, or the
/// block has no `open`).
pub fn tick_open(data: &mut DoorData, state: &State, local: Pos, other: Option<&State>, should_open: bool, facing: Dir) -> Option<Toggle> {
    if !data.should_update(should_open) {
        return None;
    }
    if state.flag("open") == should_open || !state.has("open") {
        return None;
    }
    let new = cycle_open(state);
    let lower = new.get("half") != Some("upper");
    let mut changes = vec![(local, new)];
    if let Some(other) = other.filter(|o| o.has("open")) {
        let at = [local[0], local[1] + if lower { 1 } else { -1 }, local[2]];
        changes.push((at, cycle_open(other)));
    }
    Some(Toggle {
        changes,
        world_door: (facing, should_open),
        sound: if should_open { "minecraft:block.iron_door.open" } else { "minecraft:block.iron_door.close" },
    })
}

/// Volume of the door's open and close sounds (`.125f`).
pub const DOOR_SOUND_VOLUME: f32 = 0.125;

/// Whether a block in the world is a door that `tickOpen` may open: a door with `open` whose `facing` is along the same axis
/// (`inWorldDoorState.getOptionalValue(FACING).orElse(UP).getAxis() == facing.getAxis()`).
pub fn world_door_matches(state: &State, facing: Dir) -> bool {
    state.has("open") && state.dir("facing").unwrap_or(Dir::Up).axis() == facing.axis()
}

// --- DoorMovingInteraction ------------------------------------------------------------------------------------------

/// `DoorMovingInteraction.handle` for a sliding door clicked by a player: the new state of the door and its other half, the
/// double door beside it (`isDoubleDoor`: toggled by a second click, as Java's `handlePlayerInteraction(null, ..)` does) and
/// the sound (`IRON_DOOR_OPEN` when it opens, none when it closes).
#[derive(Clone, Debug, PartialEq)]
pub struct Click {
    pub changes: Vec<(Pos, State)>,
    pub sound: Option<(&'static str, f32)>,
    /// The position of the second leaf of a double door that was toggled too, if there is one.
    pub double: Option<Pos>,
}

/// The block to the side that may be the other leaf of a double door
/// (`pos.relative(hinge == LEFT ? facing.getClockWise() : facing.getCounterClockWise())`).
pub fn double_door_pos(local: Pos, state: &State) -> Option<Pos> {
    let facing = state.dir("facing")?;
    let left = state.get("hinge") == Some("left");
    let side = if left { facing.clockwise_about(crate::Axis::Y) } else { facing.clockwise_about(crate::Axis::Y).opposite() };
    let n = side.normal();
    Some([local[0] + n[0], local[1] + n[1], local[2] + n[2]])
}

/// `SlidingDoorBlock.isDoubleDoor(state, hinge, facing, otherDoor)` for the door as it is before the click's change was
/// applied to `other` (the other leaf is a door of the same block with the opposite hinge, the same facing, an
/// opposite `open` and the same half).
pub fn is_double_door(state: &State, other: &State) -> bool {
    other.name == state.name
        && other.get("hinge") != state.get("hinge")
        && other.get("facing") == state.get("facing")
        && other.flag("open") != state.flag("open")
        && other.get("half") == state.get("half")
}

/// `DoorMovingInteraction.handle`: `state` at `local`, `other` the other half. `pitch` is `random.nextFloat() * 0.1 + 0.9`.
/// `None` when the block isn't a door with `open`.
pub fn click(state: &State, local: Pos, other: Option<&State>, double: Option<&State>, pitch: f32) -> Option<Click> {
    if !state.has("open") {
        return None;
    }
    let opening = !state.flag("open");
    let lower = state.get("half") != Some("upper");
    let mut changes = Vec::new();
    if let Some(other) = other.filter(|o| o.has("open")) {
        let at = [local[0], local[1] + if lower { 1 } else { -1 }, local[2]];
        changes.push((at, cycle_open(other)));
    }
    let new = cycle_open(state);
    // `isDoubleDoor(currentState, ..)` is asked of the state after the cycle, as Java does.
    let double_at = double.filter(|d| is_double_door(&new, d)).and(double_door_pos(local, &new));
    changes.insert(0, (local, new));
    Some(Click { changes, sound: opening.then_some(("minecraft:block.iron_door.open", pitch)), double: double_at })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn door(half: &str, open: bool) -> State {
        State::new("create:train_door").with("facing", "north").with("half", half).with("hinge", "left").with("open", if open { "true" } else { "false" })
    }

    #[test]
    fn a_control_matches_the_doors_it_names() {
        assert!(DoorControl::All.matches(Dir::West));
        assert!(DoorControl::North.matches(Dir::North));
        assert!(!DoorControl::North.matches(Dir::South));
        assert!(!DoorControl::None.matches(Dir::North));
        assert!(!DoorControl::East.matches(Dir::Up));
    }

    #[test]
    fn a_control_is_saved_by_name_and_read_back() {
        for c in DoorControl::ALL {
            assert_eq!(DoorControl::read(Some(c.constant())), c);
            assert_eq!(DoorControl::parse(c.id()), Some(c));
        }
        assert_eq!(DoorControl::read(None), DoorControl::All);
        assert_eq!(DoorControl::read(Some("sideways")), DoorControl::All);
        assert_eq!(DoorControl::parse("NORTH"), Some(DoorControl::North));
        assert_eq!(DoorControl::parse("up"), None);
        assert_eq!(DoorControl::None.long(), "Keep Doors Closed");
        assert_eq!(DoorControl::All.short(), "Open All");
    }

    #[test]
    fn the_five_sliding_doors() {
        assert!(is_sliding_door("create:train_door"));
        assert!(is_sliding_door("create:framed_glass_door"));
        assert!(!is_sliding_door("minecraft:oak_door"));
    }

    #[test]
    fn the_bounds_include_the_anchor() {
        // Blocks only to the east: the anchor's own cell still counts.
        assert_eq!(bounds_centre([[1, 0, 0], [2, 0, 0]]), [1.5, 0.5, 0.5]);
        assert_eq!(bounds_centre([[-2, 0, 0], [3, 2, 0]]), [1.0, 1.5, 0.5]);
        assert_eq!(bounds_centre([]), [0.5, 0.5, 0.5]);
    }

    fn identity(v: [f64; 3]) -> [f64; 3] {
        v
    }

    #[test]
    fn a_door_faces_the_side_of_the_structure_it_sits_on() {
        // A carriage 5 long along x, centre x = 2.5 (cells 0..4). A door in the +z wall faces z: the door at z = 1 is south of
        // the centre line (z centre .5).
        let cells: Vec<Pos> = (0..5).map(|x| [x, 0, 0]).chain([[2, 0, 1]]).collect();
        let centre = bounds_centre(cells.iter().copied());
        assert_eq!(centre, [2.5, 0.5, 1.0]);
        // Its state faces north (the door's own axis is z): the door at z=1 lies on the +z side, `original` south.
        assert_eq!(door_facing(Dir::North, [2, 0, 1], centre, &identity), Dir::South);
        // A door at z=0 lies on the -z side of the centre z = 1.0.
        assert_eq!(door_facing(Dir::North, [2, 0, 0], centre, &identity), Dir::North);
        // A door in the end wall at x = 4, facing along x, is east.
        assert_eq!(door_facing(Dir::East, [4, 0, 0], centre, &identity), Dir::East);
        assert_eq!(door_facing(Dir::East, [0, 0, 0], centre, &identity), Dir::West);
    }

    #[test]
    fn a_turned_carriage_turns_its_doors() {
        let cells: Vec<Pos> = (0..5).map(|x| [x, 0, 0]).collect();
        let centre = bounds_centre(cells.iter().copied());
        // The carriage turned a quarter: the local south is the world's west.
        let quarter = |v: [f64; 3]| [v[2], v[1], -v[0]];
        // The door at x=0 is on the west side, `original` = west; local west turned by `quarter` is local (-1,0,0) to (0,0,1) = south.
        assert_eq!(door_facing(Dir::East, [0, 0, 0], centre, &quarter), Dir::South);
        assert_eq!(door_facing(Dir::East, [4, 0, 0], centre, &quarter), Dir::North);
    }

    #[test]
    fn doors_open_only_when_the_train_stands_at_a_station() {
        assert!(should_open(false, 0.0, Some(DoorControl::All), Dir::North));
        assert!(!should_open(false, 0.5, Some(DoorControl::All), Dir::North));
        assert!(!should_open(false, 1.0 / 128.0, Some(DoorControl::All), Dir::North));
        assert!(should_open(false, 0.007, Some(DoorControl::All), Dir::North));
        // Disabled by the controls, no station, or the station says none.
        assert!(!should_open(true, 0.0, Some(DoorControl::All), Dir::North));
        assert!(!should_open(false, 0.0, None, Dir::North));
        assert!(!should_open(false, 0.0, Some(DoorControl::None), Dir::North));
    }

    #[test]
    fn a_side_only_station_opens_only_that_side() {
        assert!(should_open(false, 0.0, Some(DoorControl::East), Dir::East));
        assert!(!should_open(false, 0.0, Some(DoorControl::East), Dir::West));
    }

    #[test]
    fn the_first_tick_does_not_open_a_door_but_does_not_stop_it_closing() {
        let mut d = DoorData::default();
        assert!(!d.should_update(true));
        assert_eq!(d.open, None);
        // Not opening on the first movement records the state.
        assert!(d.should_update(false));
        assert_eq!(d.open, Some(false));
        d.ticked();
        assert!(d.should_update(true));
        assert!(!d.should_update(true));
        assert!(d.should_update(false));
    }

    #[test]
    fn opening_a_door_opens_both_halves_and_the_world_door() {
        let mut data = DoorData { open: Some(false), first_movement: false };
        let lower = door("lower", false);
        let upper = door("upper", false);
        let t = tick_open(&mut data, &lower, [2, 0, 1], Some(&upper), true, Dir::South).expect("opens");
        assert_eq!(t.changes.len(), 2);
        assert_eq!(t.changes[0].0, [2, 0, 1]);
        assert!(t.changes[0].1.flag("open"));
        assert_eq!(t.changes[1].0, [2, 1, 1]);
        assert!(t.changes[1].1.flag("open"));
        assert_eq!(t.world_door, (Dir::South, true));
        assert_eq!(t.sound, "minecraft:block.iron_door.open");
    }

    #[test]
    fn the_upper_half_pairs_with_the_block_below() {
        let mut data = DoorData { open: Some(true), first_movement: false };
        let upper = door("upper", true);
        let lower = door("lower", true);
        let t = tick_open(&mut data, &upper, [2, 1, 1], Some(&lower), false, Dir::North).expect("closes");
        assert_eq!(t.changes[1].0, [2, 0, 1]);
        assert!(!t.changes[1].1.flag("open"));
        assert_eq!(t.sound, "minecraft:block.iron_door.close");
    }

    #[test]
    fn a_door_already_as_it_should_be_changes_nothing() {
        let mut data = DoorData { open: Some(false), first_movement: false };
        let lower = door("lower", true);
        // Wants to open, is open already (the other half toggled it).
        assert!(tick_open(&mut data, &lower, [0, 0, 0], None, true, Dir::North).is_none());
    }

    #[test]
    fn only_a_door_on_the_same_axis_opens_in_the_world() {
        let vanilla = State::new("minecraft:oak_door").with("facing", "east").with("open", "false");
        assert!(world_door_matches(&vanilla, Dir::West));
        assert!(!world_door_matches(&vanilla, Dir::North));
        assert!(!world_door_matches(&State::new("minecraft:stone"), Dir::West));
        // No facing: `UP`, whose axis is never horizontal.
        assert!(!world_door_matches(&State::new("x:y").with("open", "false"), Dir::West));
    }

    #[test]
    fn a_click_toggles_both_halves_and_sounds_only_when_opening() {
        let c = click(&door("lower", false), [0, 0, 0], Some(&door("upper", false)), None, 0.95).expect("door");
        assert_eq!(c.changes.len(), 2);
        assert!(c.changes.iter().all(|(_, s)| s.flag("open")));
        assert_eq!(c.sound, Some(("minecraft:block.iron_door.open", 0.95)));
        let c = click(&door("lower", true), [0, 0, 0], Some(&door("upper", true)), None, 0.95).expect("door");
        assert!(c.changes.iter().all(|(_, s)| !s.flag("open")));
        assert_eq!(c.sound, None);
        assert!(click(&State::new("minecraft:stone"), [0, 0, 0], None, None, 1.0).is_none());
    }

    #[test]
    fn a_double_door_is_clicked_too() {
        // Two leaves with opposite hinges and the same facing, both shut before the click: the click's new state is open, the
        // other leaf's is not, which is what `isDoubleDoor` asks.
        let clicked = door("lower", false);
        let leaf = State::new("create:train_door").with("facing", "north").with("half", "lower").with("hinge", "right").with("open", "false");
        let c = click(&clicked, [0, 0, 0], None, Some(&leaf), 1.0).expect("door");
        // Hinge left, facing north: the other leaf is `facing.getClockWise()` = east of it.
        assert_eq!(c.double, Some([1, 0, 0]));
        // A leaf with the same hinge is no double door.
        let same = State::new("create:train_door").with("facing", "north").with("half", "lower").with("hinge", "left").with("open", "false");
        assert_eq!(click(&clicked, [0, 0, 0], None, Some(&same), 1.0).and_then(|c| c.double), None);
    }
}
