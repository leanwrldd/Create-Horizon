//! The Analog Lever's rules (`AnalogLeverBlock`, `AnalogLeverBlockEntity`, `AnalogLeverRenderer`):
//! a 0 to 15 output raised and lowered a step at a time, whose neighbours are told 15 ticks after
//! the last change. No game types: the game glue is `machines/analog_lever.rs`.
//!
//! Known differences from Create, for the README:
//! - The handle's easing (`LerpedFloat.chase(state, 0.2, Chaser.EXP)`) is worked out from the time
//!   of the change (`eased`) instead of ticking on the client.
//! - Goggles don't show "Analog Strength" yet.
//! - The lever is not wrenchable, as in Create, and its shapes are vanilla's lever's
//!   (`BlockBehaviourAccessor.create$getShape(Blocks.LEVER ...)`), written in `tools/content`.

/// `AnalogLeverBlockEntity.changeState`: the output is kept between 0 and 15.
pub const MAX_STATE: i32 = 15;

/// `AnalogLeverBlockEntity.changeState`: `lastChange = 15` after a change; neighbours are told
/// when it counts down to 0 (`updateOutput`).
pub const CHANGE_DELAY: i32 = 15;

/// `AnalogLeverBlockEntity.changeState(back)`: the new output and whether it changed.
pub fn change_state(state: i32, back: bool) -> (i32, bool) {
    let next = (state + if back { -1 } else { 1 }).clamp(0, MAX_STATE);
    (next, next != state)
}

/// `AnalogLeverBlock.useWithoutItem`: the click's pitch, from the output after the change
/// (`.25f + ((be.state + 5) / 15f) * .5f`).
pub fn click_pitch(state: i32) -> f32 {
    0.25 + ((state + 5) as f32 / 15.0) * 0.5
}

/// `AnalogLeverBlockEntity.tick`: counts the timer down; true on the tick it reaches 0, when the
/// neighbours are told. Answers the new timer.
pub fn tick_timer(timer: i32) -> (i32, bool) {
    if timer > 0 {
        let next = timer - 1;
        (next, next == 0)
    } else {
        (0, false)
    }
}

/// `LerpedFloat.chase(target, 0.2, Chaser.EXP)`: each tick closes a fifth of what is left. Where
/// the handle is `age` ticks after it was at `from` and the output became `to`.
pub fn eased(from: f32, to: f32, age: f32) -> f32 {
    to + (from - to) * 0.8f32.powf(age.max(0.0))
}

/// Past this age the handle is within a hundredth of a step of its place and need not be redrawn.
pub const SETTLED_AFTER: f32 = 40.0;

/// `AnalogLeverRenderer`: the handle turns a quarter turn as the output goes from 0 to 15
/// (`state / 15 * 90`), in degrees.
pub fn handle_angle(state: f32) -> f32 {
    state / 15.0 * 90.0
}

/// `Color.mixColors`: each channel of `from` moved towards `to` by `weight`.
pub fn mix_colours(from: u32, to: u32, weight: f32) -> u32 {
    let channel = |shift: u32| {
        let a = ((from >> shift) & 0xFF) as f32;
        let b = ((to >> shift) & 0xFF) as f32;
        (a + (b - a) * weight.clamp(0.0, 1.0)) as u32 & 0xFF
    };
    channel(16) << 16 | channel(8) << 8 | channel(0)
}

/// `AnalogLeverRenderer`'s indicator colour: from dark red (0x2C0300) at 0 to red (0xCD0000) at 15.
pub fn indicator_colour(state: f32) -> u32 {
    mix_colours(0x2C0300, 0xCD0000, state / 15.0)
}

/// `FaceAttachedHorizontalDirectionalBlock.getConnectedDirection`: the direction from the block the
/// lever is attached to towards the lever (up for a floor lever, down for a ceiling one, the lever's
/// `facing` for one on a wall).
pub fn connected_direction(face: &str, facing: &str) -> &'static str {
    match face {
        "ceiling" => "down",
        "floor" => "up",
        _ => match facing {
            "north" => "north",
            "south" => "south",
            "west" => "west",
            _ => "east",
        },
    }
}

