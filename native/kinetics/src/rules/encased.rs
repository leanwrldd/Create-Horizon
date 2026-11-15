//! Encased shafts and encased cogwheels: the rules of `EncasedShaftBlock`, `EncasedCogwheelBlock`,
//! `AbstractEncasedShaftBlock` and Create's encasing (`EncasableBlock`, `EncasingRegistry`).
//!
//! Known differences from Create, for the README:
//! - Encasing doesn't use up the casing, as in Create (`EncasableBlock.tryEncase` never shrinks the
//!   stack); un-encasing with a sneaking Wrench gives back no casing either.
//! - A wrenched encased cogwheel isn't refused where `canSurvive` would refuse it
//!   (`CogWheelBlock.isValidCogwheelPosition`); nothing checks that for any cogwheel yet.
//! - No connected textures between neighbouring casings (`EncasedCTBehaviour`).

use crate::{Axis, Dir};

/// Create's casing items (`AllBlocks.ANDESITE_CASING`, `BRASS_CASING`), declared elsewhere.
pub const ANDESITE_CASING: &str = "create:andesite_casing";
pub const BRASS_CASING: &str = "create:brass_casing";

/// The plain blocks that can be encased (`ShaftBlock`, `CogWheelBlock` are `EncasableBlock`s).
pub const SHAFT: &str = "create:shaft";
pub const COGWHEEL: &str = "create:cogwheel";
pub const LARGE_COGWHEEL: &str = "create:large_cogwheel";

/// Every encased block with the casing item that makes it and the plain block it encases
/// (`AllBlocks`, `EncasingRegistry.addVariantTo`).
pub const VARIANTS: [(&str, &str, &str); 6] = [
    ("create:andesite_encased_shaft", ANDESITE_CASING, SHAFT),
    ("create:brass_encased_shaft", BRASS_CASING, SHAFT),
    ("create:andesite_encased_cogwheel", ANDESITE_CASING, COGWHEEL),
    ("create:brass_encased_cogwheel", BRASS_CASING, COGWHEEL),
    (
        "create:andesite_encased_large_cogwheel",
        ANDESITE_CASING,
        LARGE_COGWHEEL,
    ),
    (
        "create:brass_encased_large_cogwheel",
        BRASS_CASING,
        LARGE_COGWHEEL,
    ),
];

/// What `casing` held in hand makes of the plain block `plain` (`EncasableBlock.tryEncase`).
pub fn encased_by(casing: &str, plain: &str) -> Option<&'static str> {
    VARIANTS
        .iter()
        .find(|(_, c, p)| *c == casing && *p == plain)
        .map(|(encased, _, _)| *encased)
}

/// The plain block under an encased one (`onSneakWrenched` switches to it).
pub fn plain_of(encased: &str) -> Option<&'static str> {
    VARIANTS
        .iter()
        .find(|(e, _, _)| *e == encased)
        .map(|(_, _, plain)| *plain)
}

/// Whether the encased block is a cogwheel, and a large one.
pub fn cog_kind(encased: &str) -> Option<bool> {
    match plain_of(encased)? {
        COGWHEEL => Some(false),
        LARGE_COGWHEEL => Some(true),
        _ => None,
    }
}

/// `IWrenchable.getRotatedBlockState` for a block with an `axis`: a quarter turn about the axis
/// of the face clicked (`VoxelShaper.axisAsFace(axis).getClockWise(faceAxis).getAxis()`). A block
/// lying along the face's own axis stays.
pub fn rotate_axis(axis: Axis, face_axis: Axis) -> Axis {
    if axis == face_axis {
        return axis;
    }
    Axis::ALL
        .into_iter()
        .find(|a| *a != axis && *a != face_axis)
        .expect("three axes")
}

/// `RotatedPillarKineticBlock.getStateForPlacement` (and `AbstractEncasedShaftBlock`'s, which
/// agrees): the axis a block is placed on. `preferred` is the axis the shaft ends beside it point
/// along, when they agree (`getPreferredAxis`).
pub fn placement_axis(
    preferred: Option<Axis>,
    sneaking: bool,
    clicked_face: Axis,
    looking: Axis,
) -> Axis {
    match preferred {
        Some(axis) if !sneaking => axis,
        Some(_) => clicked_face,
        None => looking,
    }
}

/// `EncasedCogwheelBlock.handleEncasing`: which shaft ends a cogwheel gets when it is encased,
/// `(top, bottom)`: one at each end of its axis where the neighbour there has a shaft end
/// pointing at it (`toward(d)`).
pub fn encased_cog_shafts(axis: Axis, toward: impl Fn(Dir) -> bool) -> (bool, bool) {
    let positive = [Dir::Up, Dir::South, Dir::East]
        .into_iter()
        .find(|d| d.axis() == axis)
        .expect("a positive end");
    (toward(positive), toward(positive.opposite()))
}

/// What a Wrench does to an encased cogwheel (`EncasedCogwheelBlock.onWrenched`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CogWrench {
    /// Clicked on a side: a quarter turn ([`rotate_cog`]).
    Turn,
    /// Clicked on an end: add or take away that end's shaft.
    ToggleTop,
    ToggleBottom,
}

pub fn cog_wrench(axis: Axis, clicked: Dir) -> CogWrench {
    if clicked.axis() != axis {
        CogWrench::Turn
    } else if clicked.sign() > 0 {
        CogWrench::ToggleTop
    } else {
        CogWrench::ToggleBottom
    }
}

