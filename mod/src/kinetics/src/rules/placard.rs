//! The Placard's rules (`PlacardBlock`, `PlacardBlockEntity`, `PlacardRenderer`): a plate on a
//! wall, floor or ceiling that holds one item set by a click and gives a 19 tick pulse when
//! clicked with a matching item. No game types: the game glue is `machines/placard.rs`.
//!
//! Known differences from Create, for the README:
//! - Create's display link doesn't target a Placard either (nothing names it in `display_link`).
//! - A click with a stack that carries components can set the held item in Creative only: Horizon
//!   can take only plain stacks from a hand inside a player's hook.

use crate::Dir;

/// `PlacardBlock.useItemOn`: `pte.poweredTicks = 19`.
pub const POWERED_TICKS: i32 = 19;

/// `AttachFace`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attach {
    Floor,
    Wall,
    Ceiling,
}

impl Attach {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "floor" => Some(Self::Floor),
            "wall" => Some(Self::Wall),
            "ceiling" => Some(Self::Ceiling),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Floor => "floor",
            Self::Wall => "wall",
            Self::Ceiling => "ceiling",
        }
    }
}

/// `FaceAttachedHorizontalDirectionalBlock.getConnectedDirection`: the way the plate's front
/// points (away from what it is fixed to).
pub fn connected_direction(face: Attach, facing: Dir) -> Dir {
    match face {
        Attach::Ceiling => Dir::Down,
        Attach::Floor => Dir::Up,
        Attach::Wall => facing,
    }
}

/// `FaceAttachedHorizontalDirectionalBlock.getStateForPlacement` with `PlacardBlock`'s change:
/// the `face` and `facing` for a placement on the `clicked` face of a block by a player looking
/// towards `player_facing`. On a floor, vanilla gives the player's direction and Create turns it
/// round, so the plate faces the player.
pub fn placement(clicked: Dir, player_facing: Dir) -> (Attach, Dir) {
    match clicked {
        Dir::Up => (Attach::Floor, player_facing.opposite()),
        Dir::Down => (Attach::Ceiling, player_facing),
        wall => (Attach::Wall, wall),
    }
}

/// What a click with an item in the hand does (`PlacardBlock.useItemOn`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Click {
    /// Nothing happens (`FAIL`).
    Ignore,
    /// The item in the hand is set as the held item (one of it, the hand's stack shrinks).
    Place,
    /// The item matched: the plate gives its pulse (`CONFIRM`).
    Confirm,
    /// The item didn't match (`DENY`).
    Deny,
}

/// `PlacardBlock.useItemOn`, for a player not sneaking and on the server. `matches` is asked only
/// when the rest decides on it: the held item is a filter or the same item as the one in the hand.
pub fn click(
    may_build: bool,
    hand_empty: bool,
    held_empty: bool,
    powered: bool,
    matches: impl FnOnce() -> bool,
) -> Click {
    if may_build && !hand_empty && held_empty {
        return Click::Place;
    }
    if held_empty || hand_empty || powered {
        return Click::Ignore;
    }
    if matches() { Click::Confirm } else { Click::Deny }
}

/// `PlacardBlockEntity.tick`: counts `poweredTicks` down; the second answer is true on the tick it
/// reaches 0, when the block stops giving a signal.
pub fn tick(powered_ticks: i32) -> (i32, bool) {
    if powered_ticks <= 0 {
        return (0, false);
    }
    let next = powered_ticks - 1;
    (next, next == 0)
}

/// `PlacardBlock.getSignal` and `getDirectSignal`: 15 in every direction while powered, and into
/// the block it is fixed to for a direct signal. `towards_support` is whether the query asks
/// across the face on which the plate is fixed.
pub fn signal(powered: bool, direct: bool, towards_support: bool) -> i32 {
    if powered && (!direct || towards_support) { 15 } else { 0 }
}

/// How `PlacardRenderer` places the held item, in degrees and blocks about the block's centre:
/// the turn about y, then about x, a move forwards (+z after the turns) and the scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ItemPose {
    pub y_degrees: f32,
    pub x_degrees: f32,
    pub forward: f32,
    pub scale: f32,
}

/// `PlacardRenderer.renderSafe`. `facing_angle` is `AngleHelper.horizontalAngle(facing)`;
/// `block_item` stands for `BakedModel.isGui3d`.
pub fn item_pose(face: Attach, facing_angle: f32, block_item: bool) -> ItemPose {
    ItemPose {
        y_degrees: (if face == Attach::Ceiling { 180.0 } else { 0.0 }) + 180.0 + facing_angle,
        x_degrees: match face {
            Attach::Ceiling => -90.0,
            Attach::Floor => 90.0,
            Attach::Wall => 0.0,
        },
        forward: 4.5 / 16.0,
        scale: if block_item { 0.5 } else { 0.375 },
    }
}

