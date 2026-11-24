//! Clicking blocks on a moving structure. No game types.
//!
//! The decisions of Create's `ContraptionHandlerClient.rayTraceContraption`,
//! `AbstractContraptionEntity.handlePlayerInteraction` and the `MovingInteractionBehaviour`s of
//! `AllInteractionBehaviours` (lever, wooden doors, wooden trapdoors, fence gates) and of the seat, the
//! Deployer and the Contraption Controls, as plain functions over a structure's blocks:
//!
//! - [`ray_trace`]: the ray in the structure's local frame against each block's shape; the nearest
//!   block, face and point (Create clips each block's `getShape` along the cells of the ray, which finds
//!   the same nearest hit).
//! - [`interaction_of`]: which behaviour a block has (`MovingInteractionBehaviour.REGISTRY`).
//! - [`simple_toggle`]: `SimpleBlockMovingInteraction` and its four subclasses: the new state of the
//!   block (and of the other half of a door), the sound, whether the colliders are rebuilt.
//! - [`toggle_controls`]: `ContraptionControlsMovingInteraction`'s list of disabled actors.
//! - [`swap_hand`]: `DeployerMovingInteraction`'s change of the player's item with the Deployer's.
//!
//! The five Train Doors (`SlidingDoorBlock`, `DoorMovingInteraction`: `train_doors::click`) and the Conductor's Blaze
//! Burner (`ConductorBlockInteractionBehavior`: `schedule_hand::block_click`) are routed here too; their decisions are
//! in `rules::trains`.
//!
//! Known differences from Create: the Depot has no interaction here; the click's `Fail` swing is not suppressed.

use super::collider_obb::Aabb;
use super::contraption::State;
use crate::{Dir, Pos};

type Vec3 = [f64; 3];

// --- ContraptionHandlerClient.rayTraceContraption ----------------------------------------------------

/// A block of the structure as [`ray_trace`] takes it: its local position and the boxes of its shape
/// (`BlockState.getShape`), relative to the block.
#[derive(Clone, Copy, Debug)]
pub struct ClickBlock<'a> {
    pub local: Pos,
    pub boxes: &'a [Aabb],
}

/// Where the ray met the structure (`BlockHitResult`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub local: Pos,
    /// The face the ray entered through (`getDirection`).
    pub face: Dir,
    /// In the structure's local frame.
    pub point: Vec3,
    /// How far along the ray from `from` to `to`, 0 to 1.
    pub t: f64,
}

/// Where the segment `from`..`to` first enters a box at `origin` (its block's corner): the time, and
/// the face it came through. A segment that starts inside enters at 0 through the face that looks
/// back along it (Create's `clip` of a point inside answers the nearest face likewise).
fn clip(b: &Aabb, origin: Vec3, from: Vec3, to: Vec3) -> Option<(f64, Dir)> {
    let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let (mut t_min, mut t_max) = (0.0f64, 1.0f64);
    let mut enter: Option<(usize, bool)> = None;
    for i in 0..3 {
        let (lo, hi) = (origin[i] + b.min[i], origin[i] + b.max[i]);
        if d[i].abs() < 1.0E-12 {
            if from[i] < lo || from[i] > hi {
                return None;
            }
            continue;
        }
        let (t0, t1) = ((lo - from[i]) / d[i], (hi - from[i]) / d[i]);
        // Entering through the low face when moving up the axis, the high face when moving down.
        let (near, far, low_face) = if t0 < t1 { (t0, t1, true) } else { (t1, t0, false) };
        if near > t_min {
            t_min = near;
            enter = Some((i, low_face));
        }
        t_max = t_max.min(far);
        if t_min > t_max {
            return None;
        }
    }
    let face = match enter {
        Some((axis, low)) => face_of(axis, low),
        None => {
            // Inside: the face the segment points away from.
            let axis = (0..3).max_by(|a, b| d[*a].abs().total_cmp(&d[*b].abs())).unwrap_or(1);
            face_of(axis, d[axis] > 0.0)
        }
    };
    Some((t_min, face))
}