/// `EncasedCogwheelBlock.swapShaftsForRotation` for a clockwise quarter turn about
/// `rotation_axis`: whether the cogwheel's two shaft ends change places.
fn swaps_shafts(axis: Axis, rotation_axis: Axis) -> bool {
    axis != rotation_axis
        && matches!(
            (rotation_axis, axis),
            (Axis::X, Axis::Y) | (Axis::Y, Axis::Z) | (Axis::Z, Axis::X)
        )
}

/// `EncasedCogwheelBlock.getRotatedBlockState`: the axis and shaft ends `(axis, top, bottom)`
/// after a wrench on a face along `face_axis`.
pub fn rotate_cog(axis: Axis, top: bool, bottom: bool, face_axis: Axis) -> (Axis, bool, bool) {
    let (top, bottom) = if swaps_shafts(axis, face_axis) {
        (bottom, top)
    } else {
        (top, bottom)
    };
    (rotate_axis(axis, face_axis), top, bottom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_casing_in_hand_picks_the_encased_block() {
        assert_eq!(
            encased_by(ANDESITE_CASING, SHAFT),
            Some("create:andesite_encased_shaft")
        );
        assert_eq!(
            encased_by(BRASS_CASING, LARGE_COGWHEEL),
            Some("create:brass_encased_large_cogwheel")
        );
        assert_eq!(encased_by("create:shaft", SHAFT), None);
        assert_eq!(encased_by(BRASS_CASING, "create:gearbox"), None);
        assert_eq!(
            plain_of("create:brass_encased_cogwheel"),
            Some(COGWHEEL)
        );
        assert_eq!(cog_kind("create:brass_encased_cogwheel"), Some(false));
        assert_eq!(cog_kind("create:andesite_encased_large_cogwheel"), Some(true));
        assert_eq!(cog_kind("create:andesite_encased_shaft"), None);
    }

    #[test]
    fn a_wrench_turns_a_block_a_quarter_about_the_face_clicked() {
        assert_eq!(rotate_axis(Axis::X, Axis::Y), Axis::Z);
        assert_eq!(rotate_axis(Axis::Z, Axis::Y), Axis::X);
        assert_eq!(rotate_axis(Axis::Y, Axis::Y), Axis::Y);
        assert_eq!(rotate_axis(Axis::X, Axis::Z), Axis::Y);
        assert_eq!(rotate_axis(Axis::Y, Axis::X), Axis::Z);
        assert_eq!(rotate_axis(Axis::X, Axis::X), Axis::X);
    }

    #[test]
    fn placement_follows_the_shafts_beside_it_unless_sneaking() {
        let p = |pref, sneak| placement_axis(pref, sneak, Axis::Z, Axis::X);
        assert_eq!(p(Some(Axis::Y), false), Axis::Y);
        assert_eq!(p(Some(Axis::Y), true), Axis::Z);
        assert_eq!(p(None, false), Axis::X);
        assert_eq!(p(None, true), Axis::X);
    }

    #[test]
    fn an_encased_cogwheel_gets_a_shaft_where_one_points_at_it() {
        let toward = |d| d == Dir::Up || d == Dir::East;
        assert_eq!(encased_cog_shafts(Axis::Y, toward), (true, false));
        assert_eq!(encased_cog_shafts(Axis::X, toward), (true, false));
        assert_eq!(encased_cog_shafts(Axis::Z, toward), (false, false));
        assert_eq!(encased_cog_shafts(Axis::Y, |d| d == Dir::Down), (false, true));
    }

    #[test]
    fn the_wrench_toggles_the_end_it_hits_and_turns_from_a_side() {
        assert_eq!(cog_wrench(Axis::Y, Dir::Up), CogWrench::ToggleTop);
        assert_eq!(cog_wrench(Axis::Y, Dir::Down), CogWrench::ToggleBottom);
        assert_eq!(cog_wrench(Axis::X, Dir::East), CogWrench::ToggleTop);
        assert_eq!(cog_wrench(Axis::X, Dir::West), CogWrench::ToggleBottom);
        assert_eq!(cog_wrench(Axis::Z, Dir::North), CogWrench::ToggleBottom);
        assert_eq!(cog_wrench(Axis::Y, Dir::North), CogWrench::Turn);
    }

    #[test]
    fn turning_a_cogwheel_keeps_its_shaft_ends_where_they_point() {
        // `swapShaftsForRotation` swaps the ends for a clockwise turn about X of a y cogwheel,
        // about Y of a z one and about Z of an x one, and for no other.
        assert_eq!(rotate_cog(Axis::Y, true, false, Axis::X), (Axis::Z, false, true));
        assert_eq!(rotate_cog(Axis::Z, true, false, Axis::Y), (Axis::X, false, true));
        assert_eq!(rotate_cog(Axis::X, true, false, Axis::Z), (Axis::Y, false, true));
        assert_eq!(rotate_cog(Axis::Y, true, false, Axis::Z), (Axis::X, true, false));
        assert_eq!(rotate_cog(Axis::Z, true, false, Axis::X), (Axis::Y, true, false));
        assert_eq!(rotate_cog(Axis::X, true, false, Axis::Y), (Axis::Z, true, false));
        // Along the face's own axis nothing turns.
        assert_eq!(rotate_cog(Axis::Y, true, false, Axis::Y), (Axis::Y, true, false));
    }
}
