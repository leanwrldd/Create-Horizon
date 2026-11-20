//! The Stock Link block (`PackagerLinkBlock`, `FaceAttachedHorizontalDirectionalBlock`) and the saved form of
//! the network frequency (`LogisticallyLinkedBehaviour.write`, `NbtUtils.createUUID`): which way the block is
//! attached, where it is placed, how its frequency is saved, and the ids the glue gives packagers and
//! inventories.
//!
//! No game types. Known differences from Create, for the README:
//! - `canSurvive` is always true in Create, as here; `getStateForPlacement` of the vanilla base class takes the first of
//!   the player's nearest-looking directions, which is what [`placement`] does.
//! - A packager's and an inventory's identity (`InventoryIdentifier`, `PackagerBlockEntity.hashCode`) is its block position
//!   here ([`pos_id`]); Create identifies a multi-block inventory (a vault, a double chest) by all its parts, which the glue
//!   answers by giving every part the same id (the part with the lowest position).

use super::logistics_network::Freq;
use crate::{Dir, Pos};

/// `AttachFace`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachFace {
    Floor,
    Wall,
    Ceiling,
}

impl AttachFace {
    pub fn name(self) -> &'static str {
        match self {
            AttachFace::Floor => "floor",
            AttachFace::Wall => "wall",
            AttachFace::Ceiling => "ceiling",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "floor" => AttachFace::Floor,
            "wall" => AttachFace::Wall,
            "ceiling" => AttachFace::Ceiling,
            _ => return None,
        })
    }
}

/// `FaceAttachedHorizontalDirectionalBlock.getConnectedDirection`: the ceiling is `DOWN`, the floor `UP`, a wall the
/// way the block faces.
pub fn connected_direction(face: AttachFace, facing: Dir) -> Dir {
    match face {
        AttachFace::Ceiling => Dir::Down,
        AttachFace::Floor => Dir::Up,
        AttachFace::Wall => facing,
    }
}

/// `FaceAttachedHorizontalDirectionalBlock.getStateForPlacement` then `PackagerLinkBlock.getStateForPlacement`:
/// `look` is the first of `getNearestLookingDirections`, `horizontal` is `getHorizontalDirection`. Looking up
/// attaches to the ceiling, down to the floor, any other way to a wall facing back at the player; a block on the ceiling
/// faces the opposite way (`facing.getOpposite()`).
pub fn placement(look: Dir, horizontal: Dir) -> (AttachFace, Dir) {
    match look {
        Dir::Up => (AttachFace::Ceiling, horizontal.opposite()),
        Dir::Down => (AttachFace::Floor, horizontal),
        wall => (AttachFace::Wall, wall.opposite()),
    }
}

/// `NbtUtils.createUUID` / `uuidFromIntArray`: a UUID as four ints, the most significant first.
pub fn freq_to_ints(freq: Freq) -> [i32; 4] {
    [(freq >> 96) as u32 as i32, (freq >> 64) as u32 as i32, (freq >> 32) as u32 as i32, freq as u32 as i32]
}

/// The inverse of [`freq_to_ints`].
pub fn freq_from_ints(ints: [i32; 4]) -> Freq {
    ints.iter().fold(0u128, |acc, i| (acc << 32) | u128::from(*i as u32))
}

/// A UUID's text form (`8-4-4-4-12` hex digits), for a chat line.
pub fn freq_text(freq: Freq) -> String {
    let hex = format!("{freq:032x}");
    format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])
}

/// `BlockPos.asLong`: x in 26 bits, z in 26 bits, y in 12 bits.
pub fn pos_id(pos: Pos) -> u64 {
    let x = (pos[0] as i64 & 0x3FF_FFFF) as u64;
    let y = (pos[1] as i64 & 0xFFF) as u64;
    let z = (pos[2] as i64 & 0x3FF_FFFF) as u64;
    (x << 38) | (z << 12) | y
}

/// The salt that spreads a block entity's lazy tick over the period (`BlockEntity` ticks are not in step).
pub fn lazy_salt(pos: Pos, period: i64) -> i64 {
    (i64::from(pos[0]) * 31 + i64::from(pos[1]) * 17 + i64::from(pos[2]) * 7).rem_euclid(period)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connected_direction_follows_the_face() {
        assert_eq!(connected_direction(AttachFace::Floor, Dir::North), Dir::Up);
        assert_eq!(connected_direction(AttachFace::Ceiling, Dir::North), Dir::Down);
        assert_eq!(connected_direction(AttachFace::Wall, Dir::East), Dir::East);
    }

    #[test]
    fn placement_takes_the_first_looking_direction() {
        assert_eq!(placement(Dir::Down, Dir::South), (AttachFace::Floor, Dir::South));
        assert_eq!(placement(Dir::Up, Dir::South), (AttachFace::Ceiling, Dir::North));
        assert_eq!(placement(Dir::West, Dir::South), (AttachFace::Wall, Dir::East));
    }

    #[test]
    fn a_frequency_survives_its_int_form() {
        let freq: Freq = 0x0123_4567_89ab_cdef_fedc_ba98_7654_3210;
        let ints = freq_to_ints(freq);
        assert_eq!(ints[0], 0x0123_4567);
        assert_eq!(ints[3], 0x7654_3210);
        assert_eq!(freq_from_ints(ints), freq);
        assert_eq!(freq_from_ints(freq_to_ints(u128::MAX)), u128::MAX);
        assert_eq!(freq_text(freq), "01234567-89ab-cdef-fedc-ba9876543210");
    }

    #[test]
    fn position_ids_differ_by_position() {
        assert_ne!(pos_id([1, 64, 2]), pos_id([2, 64, 1]));
        assert_eq!(pos_id([5, 70, -3]), pos_id([5, 70, -3]));
    }

    #[test]
    fn the_lazy_salt_stays_in_the_period() {
        for x in -20..20 {
            assert!((0..10).contains(&lazy_salt([x, 3, -x], 10)));
        }
    }
}
