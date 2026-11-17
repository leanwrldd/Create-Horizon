//! The chassis rules: which side holds on to the blocks next to it, what a click does to a side,
//! how a chassis is placed, how its sides turn with a structure, and its range. Ported from Create
//! (MIT, the Creators of Create), `mc1.21.1/dev` at 4148fd8: `AbstractChassisBlock` (`useItemOn`,
//! `rotate`, `transformInner`), `LinearChassisBlock` (`getStateForPlacement`, `updateShape`,
//! `getGlueableSide`, `glueAllowedOnSide`), `RadialChassisBlock.getGlueableSide` and
//! `ChassisBlockEntity` (the range, `ChassisScrollValueBehaviour`). What a chassis takes along when
//! a structure is gathered is `contraption::chassis_targets`.
//!
//! No game types: the glue is `native/api/src/machines/chassis.rs`.
//!
//! Known differences from Create:
//! - The range is set with the Wrench, not the scroll value box (see the README's Settings): a
//!   click on a side steps it by one, 16 back to 1. The group's range isn't set at once
//!   (`BulkScrollValueBehaviour`): each chassis keeps its own.
//! - Waterlogging is left out, as elsewhere. No connected textures.

use crate::rules::contraption::{MAX_CHASSIS_RANGE, State, rotate_dir};
use crate::{Axis, Dir};

pub const LINEAR: &str = "create:linear_chassis";
pub const SECONDARY: &str = "create:secondary_linear_chassis";
pub const RADIAL: &str = "create:radial_chassis";
/// `AllItems.SUPER_GLUE`, which `AbstractChassisBlock.useItemOn` treats as a slime ball.
pub const SUPER_GLUE: &str = "create:super_glue";
/// `Tags.Items.SLIMEBALLS` of vanilla's items.
pub const SLIME_BALL: &str = "minecraft:slime_ball";
/// The saved key of the range (`ScrollValueBehaviour.write`).
pub const RANGE_KEY: &str = "ScrollValue";

/// `range.setValue(max / 2)`.
pub const DEFAULT_RANGE: i32 = MAX_CHASSIS_RANGE / 2;

pub fn is_chassis(name: &str) -> bool {
    matches!(name, LINEAR | SECONDARY | RADIAL)
}

pub fn is_linear(name: &str) -> bool {
    matches!(name, LINEAR | SECONDARY)
}

/// The Wrench's stand-in for the value box: the range one more, 16 wrapping round to 1
/// (`range.between(1, max)`).
pub fn step_range(range: i32) -> i32 {
    if range >= MAX_CHASSIS_RANGE || range < 1 { 1 } else { range + 1 }
}

/// `getGlueableSide`: the `sticky_*` property that says whether `face` of a chassis turned about
/// `axis` is sticky; none for an end of a radial chassis or a side of a linear one.
pub fn glueable_side(radial: bool, axis: Axis, face: Dir) -> Option<&'static str> {
    if !radial {
        // `LinearChassisBlock`: only the two ends.
        if face.axis() != axis {
            return None;
        }
        return Some(if face.sign() > 0 { "sticky_top" } else { "sticky_bottom" });
    }
    // `RadialChassisBlock`: four properties, laid out by the axis.
    match (axis, face) {
        (Axis::X, Dir::North) => Some("sticky_west"),
        (Axis::X, Dir::South) => Some("sticky_east"),
        (Axis::X, Dir::Up) => Some("sticky_north"),
        (Axis::X, Dir::Down) => Some("sticky_south"),
        (Axis::Y, Dir::North) => Some("sticky_north"),
        (Axis::Y, Dir::South) => Some("sticky_south"),
        (Axis::Y, Dir::East) => Some("sticky_east"),
        (Axis::Y, Dir::West) => Some("sticky_west"),
        (Axis::Z, Dir::Up) => Some("sticky_north"),
        (Axis::Z, Dir::Down) => Some("sticky_south"),
        (Axis::Z, Dir::East) => Some("sticky_east"),
        (Axis::Z, Dir::West) => Some("sticky_west"),
        _ => None,
    }
}

