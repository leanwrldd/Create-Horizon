//! The Nixie Tube's rules (`NixieTubeBlock`, `DoubleFaceAttachedBlock`, `NixieTubeBlockEntity`,
//! `NixieTubeRenderer`, `DyeHelper`): how it is placed, which way a row of tubes runs, what its two
//! tubes show, and the colours they glow in.
//!
//! Known differences from Create, for the README:
//! - The block has no waterlogging and no tube-shaped hit box (a declared shape can hang on one
//!   state property only, and the tube needs two); the glass-like cube is used.
//! - No train signal or ComputerCraft display (`SignalBlockEntity`, `ComputerSignal`).

use crate::{Dir, Pos};

use super::content_observer_target::ordered_by_nearest;

/// `DoubleFaceAttachedBlock.DoubleAttachFace`, in Create's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    Floor,
    Wall,
    WallReversed,
    Ceiling,
}

impl Face {
    pub const ALL: [Face; 4] = [Face::Floor, Face::Wall, Face::WallReversed, Face::Ceiling];

    /// The state's `double_face` value.
    pub const fn name(self) -> &'static str {
        match self {
            Face::Floor => "floor",
            Face::Wall => "wall",
            Face::WallReversed => "wall_reversed",
            Face::Ceiling => "ceiling",
        }
    }

    pub fn from_name(name: &str) -> Option<Face> {
        Face::ALL.into_iter().find(|f| f.name() == name)
    }
}

/// `Direction.getClockWise` for a horizontal direction.
pub fn clockwise(dir: Dir) -> Dir {
    match dir {
        Dir::North => Dir::East,
        Dir::East => Dir::South,
        Dir::South => Dir::West,
        Dir::West => Dir::North,
        other => other,
    }
}

/// `NixieTubeBlock.getLeftNixieDirection`: where the row's first tube is.
pub fn left_direction(face: Face, facing: Dir) -> Dir {
    match face {
        Face::Wall => Dir::Up,
        Face::WallReversed => Dir::Down,
        _ => facing.opposite(),
    }
}

/// `NixieTubeBlock.getRightNixieDirection`: the way the row's text reads.
pub fn right_direction(face: Face, facing: Dir) -> Dir {
    left_direction(face, facing).opposite()
}

/// `DoubleFaceAttachedBlock.getConnectedDirection`.
pub fn connected_direction(face: Face, facing: Dir) -> Dir {
    match face {
        Face::Ceiling => Dir::Down,
        Face::Floor => Dir::Up,
        _ => facing,
    }
}

/// `BlockPlaceContext.getNearestLookingDirections`: the directions nearest the look first, the
/// clicked face's opposite moved to the front unless the click replaced the block it hit.
pub fn nearest_looking(yaw: f32, pitch: f32, clicked: Dir, replace_clicked: bool) -> [Dir; 6] {
    let mut nearest = ordered_by_nearest(yaw, pitch);
    if replace_clicked {
        return nearest;
    }
    let behind = clicked.opposite();
    if let Some(at) = nearest.iter().position(|&d| d == behind)
        && at > 0
    {
        nearest.copy_within(0..at, 1);
        nearest[0] = behind;
    }
    nearest
}

/// `Entity.calculateViewVector`.
pub fn look_vector(yaw: f32, pitch: f32) -> [f64; 3] {
    let (yaw, pitch) = (f64::from(yaw).to_radians(), f64::from(pitch).to_radians());
    [-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos()]
}

/// `DoubleFaceAttachedBlock.getStateForPlacement` then `NixieTubeBlock.getStateForPlacement`: the
/// face and facing of a tube put down by a player who looks `look`, faces `horizontal` and whose
/// look orders the directions `nearest` (every one can be built on: a tube has no `canSurvive`).
pub fn placement(nearest: [Dir; 6], horizontal: Dir, look: [f64; 3]) -> (Face, Dir) {
    let direction = nearest[0];
    if direction.axis() == crate::Axis::Y {
        let face = if direction == Dir::Up { Face::Ceiling } else { Face::Floor };
        // A tube lying on the floor or ceiling has its row turned across the player.
        return (face, clockwise(horizontal));
    }
    let n = clockwise(direction).normal();
    let dot = look[0] * f64::from(n[0]) + look[1] * f64::from(n[1]) + look[2] * f64::from(n[2]);
    let face = if dot < 0.0 { Face::WallReversed } else { Face::Wall };
    (face, direction.opposite())
}

/// `NixieTubeBlockEntity.charOrEmpty`.
fn char_or_empty(text: &str, index: usize) -> String {
    text.chars().nth(index).map_or_else(|| " ".to_owned(), |c| c.to_string())
}

/// `NixieTubeBlockEntity.updateDisplayedStrings`: what the two tubes of the block at `index` in
/// its row show, for the text a link wrote (`custom`) or else the redstone strength.
pub fn displayed(custom: Option<&str>, strength: i32, index: usize) -> (String, String) {
    match custom {
        Some(text) => (char_or_empty(text, index * 2), char_or_empty(text, index * 2 + 1)),
        None => (
            if strength < 10 { "0" } else { "1" }.to_owned(),
            (strength % 10).to_string(),
        ),
    }
}