fn face_of(axis: usize, low: bool) -> Dir {
    match (axis, low) {
        (0, true) => Dir::West,
        (0, false) => Dir::East,
        (1, true) => Dir::Down,
        (1, false) => Dir::Up,
        (_, true) => Dir::North,
        (_, false) => Dir::South,
    }
}

/// `rayTraceContraption`: the nearest block of `blocks` whose shape the segment `from`..`to` (the
/// player's ray in the structure's local frame) enters. A block with no boxes is not hit.
pub fn ray_trace<'a>(blocks: impl IntoIterator<Item = ClickBlock<'a>>, from: Vec3, to: Vec3) -> Option<Hit> {
    let lo = [0, 1, 2].map(|i| from[i].min(to[i]) - 1.0);
    let hi = [0, 1, 2].map(|i| from[i].max(to[i]) + 1.0);
    let mut best: Option<Hit> = None;
    for block in blocks {
        let origin = block.local.map(f64::from);
        // A block's shape is inside its cell (a few shapes overhang a little): skip the far ones.
        if (0..3).any(|i| origin[i] + 1.5 < lo[i] || origin[i] - 0.5 > hi[i]) {
            continue;
        }
        for b in block.boxes {
            let Some((t, face)) = clip(b, origin, from, to) else { continue };
            if best.is_none_or(|h| t < h.t) {
                let point = [0, 1, 2].map(|i| from[i] + (to[i] - from[i]) * t);
                best = Some(Hit { local: block.local, face, point, t });
            }
        }
    }
    best
}

// --- which behaviour a block has -----------------------------------------------------------------------

/// `MovingInteractionBehaviour` of a block (`AllInteractionBehaviours`, and the ones Create's blocks
/// register for themselves).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interaction {
    Lever,
    Door,
    Trapdoor,
    /// `FenceGateBlock`s: Create registers `TrapdoorMovingInteraction` for them (the fence gate class
    /// beside it is unused).
    FenceGate,
    Seat,
    Deployer,
    Controls,
    /// `ControlsInteractionBehaviour`: the Train Controls of a carriage start a drive.
    TrainControls,
    /// `DoorMovingInteraction` on a `SlidingDoorBlock` (`trains::train_doors::click`).
    SlidingDoor,
    /// `ConductorBlockInteractionBehavior.BlazeBurner`: the burner of a carriage in front of Train Controls
    /// (`trains::schedule_hand::block_click`).
    Conductor,
}

/// `BlockTags.WOODEN_DOORS` and `WOODEN_TRAPDOORS`: every door of wood, not iron or copper.
fn wooden(path: &str, suffix: &str) -> bool {
    path.ends_with(suffix) && !path.contains("iron") && !path.contains("copper")
}

/// The behaviour of a block, by name.
pub fn interaction_of(name: &str) -> Option<Interaction> {
    let (namespace, path) = name.split_once(':').unwrap_or(("minecraft", name));
    match namespace {
        "minecraft" => {
            if path == "lever" {
                Some(Interaction::Lever)
            } else if wooden(path, "_trapdoor") {
                Some(Interaction::Trapdoor)
            } else if wooden(path, "_door") {
                Some(Interaction::Door)
            } else if path.ends_with("_fence_gate") {
                Some(Interaction::FenceGate)
            } else {
                None
            }
        }
        "create" => match path {
            "deployer" => Some(Interaction::Deployer),
            "contraption_controls" => Some(Interaction::Controls),
            "controls" => Some(Interaction::TrainControls),
            "blaze_burner" => Some(Interaction::Conductor),
            _ if super::trains::train_doors::is_sliding_door(name) => Some(Interaction::SlidingDoor),
            p if p.ends_with("_seat") => Some(Interaction::Seat),
            _ => None,
        },
        _ => None,
    }
}

// --- SimpleBlockMovingInteraction ----------------------------------------------------------------------

/// A sound to play where the structure is: the event, the volume and the pitch.
#[derive(Clone, Debug, PartialEq)]
pub struct Sound {
    pub event: &'static str,
    pub volume: f32,
    pub pitch: f32,
}