/// The chassis state's `axis` property (Y when it has none).
pub fn axis_of(state: &State) -> Axis {
    state.get("axis").and_then(Axis::from_name).unwrap_or(Axis::Y)
}

/// Whether `face` of the chassis `state` is sticky.
pub fn is_sticky(state: &State, face: Dir) -> bool {
    glueable_side(state.is(RADIAL), axis_of(state), face).is_some_and(|key| state.flag(key))
}

/// `glueAllowedOnSide`: a linear chassis takes no glue on a side that touches a chassis of its own
/// kind and axis (they join there); a radial one always does. `other` is the block on that side:
/// its name and axis, if it has one.
pub fn glue_allowed(own: &State, other: Option<(&str, Axis)>) -> bool {
    if !is_linear(&own.name) {
        return true;
    }
    !matches!(other, Some((name, axis)) if name == own.name && axis == axis_of(own))
}

/// What `AbstractChassisBlock.useItemOn` does. `glue_item` is a slime ball or Super Glue, `empty`
/// an empty hand, `face` the face clicked. `allowed` answers `glueAllowedOnSide` for a face.
/// Returns the sides to change (property, new value), or none to leave the click to the block's
/// default (`PASS_TO_DEFAULT_BLOCK_INTERACTION`); an empty list means "handled, nothing changed".
pub fn use_item(
    state: &State,
    face: Dir,
    glue_item: bool,
    empty: bool,
    sneaking: bool,
    allowed: &dyn Fn(Dir) -> bool,
) -> Option<Vec<(&'static str, bool)>> {
    let radial = state.is(RADIAL);
    let axis = axis_of(state);
    let affected = glueable_side(radial, axis, face)?;
    if glue_item && state.flag(affected) {
        // A sticky side clicked with a slime ball: every other side that can take glue gets it.
        let mut changes = Vec::new();
        for other in Dir::ALL {
            let Some(key) = glueable_side(radial, axis, other) else { continue };
            if !state.flag(key) && allowed(other) {
                changes.push((key, true));
            }
        }
        return Some(changes);
    }
    if (!empty || !sneaking) && !glue_item {
        return None;
    }
    if state.flag(affected) == glue_item {
        return None;
    }
    if !allowed(face) {
        return None;
    }
    Some(vec![(affected, glue_item)])
}

/// `LinearChassisBlock.getStateForPlacement` and the radial chassis' plain pillar placement: the
/// axis. `placed_on` is the axis of the chassis the block is placed against, if it is one,
/// `nearest` the direction the player looks most along, `face` the face clicked.
pub fn placement_axis(radial: bool, placed_on: Option<Axis>, nearest: Dir, face: Dir, sneaking: bool) -> Axis {
    if radial || sneaking {
        // `RotatedPillarBlock.getStateForPlacement`.
        return face.axis();
    }
    placed_on.unwrap_or_else(|| nearest.axis())
}

/// `LinearChassisBlock.updateShape`: the property to clear when the neighbour on `side` is a
/// linear chassis of the same kind and axis (the two join there). `other` is that neighbour.
pub fn update_shape(state: &State, side: Dir, other: Option<(&str, Axis)>) -> Option<&'static str> {
    if !is_linear(&state.name) {
        return None;
    }
    let property = glueable_side(false, axis_of(state), side)?;
    let (name, axis) = other?;
    if name != state.name || axis != axis_of(state) || !state.flag(property) {
        return None;
    }
    Some(property)
}