/// `NixieTubeBlockEntity.getFullText`: the text a display link reads from a tube.
pub fn full_text(custom: Option<&str>, strength: i32) -> String {
    custom.map_or_else(|| strength.to_string(), str::to_owned)
}

/// `NixieTubeBlock.walkNixies`: the positions of the row `start` is in, left to right, given the
/// direction to the left and whether the block at a position is an equal tube.
pub fn row(start: Pos, left: Dir, same: impl Fn(Pos) -> bool) -> Vec<Pos> {
    let step = |from: Pos, d: Dir| {
        let n = d.normal();
        [from[0] + n[0], from[1] + n[1], from[2] + n[2]]
    };
    let mut first = start;
    // The row is bounded as the world is; the cap stops a loop in a broken `same`.
    for _ in 0..4096 {
        let next = step(first, left);
        if !same(next) {
            break;
        }
        first = next;
    }
    let mut out = vec![first];
    let right = left.opposite();
    for _ in 0..4096 {
        let next = step(*out.last().unwrap_or(&first), right);
        if !same(next) {
            break;
        }
        out.push(next);
    }
    out
}

/// `DyeHelper.DYE_TABLE`: (dye, front colour, back colour) in `DyeColor` order.
pub const DYES: [(&str, u32, u32); 16] = [
    ("white", 0xEDEAE5, 0xBBB6B0),
    ("orange", 0xEE9246, 0xD94927),
    ("magenta", 0xF062B0, 0xC04488),
    ("light_blue", 0x69CED2, 0x508AA5),
    ("yellow", 0xE6D756, 0xE9AC29),
    ("lime", 0xA3DF55, 0x4FB16F),
    ("pink", 0xD5A8CB, 0xB86B95),
    ("gray", 0x5D666F, 0x313538),
    ("light_gray", 0x95969B, 0x707070),
    ("cyan", 0x3EABB4, 0x3C7872),
    ("purple", 0x9F54AE, 0x63366C),
    ("blue", 0x5391E1, 0x504B90),
    ("brown", 0xAC855C, 0x68533E),
    ("green", 0x208A46, 0x1D6045),
    ("red", 0xB13937, 0x632737),
    ("black", 0x45403B, 0x21201F),
];

/// `DyeHelper.getDyeColors`: the front and back colour of a dye (white's for an unknown one).
pub fn dye_colors(dye: &str) -> (u32, u32) {
    let (_, bright, dark) = DYES.iter().find(|(name, _, _)| *name == dye).copied().unwrap_or(DYES[0]);
    (bright, dark)
}

/// The id of the tube of a colour (`AllBlocks.NIXIE_TUBES`): the orange one has no colour in its name.
pub fn block_id(dye: &str) -> String {
    if dye == "orange" {
        "create:nixie_tube".to_owned()
    } else {
        format!("create:{dye}_nixie_tube")
    }
}

/// `NixieTubeBlock.colorOf`: the dye of a tube block, `None` for another block.
pub fn dye_of_block(id: &str) -> Option<&'static str> {
    DYES.iter().map(|(name, _, _)| *name).find(|dye| block_id(dye) == id)
}

/// `DyeColor.getColor(stack)`: the dye an item is.
pub fn dye_of_item(item: &str) -> Option<&'static str> {
    let name = item.strip_prefix("minecraft:")?.strip_suffix("_dye")?;
    DYES.iter().map(|(dye, _, _)| *dye).find(|dye| *dye == name)
}

/// `Color.mixColors(a, b, t)`: `a` towards `b` by `t`, channel by channel.
pub fn mix_colors(a: u32, b: u32, t: f32) -> u32 {
    let channel = |shift: u32| {
        let (x, y) = (((a >> shift) & 0xFF) as f32, ((b >> shift) & 0xFF) as f32);
        ((x + (y - x) * t) as u32).min(255) << shift
    };
    channel(16) | channel(8) | channel(0)
}