/// What a lever, door, trapdoor or fence gate click does.
#[derive(Clone, Debug, PartialEq)]
pub struct Toggle {
    /// The blocks that change: local position and new state (the clicked block first).
    pub changes: Vec<(Pos, State)>,
    pub sound: Sound,
    /// `updateColliders`: the structure's colliders are rebuilt (a door, trapdoor or gate).
    pub colliders: bool,
}

/// `state.cycle(property)` for a boolean property.
fn cycle(state: &State, key: &str) -> State {
    let mut out = state.clone();
    out.set(key, if state.flag(key) { "false" } else { "true" });
    out
}

/// `handle` of the behaviour for `state` at `local`. `other` is the block at the other half of a
/// door (`pos.above()` for the lower half, `pos.below()` for the upper) and `rand` a number in 0..1
/// for the pitch (`random.nextFloat() * 0.1F + 0.9F`). `None` when nothing changes
/// (`handlePlayerInteraction` returns false).
pub fn simple_toggle(
    kind: Interaction,
    local: Pos,
    state: &State,
    other: Option<&State>,
    rand: f32,
) -> Option<Toggle> {
    let pitch = rand * 0.1 + 0.9;
    match kind {
        Interaction::Lever => {
            if !state.has("powered") {
                return None;
            }
            // `LeverMovingInteraction`: the sound's pitch is by the state before the click.
            let pitch = if state.flag("powered") { 0.5 } else { 0.6 };
            Some(Toggle {
                changes: vec![(local, cycle(state, "powered"))],
                sound: Sound { event: "minecraft:block.lever.click", volume: 0.3, pitch },
                colliders: false,
            })
        }
        Interaction::Door => {
            if !state.has("open") {
                return None;
            }
            let lower = state.get("half") != Some("upper");
            let mut changes = vec![(local, cycle(state, "open"))];
            if let Some(other) = other.filter(|o| o.has("open")) {
                let at = [local[0], local[1] + if lower { 1 } else { -1 }, local[2]];
                changes.push((at, cycle(other, "open")));
            }
            let event = if state.flag("open") {
                "minecraft:block.wooden_door.close"
            } else {
                "minecraft:block.wooden_door.open"
            };
            Some(Toggle { changes, sound: Sound { event, volume: 0.3, pitch }, colliders: true })
        }
        Interaction::Trapdoor | Interaction::FenceGate => {
            if !state.has("open") {
                return None;
            }
            // Create registers the trapdoor's behaviour for fence gates too.
            let event = if state.flag("open") {
                "minecraft:block.wooden_trapdoor.close"
            } else {
                "minecraft:block.wooden_trapdoor.open"
            };
            Some(Toggle {
                changes: vec![(local, cycle(state, "open"))],
                sound: Sound { event, volume: 0.3, pitch },
                colliders: true,
            })
        }
        Interaction::Seat
        | Interaction::Deployer
        | Interaction::Controls
        | Interaction::TrainControls
        | Interaction::SlidingDoor
        | Interaction::Conductor => None,
    }
}

// --- AbstractContraptionEntity.handlePlayerInteraction ---------------------------------------------------------

/// What the click on a block does first: `handlePlayerInteraction`'s choice between sitting down
/// and the block's own behaviour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// The player sits on the seat.
    Sit,
    /// The block's behaviour.
    Behaviour(Interaction),
    /// `contraption.storage.handlePlayerStorageInteraction`: a chest on a structure. Not here.
    Storage,
}

/// `indexOfSeat == -1 || wrench`: a seat is sat on, unless the player holds the Wrench (then the
/// seat's own behaviour, which does nothing). Whether the player may sit (not already riding, no
/// player on the seat) is for the caller.
pub fn route(name: &str, holding_wrench: bool) -> Route {
    match interaction_of(name) {
        Some(Interaction::Seat) if !holding_wrench => Route::Sit,
        Some(kind) => Route::Behaviour(kind),
        None => Route::Storage,
    }
}

// --- ContraptionControlsMovingInteraction ----------------------------------------------------------------