/// `AbstractChassisBlock.transform` for a structure turned `quarters` quarter turns about `axis`
/// (`rotate` about Y, `transformInner` about the others): the axis turns, the sticky sides are
/// cleared, and each side that was sticky is set on the face it moved to.
pub fn rotate(state: &State, about: Axis, quarters: i32) -> State {
    let quarters = quarters.rem_euclid(4);
    if quarters == 0 || !is_chassis(&state.name) {
        return state.clone();
    }
    let radial = state.is(RADIAL);
    let old_axis = axis_of(state);
    let positive = match old_axis {
        Axis::X => Dir::East,
        Axis::Y => Dir::Up,
        Axis::Z => Dir::South,
    };
    let new_axis = rotate_dir(positive, about, quarters).axis();
    let mut out = state.clone();
    out.set("axis", new_axis.name());
    for face in Dir::ALL {
        if let Some(key) = glueable_side(radial, new_axis, face) {
            out.set(key, "false");
        }
    }
    for face in Dir::ALL {
        if !is_sticky(state, face) {
            continue;
        }
        let moved = rotate_dir(face, about, quarters);
        if let Some(key) = glueable_side(radial, new_axis, moved) {
            out.set(key, "true");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linear(axis: &str, top: bool, bottom: bool) -> State {
        State::new(LINEAR)
            .with("axis", axis)
            .with("sticky_top", if top { "true" } else { "false" })
            .with("sticky_bottom", if bottom { "true" } else { "false" })
    }

    fn radial(axis: &str) -> State {
        State::new(RADIAL)
            .with("axis", axis)
            .with("sticky_north", "false")
            .with("sticky_south", "false")
            .with("sticky_east", "false")
            .with("sticky_west", "false")
    }

    #[test]
    fn the_range_steps_and_wraps() {
        assert_eq!(DEFAULT_RANGE, 8);
        assert_eq!(step_range(8), 9);
        assert_eq!(step_range(16), 1);
        assert_eq!(step_range(0), 1);
    }

    #[test]
    fn a_linear_chassis_is_sticky_at_its_ends_only() {
        assert_eq!(glueable_side(false, Axis::X, Dir::East), Some("sticky_top"));
        assert_eq!(glueable_side(false, Axis::X, Dir::West), Some("sticky_bottom"));
        assert_eq!(glueable_side(false, Axis::Y, Dir::Up), Some("sticky_top"));
        assert_eq!(glueable_side(false, Axis::Y, Dir::North), None);
    }

    #[test]
    fn a_radial_chassis_has_four_sides_laid_out_by_its_axis() {
        // `RadialChassisBlock.getGlueableSide`.
        assert_eq!(glueable_side(true, Axis::Y, Dir::East), Some("sticky_east"));
        assert_eq!(glueable_side(true, Axis::Y, Dir::Up), None);
        assert_eq!(glueable_side(true, Axis::X, Dir::North), Some("sticky_west"));
        assert_eq!(glueable_side(true, Axis::X, Dir::Down), Some("sticky_south"));
        assert_eq!(glueable_side(true, Axis::Z, Dir::Up), Some("sticky_north"));
        assert_eq!(glueable_side(true, Axis::Z, Dir::South), None);
        // Every axis gives each of its four sides a different property.
        for axis in Axis::ALL {
            let mut keys: Vec<_> = Dir::ALL
                .into_iter()
                .filter_map(|d| glueable_side(true, axis, d))
                .collect();
            keys.sort_unstable();
            keys.dedup();
            assert_eq!(keys.len(), 4);
        }
    }

    #[test]
    fn a_slime_ball_makes_a_side_sticky_and_an_empty_sneaking_hand_clears_it() {
        let state = linear("y", false, false);
        let all = |_: Dir| true;
        let made = use_item(&state, Dir::Up, true, false, false, &all);
        assert_eq!(made, Some(vec![("sticky_top", true)]));
        // On a side that is not a glue side: the click is left to the block.
        assert_eq!(use_item(&state, Dir::North, true, false, false, &all), None);
        // An empty hand without sneaking does nothing; sneaking removes the glue.
        let sticky = linear("y", true, false);
        assert_eq!(use_item(&sticky, Dir::Up, false, true, false, &all), None);
        assert_eq!(
            use_item(&sticky, Dir::Up, false, true, true, &all),
            Some(vec![("sticky_top", false)])
        );
        // A side without glue stays as it is when the hand is empty.
        assert_eq!(use_item(&state, Dir::Up, false, true, true, &all), None);
        // Another item does nothing.
        assert_eq!(use_item(&sticky, Dir::Up, false, false, true, &all), None);
    }

    #[test]
    fn a_slime_ball_on_a_sticky_side_glues_every_other_side() {
        let state = radial("y");
        let sticky = State { ..state.clone() }.with("sticky_north", "true");
        let changes = use_item(&sticky, Dir::North, true, false, false, &|_| true).unwrap();
        assert_eq!(changes.len(), 3);
        assert!(changes.iter().all(|(_, on)| *on));
        // A side that may not take glue is left out.
        let changes = use_item(&sticky, Dir::North, true, false, false, &|d| d != Dir::East).unwrap();
        assert_eq!(changes.len(), 2);
    }

    #[test]
    fn a_linear_chassis_takes_no_glue_where_it_joins_its_own_kind() {
        let own = linear("y", false, false);
        assert!(!glue_allowed(&own, Some((LINEAR, Axis::Y))));
        assert!(glue_allowed(&own, Some((LINEAR, Axis::X))));
        assert!(glue_allowed(&own, Some((SECONDARY, Axis::Y))));
        assert!(glue_allowed(&own, None));
        assert!(glue_allowed(&radial("y"), Some((RADIAL, Axis::Y))));
        // And the click then passes.
        let allowed = |_: Dir| false;
        assert_eq!(use_item(&own, Dir::Up, true, false, false, &allowed), None);
    }

    #[test]
    fn placement_follows_the_neighbour_the_look_or_the_face() {
        // Not sneaking against a chassis: its axis. Otherwise the look's.
        assert_eq!(placement_axis(false, Some(Axis::X), Dir::Up, Dir::Up, false), Axis::X);
        assert_eq!(placement_axis(false, None, Dir::East, Dir::Up, false), Axis::X);
        // Sneaking, and the radial chassis: the face clicked.
        assert_eq!(placement_axis(false, Some(Axis::X), Dir::East, Dir::Up, true), Axis::Y);
        assert_eq!(placement_axis(true, Some(Axis::X), Dir::East, Dir::South, false), Axis::Z);
    }

    #[test]
    fn joining_a_neighbour_clears_the_glue_between_them() {
        let own = linear("y", true, true);
        assert_eq!(update_shape(&own, Dir::Up, Some((LINEAR, Axis::Y))), Some("sticky_top"));
        assert_eq!(update_shape(&own, Dir::Down, Some((LINEAR, Axis::Y))), Some("sticky_bottom"));
        assert_eq!(update_shape(&own, Dir::Up, Some((SECONDARY, Axis::Y))), None);
        assert_eq!(update_shape(&own, Dir::Up, Some((LINEAR, Axis::X))), None);
        assert_eq!(update_shape(&own, Dir::North, Some((LINEAR, Axis::Y))), None);
        assert_eq!(update_shape(&linear("y", false, true), Dir::Up, Some((LINEAR, Axis::Y))), None);
        assert_eq!(update_shape(&radial("y"), Dir::East, Some((RADIAL, Axis::Y))), None);
    }

    #[test]
    fn the_sticky_sides_turn_with_a_structure() {
        // A linear chassis standing up with glue on top, turned a quarter about X: it lies along
        // Z and the glue is on the end the top went to.
        let up = linear("y", true, false);
        let turned = rotate(&up, Axis::X, 1);
        assert_eq!(turned.get("axis"), Some("z"));
        let top_went_to = rotate_dir(Dir::Up, Axis::X, 1);
        assert!(is_sticky(&turned, top_went_to));
        assert!(!is_sticky(&turned, top_went_to.opposite()));
        // A half turn about Y keeps the axis and flips nothing on a vertical chassis' ends.
        assert_eq!(rotate(&up, Axis::Y, 2).text(), up.text());
        // A radial chassis' glued side follows the turn.
        let side = radial("y").with("sticky_east", "true");
        let turned = rotate(&side, Axis::Y, 1);
        let east_went_to = rotate_dir(Dir::East, Axis::Y, 1);
        assert!(is_sticky(&turned, east_went_to));
        assert_eq!((0..6).filter(|i| is_sticky(&turned, Dir::ALL[*i])).count(), 1);
        // Not a chassis, not turned.
        let stone = State::new("minecraft:stone");
        assert_eq!(rotate(&stone, Axis::Y, 1), stone);
    }
}
