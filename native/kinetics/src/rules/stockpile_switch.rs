//! The Threshold Switch's rules (`ThresholdSwitchBlockEntity`): how full the block it looks at
//! is, when its signal turns on and off, and the glass it shows. No game types.
//!
//! Ported from Create `content/redstone/thresholdSwitch/ThresholdSwitchBlockEntity`
//! (`updateCurrentLevel`, `shouldBePowered`) and `ThresholdSwitchScreen` (the ranges and the
//! step of its two inputs).
//!
//! Known differences from Create: the presets of [`presets`] stand in for the screen's two
//! scroll inputs.

/// The saved defaults: `onWhenAbove = 128` and `offWhenBelow = 64`.
pub const ON_ABOVE: i32 = 128;
pub const OFF_BELOW: i32 = 64;
/// `scheduleBlockTick`: ticks from a change of state to the change of the signal.
pub const POWER_DELAY: i32 = 2;
/// `setLazyTickRate(10)`: ticks between looks at the block it faces.
pub const LAZY_TICK: i64 = 10;
/// A stack of items: the step of the screen's input in stacks.
pub const STACK: i32 = 64;
/// A bucket: the step of the screen's input for fluids (`valueStep = 1000`).
pub const BUCKET: i32 = 1000;
/// `Math.min(stackInSlot.getOrDefault(MAX_STACK_SIZE, 64), slotLimit)` for an empty slot: 64.
pub const EMPTY_SLOT_SPACE: i32 = 64;

/// What the switch looks at (`ThresholdType`, less `CUSTOM`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Unsupported,
    Items,
    Fluids,
}

/// How full the block it looks at is, in the unit of its kind: items or millibuckets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Level {
    pub min: i32,
    pub current: i32,
    pub max: i32,
}

/// The room in one inventory slot (`updateCurrentLevel`): the item's stack size, or 64 for an
/// empty one, never more than the slot's limit.
pub fn slot_space(stack_size: Option<u32>, slot_limit: u32) -> i32 {
    stack_size.map_or(EMPTY_SLOT_SPACE, |size| size.min(slot_limit) as i32)
}

/// `Mth.clamp(currentLevel, currentMinLevel, currentMaxLevel)`.
pub fn clamp(level: i32, min: i32, max: i32) -> i32 {
    level.max(min).min(max.max(min))
}

/// The switch's own state after a look: `redstoneState` turns off at or below `off_below` and on
/// at or above `on_above`, and keeps what it was between them.
pub fn next_state(powered: bool, current: i32, on_above: i32, off_below: i32) -> bool {
    if powered && current <= off_below {
        false
    } else if !powered && current >= on_above {
        true
    } else {
        powered
    }
}

/// `shouldBePowered`: the signal, which the Invert button flips.
pub fn should_be_powered(inverted: bool, state: bool) -> bool {
    inverted != state
}

/// The `level` property the glass shows (`displayLevel`): 0 when empty, else 1 to 5 by how full.
pub fn display_level(level: Level) -> i32 {
    if level.current > 0 {
        let normed = (level.current - level.min) as f32 / (level.max - level.min) as f32;
        // Java's `(int)` of NaN is 0, as Rust's `as`.
        (1.0 + normed * 4.0) as i32
    } else {
        0
    }
}

/// The step of the screen's inputs for a kind of target.
pub fn unit(kind: Kind) -> i32 {
    match kind {
        Kind::Fluids => BUCKET,
        _ => 1,
    }
}

/// The thresholds the stand-in for the screen steps through, as `(on_above, off_below)`: Create's
/// defaults, then a quarter-to-half, a half-to-three-quarters and a three-quarters-to-full
/// range of what the target holds, and last "as soon as there is anything, until it is empty".
/// Each is rounded down to the screen's step (stacks for a big inventory, buckets for fluids) and
/// kept inside what the screen allows: the upper at least one step above the lower.
pub fn presets(min: i32, max: i32, kind: Kind) -> Vec<(i32, i32)> {
    let step = match kind {
        Kind::Fluids => BUCKET,
        _ if max - min >= 4 * STACK => STACK,
        _ => 1,
    };
    let fix = |on: i32, off: i32| {
        let off = (off.max(min) / step * step).min(max - step).max(min);
        let on = (on / step * step).max(off + step).min(max.max(off + step));
        (on, off)
    };
    let span = max - min;
    let at = |quarters: i32| min + (i64::from(span) * i64::from(quarters) / 4) as i32;
    let mut out: Vec<(i32, i32)> = Vec::new();
    for preset in [
        (ON_ABOVE, OFF_BELOW),
        fix(at(2), at(1)),
        fix(at(3), at(2)),
        fix(max, at(3)),
        fix(min + step, min),
    ] {
        // A small target can round two of them to the same thresholds: stepping would stick.
        if !out.contains(&preset) {
            out.push(preset);
        }
    }
    out
}