/// `ContraptionControlsMovingInteraction.handlePlayerInteraction` on the server: the list of
/// disabled filters (`Contraption.disabledActors`; an empty string is the empty filter, every actor)
/// after a click on Controls with `filter`. `actors` are the block names of the actors that can be
/// disabled (`canBeDisabledVia`). Returns whether the click disabled (`true`) or enabled (`false`)
/// the filter; the list is changed.
pub fn toggle_controls(disabled: &mut Vec<String>, filter: &str, actors: &[&str]) -> bool {
    let mut disable = true;
    let mut invert = false;
    let mut i = 0;
    while i < disabled.len() {
        let present = disabled[i].clone();
        let same = present == filter;
        if present.is_empty() {
            disabled.remove(i);
            disable = false;
            if !same {
                invert = true;
            }
            continue;
        }
        if !same {
            i += 1;
            continue;
        }
        disabled.remove(i);
        disable = false;
        break;
    }
    if invert {
        for name in actors {
            if *name == filter || type_disabled(disabled, name) {
                continue;
            }
            disabled.push((*name).to_owned());
        }
    }
    if filter.is_empty() {
        disabled.clear();
    }
    if disable {
        disabled.push(filter.to_owned());
    }
    disable
}

/// `Contraption.isActorTypeDisabled`.
fn type_disabled(disabled: &[String], name: &str) -> bool {
    !name.is_empty() && disabled.iter().any(|d| !d.is_empty() && d == name)
}

// --- the client's cancel (ContraptionHandlerClient) ------------------------------------------------------------

/// `getRayInputs`'s reach: the player's `blockInteractionRange`, cut short at the real block the crosshair is
/// on (`mc.hitResult`'s distance), if there is one.
pub fn ray_reach(reach: f64, aimed: Option<f64>) -> f64 {
    aimed.map_or(reach, |a| a.min(reach))
}

/// Whether a structure's block the ray met at `structure` blocks from the eye takes the click, with the real
/// block aimed at `aimed` blocks away (none when it is the air). The ray is cut at the real block, so a
/// structure block at the same distance takes it. A little slack for the float error of the two frames.
pub fn structure_takes_click(structure: f64, aimed: Option<f64>) -> bool {
    aimed.is_none_or(|a| structure <= a + 1.0e-4)
}

// --- the elevator's Controls (ContraptionControlsMovingInteraction.elevatorInteraction) -----------------------------

/// `ElevatorControlsHandler.onScroll` and `ContraptionControlsMovement.tickFloorSelection`: the index of the
/// floor chosen after the wheel turned by `delta` (rounded away from zero, as the wheel's ~0.9 is not
/// floored to nothing), kept inside the list. `count` is the number of floors; none selects 0.
pub fn scroll_floor(index: usize, delta: f64, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    let step = if delta > 0.0 { delta.ceil() } else { delta.floor() } as i64;
    (index as i64 + step).clamp(0, count as i64 - 1) as usize
}

/// What a click on an elevator's Controls does with the chosen floor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloorClick {
    /// The cabin has no floors to choose from (`ec.namesList.isEmpty()`): only the sounds.
    NoFloors,
    /// The chosen floor already is the column's target (`currentTargetY == clientYTarget`).
    AlreadyTarget,
    /// `isTargetUnreachable`: the cabin's contact can't get that far.
    Unreachable,
    /// `ElevatorTargetFloorPacket`: the floor at this height is called.
    Call(i32),
}

/// The click, with `floors` the heights of the column's contacts from the lowest, `index` the chosen one,
/// `target` the column's target and `min`/`max` the heights the cabin can reach (`ElevatorTargetFloorPacket.handle`).
pub fn floor_click(floors: &[i32], index: usize, target: Option<i32>, min: i32, max: i32) -> FloorClick {
    let Some(&y) = floors.get(index.min(floors.len().saturating_sub(1))) else { return FloorClick::NoFloors };
    if target == Some(y) {
        FloorClick::AlreadyTarget
    } else if y < min || y > max {
        FloorClick::Unreachable
    } else {
        FloorClick::Call(y)
    }
}

// --- DeployerMovingInteraction ---------------------------------------------------------------------------------

/// What a click on a Deployer on a structure does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeployerClick {
    /// The Wrench: the mode turns (`PUNCH` and `USE`, the names Create saves).
    Mode(&'static str),
    /// Anything else: the player's held item goes into the Deployer's hand and the hand's comes back.
    Swap,
}