/// Vanilla `LeverBlock.getShape`, in blocks (the shape the analog lever borrows): the boxes for a
/// lever `face` and `facing`.
pub fn shape(face: &str, facing: &str) -> [f64; 6] {
    let b = |a: [f64; 6]| a.map(|v| v / 16.0);
    let along_x = matches!(facing, "east" | "west");
    match face {
        "floor" => b(if along_x {
            [4.0, 0.0, 5.0, 12.0, 6.0, 11.0]
        } else {
            [5.0, 0.0, 4.0, 11.0, 6.0, 12.0]
        }),
        "ceiling" => b(if along_x {
            [4.0, 10.0, 5.0, 12.0, 16.0, 11.0]
        } else {
            [5.0, 10.0, 4.0, 11.0, 16.0, 12.0]
        }),
        _ => b(match facing {
            "north" => [5.0, 4.0, 10.0, 11.0, 12.0, 16.0],
            "south" => [5.0, 4.0, 0.0, 11.0, 12.0, 6.0],
            "west" => [10.0, 4.0, 5.0, 16.0, 12.0, 11.0],
            _ => [0.0, 4.0, 5.0, 6.0, 12.0, 11.0],
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_output_is_kept_between_0_and_15() {
        assert_eq!(change_state(0, false), (1, true));
        assert_eq!(change_state(14, false), (15, true));
        assert_eq!(change_state(15, false), (15, false));
        assert_eq!(change_state(0, true), (0, false));
        assert_eq!(change_state(7, true), (6, true));
    }

    #[test]
    fn neighbours_are_told_when_the_timer_runs_out() {
        let mut timer = CHANGE_DELAY;
        let mut told = 0;
        for tick in 1..=20 {
            let (next, fire) = tick_timer(timer);
            timer = next;
            if fire {
                told = tick;
            }
        }
        assert_eq!(told, 15);
        assert_eq!(tick_timer(0), (0, false));
    }

    #[test]
    fn the_click_pitch_rises_with_the_output() {
        assert!((click_pitch(0) - (0.25 + 5.0 / 15.0 * 0.5)).abs() < 1e-6);
        assert!((click_pitch(15) - (0.25 + 20.0 / 15.0 * 0.5)).abs() < 1e-6);
    }

    #[test]
    fn the_handle_eases_a_fifth_at_a_time() {
        assert_eq!(eased(0.0, 10.0, 0.0), 0.0);
        assert!((eased(0.0, 10.0, 1.0) - 2.0).abs() < 1e-5);
        assert!((eased(0.0, 10.0, 2.0) - 3.6).abs() < 1e-5);
        assert!((eased(0.0, 10.0, SETTLED_AFTER) - 10.0).abs() < 0.01);
        assert!((handle_angle(15.0) - 90.0).abs() < 1e-6);
    }

    #[test]
    fn the_indicator_runs_from_dark_red_to_red() {
        assert_eq!(indicator_colour(0.0), 0x2C0300);
        assert_eq!(indicator_colour(15.0), 0xCD0000);
        let half = indicator_colour(7.5);
        assert_eq!(half >> 16, (0x2C + 0xCD) / 2);
        assert_eq!((half >> 8) & 0xFF, 1);
        assert_eq!(half & 0xFF, 0);
    }

    #[test]
    fn the_connected_direction_points_from_the_support() {
        assert_eq!(connected_direction("floor", "north"), "up");
        assert_eq!(connected_direction("ceiling", "east"), "down");
        assert_eq!(connected_direction("wall", "west"), "west");
    }

    #[test]
    fn shapes_are_vanillas_lever_boxes() {
        assert_eq!(shape("floor", "north"), [5.0 / 16.0, 0.0, 4.0 / 16.0, 11.0 / 16.0, 6.0 / 16.0, 12.0 / 16.0]);
        assert_eq!(shape("floor", "east")[0], 4.0 / 16.0);
        assert_eq!(shape("wall", "north")[2], 10.0 / 16.0);
        assert_eq!(shape("wall", "east")[3], 6.0 / 16.0);
        assert_eq!(shape("ceiling", "south")[1], 10.0 / 16.0);
    }
}