/// The preset after the one `(on_above, off_below)` is, wrapping; the first when it is none.
pub fn next_preset(presets: &[(i32, i32)], on_above: i32, off_below: i32) -> (i32, i32) {
    let next = presets
        .iter()
        .position(|p| *p == (on_above, off_below))
        .map_or(0, |i| (i + 1) % presets.len());
    presets[next]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_signal_turns_on_at_the_upper_threshold_and_off_at_the_lower() {
        // Starts off; rising past 128 turns it on, staying between keeps it, 64 turns it off.
        assert!(!next_state(false, 127, ON_ABOVE, OFF_BELOW));
        assert!(next_state(false, 128, ON_ABOVE, OFF_BELOW));
        assert!(next_state(true, 100, ON_ABOVE, OFF_BELOW));
        assert!(next_state(true, 65, ON_ABOVE, OFF_BELOW));
        assert!(!next_state(true, 64, ON_ABOVE, OFF_BELOW));
        assert!(!next_state(false, 100, ON_ABOVE, OFF_BELOW));
    }

    #[test]
    fn inverting_flips_the_signal() {
        assert!(should_be_powered(false, true));
        assert!(!should_be_powered(false, false));
        assert!(!should_be_powered(true, true));
        assert!(should_be_powered(true, false));
    }

    #[test]
    fn the_glass_shows_how_full_it_is() {
        let at = |current| display_level(Level { min: 0, current, max: 1000 });
        assert_eq!(at(0), 0);
        assert_eq!(at(1), 1);
        assert_eq!(at(250), 2);
        assert_eq!(at(500), 3);
        assert_eq!(at(750), 4);
        assert_eq!(at(1000), 5);
        // A target with no room reads as empty, not as a division by zero.
        assert_eq!(display_level(Level { min: 0, current: 0, max: 0 }), 0);
    }

    #[test]
    fn a_slot_holds_its_stack_size_or_64_when_empty() {
        assert_eq!(slot_space(Some(16), 64), 16);
        assert_eq!(slot_space(Some(64), 64), 64);
        assert_eq!(slot_space(None, 64), 64);
        assert_eq!(slot_space(Some(99), 64), 64);
        assert_eq!(clamp(70, 0, 60), 60);
        assert_eq!(clamp(-1, 0, 60), 0);
    }

    #[test]
    fn the_presets_start_at_create_s_defaults_and_stay_in_range() {
        // A chest: 27 slots of 64.
        let chest = presets(0, 27 * 64, Kind::Items);
        assert_eq!(chest[0], (128, 64));
        for (on, off) in &chest {
            assert!(*off >= 0 && *on <= 27 * 64 && on > off, "{on} {off}");
        }
        // Whole stacks for a big inventory (a half rounds down to 13 of 27 stacks).
        assert_eq!(chest[1], (832, 384));
        assert!(chest[1..].iter().all(|(on, off)| on % 64 == 0 && off % 64 == 0));
        assert_eq!(chest[4], (64, 0));
        // A tank of 8 buckets: whole buckets.
        let tank = presets(0, 8000, Kind::Fluids);
        assert_eq!(tank[1], (4000, 2000));
        assert_eq!(tank[3], (8000, 6000));
        assert_eq!(tank[4], (1000, 0));
        // A hopper: 5 slots, single items.
        let hopper = presets(0, 5 * 64, Kind::Items);
        assert!(hopper[1..].iter().all(|(on, off)| on > off && *on <= 5 * 64));
        // A target with no room at all still gives well-formed thresholds, each only once.
        let none = presets(0, 0, Kind::Items);
        assert!(none.iter().all(|(on, off)| on > off));
        assert_eq!(none.len(), 2);
        let mut seen = Vec::new();
        for preset in presets(0, 5 * 64, Kind::Items) {
            assert!(!seen.contains(&preset));
            seen.push(preset);
        }
    }

    #[test]
    fn stepping_goes_round_and_starts_over_from_anything_else() {
        let list = presets(0, 8000, Kind::Fluids);
        let mut now = (ON_ABOVE, OFF_BELOW);
        let mut seen = Vec::new();
        for _ in 0..list.len() {
            now = next_preset(&list, now.0, now.1);
            seen.push(now);
        }
        assert_eq!(seen[0], list[1]);
        assert_eq!(*seen.last().unwrap(), list[0]);
        assert_eq!(next_preset(&list, 5, 3), list[0]);
    }
}