/// The wrench changes the mode (`mode == PUNCH ? USE : PUNCH`: anything that is not `PUNCH` becomes
/// `PUNCH`), another item swaps.
pub fn deployer_click(holding_wrench: bool, mode: &str) -> DeployerClick {
    if holding_wrench {
        DeployerClick::Mode(if mode == "PUNCH" { "USE" } else { "PUNCH" })
    } else {
        DeployerClick::Swap
    }
}

/// The two stacks after a swap: the player's hand gets the Deployer's item, the Deployer the
/// player's (`player.setItemInHand(deployerItem.copy())`, `fake.setItemInHand(heldStack.copy())`).
pub fn swap_hand<T: Clone>(held: Option<T>, deployer: Option<T>) -> (Option<T>, Option<T>) {
    (deployer, held)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube() -> [Aabb; 1] {
        [Aabb::new([0.0; 3], [1.0; 3])]
    }

    #[test]
    fn the_ray_hits_the_nearest_block_and_its_face() {
        let a = cube();
        let blocks = [
            ClickBlock { local: [0, 0, 0], boxes: &a },
            ClickBlock { local: [2, 0, 0], boxes: &a },
        ];
        let hit = ray_trace(blocks, [-1.0, 0.5, 0.5], [5.0, 0.5, 0.5]).unwrap();
        assert_eq!(hit.local, [0, 0, 0]);
        assert_eq!(hit.face, Dir::West);
        assert!((hit.point[0]).abs() < 1e-9);
        let hit = ray_trace(blocks, [5.0, 0.5, 0.5], [-1.0, 0.5, 0.5]).unwrap();
        assert_eq!(hit.local, [2, 0, 0]);
        assert_eq!(hit.face, Dir::East);
    }

    #[test]
    fn the_ray_misses_and_stops_short() {
        let a = cube();
        let blocks = [ClickBlock { local: [0, 0, 0], boxes: &a }];
        assert!(ray_trace(blocks, [-1.0, 2.5, 0.5], [5.0, 2.5, 0.5]).is_none());
        // Not long enough to arrive.
        assert!(ray_trace(blocks, [-3.0, 0.5, 0.5], [-1.5, 0.5, 0.5]).is_none());
        // A block with no shape is not there.
        assert!(ray_trace([ClickBlock { local: [0, 0, 0], boxes: &[] }], [-1.0, 0.5, 0.5], [2.0, 0.5, 0.5]).is_none());
    }

    #[test]
    fn a_lever_is_hit_through_its_small_box_only() {
        let lever = [Aabb::new([0.3, 0.0, 0.3], [0.7, 0.4, 0.7])];
        let blocks = [ClickBlock { local: [0, 0, 0], boxes: &lever }];
        assert!(ray_trace(blocks, [-1.0, 0.8, 0.5], [2.0, 0.8, 0.5]).is_none());
        let hit = ray_trace(blocks, [0.5, 2.0, 0.5], [0.5, -1.0, 0.5]).unwrap();
        assert_eq!(hit.face, Dir::Up);
        assert!((hit.point[1] - 0.4).abs() < 1e-9);
    }

    #[test]
    fn a_ray_starting_inside_hits_at_once() {
        let a = cube();
        let hit = ray_trace([ClickBlock { local: [0, 0, 0], boxes: &a }], [0.5, 0.5, 0.5], [3.0, 0.5, 0.5]).unwrap();
        assert_eq!(hit.t, 0.0);
        assert_eq!(hit.face, Dir::West);
    }

    #[test]
    fn behaviours_by_block() {
        assert_eq!(interaction_of("minecraft:lever"), Some(Interaction::Lever));
        assert_eq!(interaction_of("minecraft:oak_door"), Some(Interaction::Door));
        assert_eq!(interaction_of("minecraft:crimson_trapdoor"), Some(Interaction::Trapdoor));
        assert_eq!(interaction_of("minecraft:iron_door"), None);
        assert_eq!(interaction_of("minecraft:copper_trapdoor"), None);
        assert_eq!(interaction_of("minecraft:exposed_copper_door"), None);
        assert_eq!(interaction_of("minecraft:birch_fence_gate"), Some(Interaction::FenceGate));
        assert_eq!(interaction_of("create:red_seat"), Some(Interaction::Seat));
        assert_eq!(interaction_of("create:deployer"), Some(Interaction::Deployer));
        assert_eq!(interaction_of("create:contraption_controls"), Some(Interaction::Controls));
        assert_eq!(interaction_of("minecraft:stone"), None);
    }

    #[test]
    fn a_lever_toggles() {
        let lever = State::new("minecraft:lever").with("powered", "false");
        let t = simple_toggle(Interaction::Lever, [1, 2, 3], &lever, None, 0.5).unwrap();
        assert_eq!(t.changes, vec![([1, 2, 3], State::new("minecraft:lever").with("powered", "true"))]);
        assert_eq!(t.sound.pitch, 0.6);
        assert!(!t.colliders);
        let on = State::new("minecraft:lever").with("powered", "true");
        assert_eq!(simple_toggle(Interaction::Lever, [0; 3], &on, None, 0.0).unwrap().sound.pitch, 0.5);
        assert!(simple_toggle(Interaction::Lever, [0; 3], &State::new("minecraft:lever"), None, 0.0).is_none());
    }

    #[test]
    fn the_train_doors_and_the_conductor_s_burner_have_behaviours_of_their_own() {
        assert_eq!(interaction_of("create:train_door"), Some(Interaction::SlidingDoor));
        assert_eq!(interaction_of("create:andesite_door"), Some(Interaction::SlidingDoor));
        assert_eq!(interaction_of("create:blaze_burner"), Some(Interaction::Conductor));
        assert_eq!(route("create:train_door", false), Route::Behaviour(Interaction::SlidingDoor));
        // Not a simple toggle: the glue asks `train_doors::click`.
        let door = State::new("create:train_door").with("half", "lower").with("open", "false");
        assert!(simple_toggle(Interaction::SlidingDoor, [0, 0, 0], &door, None, 0.5).is_none());
    }

    #[test]
    fn a_door_opens_both_halves() {
        let lower = State::new("minecraft:oak_door").with("half", "lower").with("open", "false");
        let upper = State::new("minecraft:oak_door").with("half", "upper").with("open", "false");
        let t = simple_toggle(Interaction::Door, [0, 5, 0], &lower, Some(&upper), 0.0).unwrap();
        assert_eq!(t.changes.len(), 2);
        assert_eq!(t.changes[0].0, [0, 5, 0]);
        assert_eq!(t.changes[1].0, [0, 6, 0]);
        assert!(t.changes.iter().all(|(_, s)| s.flag("open")));
        assert_eq!(t.sound.event, "minecraft:block.wooden_door.open");
        assert!(t.colliders);
        // From the top half the other is below, and an open door closes.
        let open_upper = upper.clone().with("open", "true");
        let open_lower = lower.clone().with("open", "true");
        let t = simple_toggle(Interaction::Door, [0, 6, 0], &open_upper, Some(&open_lower), 0.0).unwrap();
        assert_eq!(t.changes[1].0, [0, 5, 0]);
        assert!(t.changes.iter().all(|(_, s)| !s.flag("open")));
        assert_eq!(t.sound.event, "minecraft:block.wooden_door.close");
    }

    #[test]
    fn trapdoors_and_gates_toggle() {
        let trap = State::new("minecraft:oak_trapdoor").with("open", "true");
        let t = simple_toggle(Interaction::Trapdoor, [0; 3], &trap, None, 1.0).unwrap();
        assert!(!t.changes[0].1.flag("open"));
        assert!(t.colliders);
        assert!((t.sound.pitch - 1.0).abs() < 1e-6);
        let gate = State::new("minecraft:oak_fence_gate").with("open", "false");
        assert!(simple_toggle(Interaction::FenceGate, [0; 3], &gate, None, 0.0).unwrap().changes[0].1.flag("open"));
    }

    #[test]
    fn a_seat_is_sat_on_unless_the_wrench_is_held() {
        assert_eq!(route("create:red_seat", false), Route::Sit);
        assert_eq!(route("create:red_seat", true), Route::Behaviour(Interaction::Seat));
        assert_eq!(route("minecraft:lever", false), Route::Behaviour(Interaction::Lever));
        assert_eq!(route("minecraft:chest", false), Route::Storage);
    }

    #[test]
    fn the_controls_disable_then_enable() {
        let actors = ["create:mechanical_drill", "create:mechanical_saw"];
        let mut list = Vec::new();
        assert!(toggle_controls(&mut list, "create:mechanical_drill", &actors));
        assert_eq!(list, ["create:mechanical_drill"]);
        assert!(!toggle_controls(&mut list, "create:mechanical_drill", &actors));
        assert!(list.is_empty());
    }

    #[test]
    fn empty_controls_disable_everything_and_a_filter_then_inverts() {
        let actors = ["create:mechanical_drill", "create:mechanical_saw"];
        let mut list = Vec::new();
        assert!(toggle_controls(&mut list, "", &actors));
        assert_eq!(list, [""]);
        // A drill's Controls while everything is off: the drill is on, the saw stays off.
        assert!(!toggle_controls(&mut list, "create:mechanical_drill", &actors));
        assert_eq!(list, ["create:mechanical_saw"]);
        // Empty Controls while one is off: everything off (Create clears the list and adds the empty filter).
        assert!(toggle_controls(&mut list, "", &actors));
        assert_eq!(list, [""]);
    }

    #[test]
    fn the_client_ray_stops_at_the_real_block_and_the_nearer_one_wins() {
        assert_eq!(ray_reach(4.5, None), 4.5);
        assert_eq!(ray_reach(4.5, Some(2.0)), 2.0);
        assert_eq!(ray_reach(4.5, Some(9.0)), 4.5);
        // A longer reach (the Extendo Grip) is the attribute's, not 4.5.
        assert_eq!(ray_reach(7.0, None), 7.0);
        assert!(structure_takes_click(2.0, Some(3.0)));
        assert!(structure_takes_click(3.0, Some(3.0)));
        assert!(!structure_takes_click(3.5, Some(3.0)));
        assert!(structure_takes_click(4.0, None));
    }

    #[test]
    fn the_wheel_moves_the_chosen_floor_and_keeps_it_in_the_list() {
        assert_eq!(scroll_floor(0, 1.0, 4), 1);
        // A touchpad's 0.9 is a step, not nothing.
        assert_eq!(scroll_floor(1, 0.9, 4), 2);
        assert_eq!(scroll_floor(2, -0.2, 4), 1);
        assert_eq!(scroll_floor(3, 5.0, 4), 3);
        assert_eq!(scroll_floor(0, -3.0, 4), 0);
        assert_eq!(scroll_floor(2, 1.0, 0), 0);
    }

    #[test]
    fn a_click_on_the_elevators_controls_calls_the_chosen_floor() {
        let floors = [64, 70, 80];
        assert_eq!(floor_click(&floors, 1, Some(64), 64, 80), FloorClick::Call(70));
        assert_eq!(floor_click(&floors, 1, Some(70), 64, 80), FloorClick::AlreadyTarget);
        assert_eq!(floor_click(&floors, 2, None, 64, 75), FloorClick::Unreachable);
        assert_eq!(floor_click(&floors, 0, None, 66, 80), FloorClick::Unreachable);
        assert_eq!(floor_click(&[], 0, None, 0, 10), FloorClick::NoFloors);
        // A stale index past the end takes the last floor.
        assert_eq!(floor_click(&floors, 9, None, 64, 80), FloorClick::Call(80));
    }

    #[test]
    fn the_deployer_wrench_turns_the_mode_and_other_items_swap() {
        assert_eq!(deployer_click(true, "PUNCH"), DeployerClick::Mode("USE"));
        assert_eq!(deployer_click(true, "USE"), DeployerClick::Mode("PUNCH"));
        assert_eq!(deployer_click(false, "USE"), DeployerClick::Swap);
        assert_eq!(swap_hand(Some(1), None), (None, Some(1)));
        assert_eq!(swap_hand(None, Some(2)), (Some(2), None));
    }
}
