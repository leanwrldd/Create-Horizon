//! The Wooden and Metal Brackets' rules: which bracket fits a shaft, cogwheel or pipe from the face
//! clicked. Ported from Create `mc1.21.1/dev` at 4148fd8 (MIT, the Creators of Create):
//! `BracketBlock.getSuitableBracket`, `BracketBlockItem.useOn`, `FluidPropagator.getStraightPipeAxis`
//! and `BracketedBlockEntityBehaviour`. No game types.

use crate::{Axis, Dir};

/// `BracketBlock.BracketType`: the part the bracket holds, which picks its model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    Pipe,
    Cog,
    Shaft,
}

impl Type {
    /// `getSerializedName`: the `type` property.
    pub fn name(self) -> &'static str {
        match self {
            Type::Pipe => "pipe",
            Type::Cog => "cog",
            Type::Shaft => "shaft",
        }
    }
}

/// A bracket block state: what `BracketBlock` stores under `Bracket` in the held block's data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bracket {
    /// `type`.
    pub kind: Type,
    /// `facing`: the side the bracket's plate is on.
    pub facing: Dir,
    /// `axis_along_first`.
    pub axis_along_first: bool,
}

/// `BracketBlock.getSuitableBracket(axis, direction, type)`: a bracket on the `direction` side of a
/// block whose own axis is `target_axis`, or `None` when that side lies on the block's axis
/// (nothing to hold it) or the block has no straight axis (a bent pipe).
pub fn suitable(target_axis: Option<Axis>, direction: Dir, kind: Type) -> Option<Bracket> {
    let axis = direction.axis();
    let target = target_axis?;
    if target == axis {
        return None;
    }
    let along_first = if axis != Axis::Z { target == Axis::Z } else { target == Axis::Y };
    Some(Bracket { kind, facing: direction, axis_along_first: !along_first })
}

/// `FluidPropagator.getStraightPipeAxis` for a pipe with `sides` open (down, up, north, south,
/// west, east): the axis it runs along when it is open at both ends of one axis and nowhere else.
pub fn straight_pipe_axis(sides: &[bool; 6]) -> Option<Axis> {
    let mut found = None;
    let mut connections = 0;
    for axis in Axis::ALL {
        // `Direction.get(NEGATIVE, axis)` and its opposite.
        let (negative, positive) = match axis {
            Axis::X => (Dir::West, Dir::East),
            Axis::Y => (Dir::Down, Dir::Up),
            Axis::Z => (Dir::North, Dir::South),
        };
        let at = |d: Dir| sides[Dir::ALL.iter().position(|x| *x == d).unwrap_or(0)];
        let (open1, open2) = (at(negative), at(positive));
        connections += usize::from(open1) + usize::from(open2);
        if open1 && open2 {
            found = Some(axis);
        }
    }
    if connections == 2 { found } else { None }
}

/// What `BracketBlockItem.useOn` does with the bracket already on the block when a new one goes
/// on: the first is the item the player gets back (survival), the second whether the click
/// changes anything. `None` when the same bracket is there already (`bracket == newBracket`).
pub fn replaces(old: Option<Bracket>, new: Bracket) -> bool {
    old != Some(new)
}

/// `BracketBlockItem.useOn`'s second try when the clicked face doesn't fit: the side facing the
/// player (`Direction.orderedByNearest(player)[0].getOpposite()`), given the direction they look.
pub fn towards_player(look: Dir) -> Dir {
    look.opposite()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bracket_goes_on_the_sides_of_a_shaft() {
        // A shaft along Y with a bracket on its north face: a wall bracket.
        let b = suitable(Some(Axis::Y), Dir::North, Type::Shaft).unwrap();
        assert_eq!(b.facing, Dir::North);
        assert_eq!(b.kind, Type::Shaft);
        // `alongFirst = axis != Z ? target == Z : target == Y`: north is Z, so target Y gives true
        // and the stored property is its negation.
        assert!(!b.axis_along_first);
        let b = suitable(Some(Axis::Z), Dir::Up, Type::Cog).unwrap();
        assert!(!b.axis_along_first);
        let b = suitable(Some(Axis::X), Dir::Up, Type::Pipe).unwrap();
        assert!(b.axis_along_first);
        let b = suitable(Some(Axis::Y), Dir::East, Type::Shaft).unwrap();
        assert!(b.axis_along_first);
    }

    #[test]
    fn nothing_holds_the_end_of_a_shaft_or_a_bent_pipe() {
        assert_eq!(suitable(Some(Axis::Y), Dir::Up, Type::Shaft), None);
        assert_eq!(suitable(Some(Axis::X), Dir::West, Type::Pipe), None);
        assert_eq!(suitable(None, Dir::Up, Type::Pipe), None);
    }

    #[test]
    fn a_straight_pipe_has_exactly_the_two_ends_of_one_axis() {
        let sides = |open: &[Dir]| Dir::ALL.map(|d| open.contains(&d));
        assert_eq!(straight_pipe_axis(&sides(&[Dir::North, Dir::South])), Some(Axis::Z));
        assert_eq!(straight_pipe_axis(&sides(&[Dir::Up, Dir::Down])), Some(Axis::Y));
        assert_eq!(straight_pipe_axis(&sides(&[Dir::East, Dir::West])), Some(Axis::X));
        assert_eq!(straight_pipe_axis(&sides(&[Dir::East, Dir::North])), None);
        assert_eq!(straight_pipe_axis(&sides(&[Dir::East, Dir::West, Dir::Up])), None);
        assert_eq!(straight_pipe_axis(&sides(&[Dir::East])), None);
        assert_eq!(straight_pipe_axis(&sides(&[])), None);
    }

    #[test]
    fn the_same_bracket_again_changes_nothing() {
        let a = suitable(Some(Axis::Y), Dir::North, Type::Shaft).unwrap();
        let b = suitable(Some(Axis::Y), Dir::South, Type::Shaft).unwrap();
        assert!(!replaces(Some(a), a));
        assert!(replaces(Some(a), b));
        assert!(replaces(None, a));
    }

    #[test]
    fn the_side_facing_the_player_is_opposite_their_look() {
        assert_eq!(towards_player(Dir::North), Dir::South);
        assert_eq!(towards_player(Dir::Down), Dir::Up);
    }
}
