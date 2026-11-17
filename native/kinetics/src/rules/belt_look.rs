//! Which models a belt block is drawn with, and how each is turned (Belts III).
//!
//! Ported from Create 6.0.10 (`mc1.21.1/dev` at `4148fd8`, MIT): `BeltRenderer.renderSafe` (the
//! belt's own partial models and its pulley), `getBeltPartial`, and `BeltGenerator` (the casing's
//! blockstate: which model and which turn, for each slope).

use crate::belt::{Part, Slope};
use crate::{Axis, Dir};

/// `AngleHelper.horizontalAngle`: south 0, east 90, north 180, west 270.
pub const fn horizontal_angle(facing: Dir) -> f32 {
    match facing {
        Dir::East => 90.0,
        Dir::North => 180.0,
        Dir::West => 270.0,
        _ => 0.0,
    }
}

/// `Direction.toYRot`, which the blockstate's `y` turn uses: south 0, west 90, north 180, east 270.
const fn y_rot(facing: Dir) -> i32 {
    match facing {
        Dir::West => 90,
        Dir::North => 180,
        Dir::East => 270,
        _ => 0,
    }
}

/// How `BeltRenderer` turns the belt's partial model about the block's centre
/// (`rotateY`, then `rotateZ`, then `rotateX`, in degrees) and which of its parts it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    pub y: f32,
    pub z: f32,
    pub x: f32,
    /// The belt's own first and last block, after `renderSafe` swaps them for a belt that runs
    /// backwards through its model (a downward one, or a vertical one facing a positive way).
    pub start: bool,
    pub end: bool,
    /// An upward or downward belt, with its own models and sprite and no bottom half.
    pub diagonal: bool,
}

pub fn body(facing: Dir, slope: Slope, part: Part) -> Body {
    let upward = slope == Slope::Upward;
    let sideways = slope == Slope::Sideways;
    let diagonal = slope.is_diagonal();
    let mut start = part == Part::Start;
    let mut end = part == Part::End;
    if slope == Slope::Downward || (slope == Slope::Vertical && facing.sign() > 0) {
        std::mem::swap(&mut start, &mut end);
    }
    Body {
        y: (horizontal_angle(facing) + if upward { 180.0 } else { 0.0 } + if sideways { 270.0 } else { 0.0 })
            % 360.0,
        z: if sideways { 90.0 } else { 0.0 },
        x: if !diagonal && slope != Slope::Horizontal { 90.0 } else { 0.0 },
        start,
        end,
        diagonal,
    }
}

/// `BeltRenderer.getBeltPartial`: the model of one half of the belt (a diagonal one has no bottom).
pub fn partial(diagonal: bool, start: bool, end: bool, bottom: bool) -> String {
    let name = if start {
        "start"
    } else if end {
        "end"
    } else {
        "middle"
    };
    if diagonal {
        format!("create:block/belt/diagonal_{name}")
    } else if bottom {
        format!("create:block/belt/{name}_bottom")
    } else {
        format!("create:block/belt/{name}")
    }
}

/// The turns of a belt's pulley (`BeltRenderer.renderSafe`'s `matrixStackSupplier`): it spins about
/// `axis`; the model is turned about y by `y` and then about x by `x` to lie along it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PulleyPose {
    pub axis: Axis,
    pub y: f32,
    pub x: f32,
}

pub fn pulley(facing: Dir, slope: Slope) -> PulleyPose {
    let axis = crate::belt::rotation_axis_of(facing, slope);
    PulleyPose {
        axis,
        y: if axis == Axis::X { 90.0 } else { 0.0 },
        // `rotateXDegrees(90)` for the axis y, and always once more.
        x: if axis == Axis::Y { 180.0 } else { 90.0 },
    }
}

/// The model of the casing round a belt block and its turn in the blockstate's terms (`x` then `y`,
/// degrees clockwise: `BeltGenerator.getXRotation`, `getYRotation`, `getModel` with `casing`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CasingPose {
    /// `create:block/belt_casing/<slope>_<part>`.
    pub model: String,
    pub x: i32,
    pub y: i32,
}

pub fn casing(facing: Dir, slope: Slope, part: Part) -> CasingPose {
    let negative = facing.sign() < 0;
    let downward = slope == Slope::Downward;
    let vertical = slope == Slope::Vertical;
    let sideways = slope == Slope::Sideways;
    let x = if vertical {
        90
    } else if sideways && negative {
        180
    } else {
        0
    };
    let y = (y_rot(facing)
        + if slope == Slope::Upward { 180 } else { 0 }
        + if vertical { 90 } else { 0 })
    .rem_euclid(360);
    let mut part = part;
    if (vertical && negative || downward || sideways && negative) && !matches!(part, Part::Middle | Part::Pulley) {
        part = if part == Part::End { Part::Start } else { Part::End };
    }
    // A vertical casing is the sideways model, turned.
    let kind = match slope {
        Slope::Horizontal => "horizontal",
        Slope::Upward | Slope::Downward => "diagonal",
        Slope::Vertical | Slope::Sideways => "sideways",
    };
    CasingPose {
        model: format!("create:block/belt_casing/{kind}_{}", part.name()),
        x,
        y,
    }
}