/// How `NixieTubeRenderer.drawTube` colours one tube: the front glyph's colour (the bright colour
/// flickering a quarter of the way towards the dark one by `flicker`, 0 to 1), the back glyph's
/// and the shadow behind the back glyph.
pub fn tube_colors(dye: &str, flicker: f32) -> (u32, u32, u32) {
    let (bright, dark) = dye_colors(dye);
    (mix_colors(bright, dark, flicker / 4.0), dark, mix_colors(dark, 0, 0.35))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_floor_tube_reads_along_its_facing_and_a_wall_tube_up_and_down() {
        assert_eq!(left_direction(Face::Floor, Dir::East), Dir::West);
        assert_eq!(right_direction(Face::Ceiling, Dir::East), Dir::East);
        assert_eq!(left_direction(Face::Wall, Dir::North), Dir::Up);
        assert_eq!(right_direction(Face::WallReversed, Dir::North), Dir::Up);
        assert_eq!(connected_direction(Face::Floor, Dir::North), Dir::Up);
        assert_eq!(connected_direction(Face::Ceiling, Dir::North), Dir::Down);
        assert_eq!(connected_direction(Face::Wall, Dir::North), Dir::North);
    }

    #[test]
    fn placement_on_a_floor_turns_the_row_across_the_player() {
        // Looking down while facing south.
        let nearest = nearest_looking(0.0, 80.0, Dir::Up, false);
        assert_eq!(nearest[0], Dir::Down);
        let (face, facing) = placement(nearest, Dir::South, look_vector(0.0, 80.0));
        assert_eq!(face, Face::Floor);
        assert_eq!(facing, Dir::West);
        // Looking up puts it on the ceiling.
        let nearest = nearest_looking(0.0, -80.0, Dir::Down, false);
        assert_eq!(placement(nearest, Dir::South, look_vector(0.0, -80.0)).0, Face::Ceiling);
    }

    #[test]
    fn placement_on_a_wall_faces_away_from_it_and_reverses_with_the_look() {
        // Facing south at a wall: the clicked face is north (the wall's near side), the block goes
        // against the wall to the south.
        let nearest = nearest_looking(0.0, 0.0, Dir::North, false);
        assert_eq!(nearest[0], Dir::South);
        let (face, facing) = placement(nearest, Dir::South, look_vector(0.0, 0.0));
        assert_eq!(facing, Dir::North);
        // clockwise(south) = west, normal (-1, 0, 0); looking south, the dot is 0: not reversed.
        assert_eq!(face, Face::Wall);
        // Looking a little to the east (yaw -20) leans the dot negative: reversed.
        let (face, _) = placement(nearest, Dir::South, look_vector(-20.0, 0.0));
        assert_eq!(face, Face::WallReversed);
        let (face, _) = placement(nearest, Dir::South, look_vector(20.0, 0.0));
        assert_eq!(face, Face::Wall);
    }

    #[test]
    fn the_clicked_face_goes_first_unless_the_click_replaced_the_block() {
        let order = nearest_looking(0.0, 0.0, Dir::East, false);
        assert_eq!(order[0], Dir::West);
        let replaced = nearest_looking(0.0, 0.0, Dir::East, true);
        assert_eq!(replaced[0], Dir::South);
        let mut sorted = order;
        sorted.sort_by_key(|d| *d as u8);
        assert_eq!(sorted.len(), 6);
        assert!(order.windows(2).all(|w| w[0] != w[1]));
    }

    #[test]
    fn redstone_shows_two_digits_and_text_two_characters_a_tube() {
        assert_eq!(displayed(None, 7, 0), ("0".into(), "7".into()));
        assert_eq!(displayed(None, 12, 3), ("1".into(), "2".into()));
        assert_eq!(displayed(Some("HELLO"), 0, 0), ("H".into(), "E".into()));
        assert_eq!(displayed(Some("HELLO"), 0, 2), ("O".into(), " ".into()));
        assert_eq!(displayed(Some("HELLO"), 0, 5), (" ".into(), " ".into()));
        assert_eq!(full_text(None, 9), "9");
        assert_eq!(full_text(Some("hi"), 9), "hi");
    }

    #[test]
    fn a_row_is_walked_from_its_left_end() {
        let tubes = [[0, 0, 0], [1, 0, 0], [2, 0, 0], [3, 0, 0]];
        let row = row([2, 0, 0], Dir::West, |p| tubes.contains(&p));
        assert_eq!(row, tubes.to_vec());
        let single = row_of_one();
        assert_eq!(single, vec![[5, 5, 5]]);
    }

    fn row_of_one() -> Vec<Pos> {
        row([5, 5, 5], Dir::Up, |p| p == [5, 5, 5])
    }

    #[test]
    fn colours_follow_the_dye_table() {
        assert_eq!(dye_colors("orange"), (0xEE9246, 0xD94927));
        assert_eq!(dye_colors("nonsense"), (0xEDEAE5, 0xBBB6B0));
        assert_eq!(block_id("orange"), "create:nixie_tube");
        assert_eq!(block_id("light_blue"), "create:light_blue_nixie_tube");
        assert_eq!(dye_of_block("create:red_nixie_tube"), Some("red"));
        assert_eq!(dye_of_block("create:nixie_tube"), Some("orange"));
        assert_eq!(dye_of_block("create:shaft"), None);
        assert_eq!(dye_of_item("minecraft:lime_dye"), Some("lime"));
        assert_eq!(dye_of_item("minecraft:bone_meal"), None);
        assert_eq!(mix_colors(0xFF0000, 0x0000FF, 0.0), 0xFF0000);
        assert_eq!(mix_colors(0xFF0000, 0x0000FF, 1.0), 0x0000FF);
        let (front, back, _) = tube_colors("orange", 0.0);
        assert_eq!((front, back), (0xEE9246, 0xD94927));
    }
}