/// `PlacardBlockEntity.spawnParticles`: where each of the ten dust particles goes, from ten
/// random offsets in -0.5..0.5 per axis (`VecHelper.offsetRandomly(ZERO, random, .5f)`). They are
/// flattened into the plane of the plate, put on a ring 0.45 from its centre and moved 0.45 back
/// to where the plate is fixed; the result is relative to the block's centre.
pub fn particle_offset(connected: Dir, mut offset: [f32; 3]) -> [f32; 3] {
    let [nx, ny, nz] = connected.normal();
    let normal = [nx as f32, ny as f32, nz as f32];
    for axis in 0..3 {
        if normal[axis] != 0.0 {
            offset[axis] = 0.0;
        }
    }
    let length = (offset[0] * offset[0] + offset[1] * offset[1] + offset[2] * offset[2]).sqrt();
    let scale = if length > 0.0 { 0.45 / length } else { 0.0 };
    [
        offset[0] * scale - normal[0] * 0.45,
        offset[1] * scale - normal[1] * 0.45,
        offset[2] * scale - normal[2] * 0.45,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_follows_the_face_clicked() {
        // On a wall the plate faces out of it.
        assert_eq!(placement(Dir::North, Dir::South), (Attach::Wall, Dir::North));
        // A floor plate faces the player, a ceiling one the way they look.
        assert_eq!(placement(Dir::Up, Dir::East), (Attach::Floor, Dir::West));
        assert_eq!(placement(Dir::Down, Dir::East), (Attach::Ceiling, Dir::East));
    }

    #[test]
    fn the_connected_direction_is_the_front() {
        assert_eq!(connected_direction(Attach::Floor, Dir::North), Dir::Up);
        assert_eq!(connected_direction(Attach::Ceiling, Dir::North), Dir::Down);
        assert_eq!(connected_direction(Attach::Wall, Dir::West), Dir::West);
    }

    #[test]
    fn an_empty_plate_takes_an_item_and_a_full_one_tests_it() {
        let yes = || true;
        let no = || false;
        assert_eq!(click(true, false, true, false, yes), Click::Place);
        // Adventure mode can't set an item.
        assert_eq!(click(false, false, true, false, yes), Click::Ignore);
        assert_eq!(click(true, true, true, false, yes), Click::Ignore);
        assert_eq!(click(true, false, false, false, yes), Click::Confirm);
        assert_eq!(click(true, false, false, false, no), Click::Deny);
        // A plate still giving its pulse ignores another click.
        assert_eq!(click(true, false, false, true, yes), Click::Ignore);
        assert_eq!(click(true, true, false, false, yes), Click::Ignore);
        // Adventure mode may still test an item.
        assert_eq!(click(false, false, false, false, yes), Click::Confirm);
    }

    #[test]
    fn the_matching_is_not_asked_when_it_cannot_matter() {
        let result = click(true, false, true, false, || panic!("not needed"));
        assert_eq!(result, Click::Place);
    }

    #[test]
    fn the_pulse_lasts_nineteen_ticks() {
        let mut ticks = POWERED_TICKS;
        let mut count = 0;
        loop {
            let (next, off) = tick(ticks);
            count += 1;
            ticks = next;
            if off {
                break;
            }
        }
        assert_eq!(count, 19);
        assert_eq!(tick(0), (0, false));
    }

    #[test]
    fn the_signal_is_direct_only_into_the_support() {
        assert_eq!(signal(true, false, false), 15);
        assert_eq!(signal(true, true, false), 0);
        assert_eq!(signal(true, true, true), 15);
        assert_eq!(signal(false, true, true), 0);
    }

    #[test]
    fn the_item_lies_on_the_plate() {
        let floor = item_pose(Attach::Floor, 180.0, false);
        assert_eq!((floor.y_degrees, floor.x_degrees, floor.scale), (360.0, 90.0, 0.375));
        let ceiling = item_pose(Attach::Ceiling, 0.0, true);
        assert_eq!((ceiling.y_degrees, ceiling.x_degrees, ceiling.scale), (360.0, -90.0, 0.5));
        let wall = item_pose(Attach::Wall, 90.0, true);
        assert_eq!((wall.y_degrees, wall.x_degrees, wall.forward), (270.0, 0.0, 4.5 / 16.0));
    }

    #[test]
    fn particles_ring_the_plate_on_its_support() {
        // A floor plate (front up): the ring lies flat, 0.45 below the centre.
        let at = particle_offset(Dir::Up, [0.3, 0.4, 0.0]);
        assert!((at[1] + 0.45).abs() < 1e-6);
        assert!(((at[0] * at[0] + at[2] * at[2]).sqrt() - 0.45).abs() < 1e-5);
        // A wall plate facing north is fixed to the south: its particles sit at +0.45 in z.
        let at = particle_offset(Dir::North, [0.1, 0.2, 0.5]);
        assert!((at[2] - 0.45).abs() < 1e-6);
        // No offset, no direction: the particle sits on the support.
        assert_eq!(particle_offset(Dir::Up, [0.0, 0.1, 0.0]), [0.0, -0.45, 0.0]);
    }
}