/// Every casing model a belt can wear, for the game to load.
pub fn casing_models() -> Vec<String> {
    let mut models = Vec::new();
    for kind in ["horizontal", "diagonal", "sideways"] {
        for part in Part::ALL {
            // `BeltGenerator` picks every part for every slope's model.
            models.push(format!("create:block/belt_casing/{kind}_{}", part.name()));
        }
    }
    models
}

/// Every model the belt's own body is drawn from.
pub fn body_models() -> Vec<String> {
    let mut models = Vec::new();
    for diagonal in [false, true] {
        for (start, end) in [(true, false), (false, false), (false, true)] {
            for bottom in [false, true] {
                if diagonal && bottom {
                    continue;
                }
                let model = partial(diagonal, start, end, bottom);
                if !models.contains(&model) {
                    models.push(model);
                }
            }
        }
    }
    models
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_horizontal_belt_is_only_turned_to_its_facing() {
        for (facing, angle) in [(Dir::South, 0.0), (Dir::East, 90.0), (Dir::North, 180.0), (Dir::West, 270.0)] {
            let b = body(facing, Slope::Horizontal, Part::Middle);
            assert_eq!((b.y, b.z, b.x), (angle, 0.0, 0.0));
            assert!(!b.diagonal && !b.start && !b.end);
        }
        assert!(body(Dir::South, Slope::Horizontal, Part::Start).start);
        assert!(body(Dir::South, Slope::Horizontal, Part::End).end);
        assert!(!body(Dir::South, Slope::Horizontal, Part::Pulley).start);
    }

    #[test]
    fn slopes_turn_their_models_as_the_renderer_does() {
        // An upward belt is the downward model turned half round.
        assert_eq!(body(Dir::South, Slope::Upward, Part::Middle).y, 180.0);
        assert_eq!(body(Dir::North, Slope::Upward, Part::Middle).y, 0.0);
        assert_eq!(body(Dir::South, Slope::Downward, Part::Middle).y, 0.0);
        assert!(body(Dir::South, Slope::Upward, Part::Middle).diagonal);
        // A downward belt's first block is drawn as the model's last.
        let down = body(Dir::South, Slope::Downward, Part::Start);
        assert!(down.end && !down.start);
        // A vertical belt is the horizontal one stood up, its ends swapped when it faces a
        // positive way (its start is at the bottom).
        let vertical = body(Dir::South, Slope::Vertical, Part::Start);
        assert_eq!((vertical.x, vertical.y, vertical.z), (90.0, 0.0, 0.0));
        assert!(vertical.end && !vertical.start);
        let vertical = body(Dir::North, Slope::Vertical, Part::Start);
        assert!(vertical.start && !vertical.end);
        // A sideways belt is turned about all three.
        let sideways = body(Dir::East, Slope::Sideways, Part::Middle);
        assert_eq!((sideways.y, sideways.z, sideways.x), (0.0, 90.0, 90.0));
        let sideways = body(Dir::South, Slope::Sideways, Part::Middle);
        assert_eq!((sideways.y, sideways.z, sideways.x), (270.0, 90.0, 90.0));
    }

    #[test]
    fn the_models_follow_the_part_and_the_slope() {
        assert_eq!(partial(false, true, false, false), "create:block/belt/start");
        assert_eq!(partial(false, false, true, true), "create:block/belt/end_bottom");
        assert_eq!(partial(false, false, false, true), "create:block/belt/middle_bottom");
        assert_eq!(partial(true, true, false, false), "create:block/belt/diagonal_start");
        assert_eq!(partial(true, false, false, true), "create:block/belt/diagonal_middle");
        let models = body_models();
        assert_eq!(models.len(), 9, "{models:?}");
        assert!(models.contains(&"create:block/belt/diagonal_end".to_owned()));
    }

    #[test]
    fn a_pulley_lies_along_the_shaft() {
        let p = pulley(Dir::South, Slope::Horizontal);
        assert_eq!((p.axis, p.y, p.x), (Axis::X, 90.0, 90.0));
        let p = pulley(Dir::East, Slope::Upward);
        assert_eq!((p.axis, p.y, p.x), (Axis::Z, 0.0, 90.0));
        let p = pulley(Dir::East, Slope::Sideways);
        assert_eq!((p.axis, p.y, p.x), (Axis::Y, 0.0, 180.0));
    }

    /// `assets/create/blockstates/belt.json` of the Create jar, for the casing variants of the
    /// belts facing south and north: (facing, slope, part, model, x, y).
    const BLOCKSTATE: &[(&str, &str, &str, &str, i32, i32)] = &[
        ("north", "downward", "end", "diagonal_start", 0, 180),
        ("north", "horizontal", "end", "horizontal_end", 0, 180),
        ("north", "sideways", "end", "sideways_start", 180, 180),
        ("north", "upward", "end", "diagonal_end", 0, 0),
        ("north", "vertical", "end", "sideways_start", 90, 270),
        ("north", "downward", "middle", "diagonal_middle", 0, 180),
        ("north", "horizontal", "middle", "horizontal_middle", 0, 180),
        ("north", "sideways", "middle", "sideways_middle", 180, 180),
        ("north", "upward", "middle", "diagonal_middle", 0, 0),
        ("north", "vertical", "middle", "sideways_middle", 90, 270),
        ("north", "downward", "pulley", "diagonal_pulley", 0, 180),
        ("north", "horizontal", "pulley", "horizontal_pulley", 0, 180),
        ("north", "sideways", "pulley", "sideways_pulley", 180, 180),
        ("north", "upward", "pulley", "diagonal_pulley", 0, 0),
        ("north", "vertical", "pulley", "sideways_pulley", 90, 270),
        ("north", "downward", "start", "diagonal_end", 0, 180),
        ("north", "horizontal", "start", "horizontal_start", 0, 180),
        ("north", "sideways", "start", "sideways_end", 180, 180),
        ("north", "upward", "start", "diagonal_start", 0, 0),
        ("north", "vertical", "start", "sideways_end", 90, 270),
        ("south", "downward", "end", "diagonal_start", 0, 0),
        ("south", "horizontal", "end", "horizontal_end", 0, 0),
        ("south", "sideways", "end", "sideways_end", 0, 0),
        ("south", "upward", "end", "diagonal_end", 0, 180),
        ("south", "vertical", "end", "sideways_end", 90, 90),
        ("south", "downward", "middle", "diagonal_middle", 0, 0),
        ("south", "horizontal", "middle", "horizontal_middle", 0, 0),
        ("south", "sideways", "middle", "sideways_middle", 0, 0),
        ("south", "upward", "middle", "diagonal_middle", 0, 180),
        ("south", "vertical", "middle", "sideways_middle", 90, 90),
        ("south", "downward", "pulley", "diagonal_pulley", 0, 0),
        ("south", "horizontal", "pulley", "horizontal_pulley", 0, 0),
        ("south", "sideways", "pulley", "sideways_pulley", 0, 0),
        ("south", "upward", "pulley", "diagonal_pulley", 0, 180),
        ("south", "vertical", "pulley", "sideways_pulley", 90, 90),
        ("south", "downward", "start", "diagonal_end", 0, 0),
        ("south", "horizontal", "start", "horizontal_start", 0, 0),
        ("south", "sideways", "start", "sideways_start", 0, 0),
        ("south", "upward", "start", "diagonal_start", 0, 180),
        ("south", "vertical", "start", "sideways_start", 90, 90),
    ];

    #[test]
    fn the_casing_is_drawn_as_the_blockstate_says() {
        for (facing, slope, part, model, x, y) in BLOCKSTATE {
            let got = casing(
                Dir::from_name(facing).unwrap(),
                Slope::from_name(slope).unwrap(),
                Part::from_name(part).unwrap(),
            );
            assert_eq!(
                got,
                CasingPose { model: format!("create:block/belt_casing/{model}"), x: *x, y: *y },
                "{facing} {slope} {part}"
            );
        }
        // East and west turn by the facing alone (`toYRot`).
        assert_eq!(casing(Dir::East, Slope::Horizontal, Part::Start).y, 270);
        assert_eq!(casing(Dir::West, Slope::Horizontal, Part::Start).y, 90);
        assert_eq!(casing(Dir::East, Slope::Upward, Part::Middle).y, 90);
    }

    #[test]
    fn every_model_a_casing_or_body_can_need_is_listed() {
        let listed = casing_models();
        for facing in [Dir::North, Dir::South, Dir::East, Dir::West] {
            for slope in Slope::ALL {
                for part in Part::ALL {
                    assert!(listed.contains(&casing(facing, slope, part).model));
                    let b = body(facing, slope, part);
                    assert!(body_models().contains(&partial(b.diagonal, b.start, b.end, false)));
                    assert!(body_models().contains(&partial(b.diagonal, b.start, b.end, true)) || b.diagonal);
                }
            }
        }
    }
}
