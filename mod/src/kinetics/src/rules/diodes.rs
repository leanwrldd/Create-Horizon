//! The diodes' rules: the vanilla `DiodeBlock` that Create's Pulse Repeater, Extender, Timer and
//! the two latches build on, the three `BrassDiodeBlockEntity` counters, the Powered Latch's two
//! inputs and the stand-in values a wrench steps a delay through. No game types: the game glue is
//! `machines/diodes.rs`.
//!
//! Known differences from Create, for the README:
//! - Create schedules block ticks (`neighborChanged` then `scheduleTick(pos, this, delay)`); the
//!   glue polls the inputs each tick from a block entity and counts the delay itself, which has the
//!   same timing (a change is seen up to a tick later than a neighbour update would announce it).
//!   `TickPriority` is not used.
//! - The latches have no block entity in Create; here they have one, only to poll.
//! - The delay is set by a Wrench (see [`STEPS`]), not Create's scroll value box.

use crate::Dir;

// --- the stored value ---------------------------------------------------------------------------

/// `BrassDiodeBlockEntity.addBehaviours`: the interval is `between(2, 60 * 20 * 60)` ticks: 2
/// ticks to an hour.
pub const MIN_VALUE: i32 = 2;
pub const MAX_VALUE: i32 = 60 * 20 * 60;

/// The three brass diodes and their counters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Brass {
    Repeater,
    Extender,
    Timer,
}

/// `BrassDiodeBlockEntity.defaultValue`: 2 ticks, 20 (a second) for the Pulse Timer
/// (`PulseTimerBlockEntity.defaultValue`).
pub const fn default_value(kind: Brass) -> i32 {
    match kind {
        Brass::Timer => 20,
        _ => 2,
    }
}

/// What a Wrench steps a delay through, standing in for Create's scroll value box. The box offers
/// 2 to 60 ticks, 1 to 60 seconds (`* 20`) and 1 to 60 minutes (`* 20 * 60`)
/// (`BrassDiodeScrollValueBehaviour.setValueSettings`); these are the values worth reaching: the
/// short ticks, then seconds and minutes up to the hour.
pub const STEPS: [i32; 16] = [
    2, 3, 4, 5, 10, 20, 40, 60, 100, 200, 600, 1200, 6000, 12000, 36000, 72000,
];

/// The value after `value` in [`STEPS`], coming round to the first after the last.
pub fn next_step(value: i32) -> i32 {
    STEPS
        .iter()
        .copied()
        .find(|&step| step > value)
        .unwrap_or(STEPS[0])
}

/// `BrassDiodeBlockEntity.format`: "20t", "3s", "5m".
pub fn format(value: i32) -> String {
    if value < 60 {
        format!("{value}t")
    } else if value < 20 * 60 {
        format!("{}s", value / 20)
    } else {
        format!("{}m", value / 20 / 60)
    }
}

/// `BrassDiodeBlockEntity.onMaxDelayChanged`: a new maximum cuts the counter down to it.
pub fn clamp_state(state: i32, max: i32) -> i32 {
    state.clamp(0, max)
}

/// `BrassDiodeBlockEntity.getProgress`: how far through the interval the counter is (0 to 1).
pub fn progress(state: i32, max: i32) -> f32 {
    let max = max.max(2);
    state.clamp(0, max) as f32 / max as f32
}

/// What one tick of a brass diode's counter decides: its new `State` and its `powering` state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub state: i32,
    pub powering: bool,
}

/// `PulseRepeaterBlockEntity.updateState`, `PulseExtenderBlockEntity.updateState` and
/// `PulseTimerBlockEntity.updateState`, run each tick with the block's `powered` and `powering`
/// states and the maximum (`ScrollValue`).
pub fn brass_step(kind: Brass, state: i32, max: i32, powered: bool, powering: bool) -> Step {
    let at_min = state <= 0;
    let keep = Step { state, powering };
    match kind {
        Brass::Repeater => {
            if at_min && !powered {
                return keep;
            }
            if state > max + 1 {
                return Step {
                    state: if !powered && !powering { 0 } else { state },
                    powering,
                };
            }
            let state = state + 1;
            let mut powering = powering;
            if state == max - 1 && !powering {
                powering = true;
            }
            if state == max + 1 && powering {
                powering = false;
            }
            Step { state, powering }
        }
        Brass::Extender => {
            if at_min && !powered {
                return keep;
            }
            if at_min || powered {
                return Step { state: max, powering: true };
            }
            if state == 1 {
                return Step { state: 0, powering: false };
            }
            Step { state: state - 1, powering }
        }
        Brass::Timer => {
            let state = if powered || state >= max - 1 { 0 } else { state + 1 };
            let should_power = !powered && (if max == 2 { state == 0 } else { state <= 1 });
            Step { state, powering: should_power }
        }
    }
}

// --- the diode block (vanilla `DiodeBlock`) -----------------------------------------------------

/// `BrassDiodeBlock.getDelay`: 2 game ticks (one redstone tick).
pub const BRASS_DELAY: i32 = 2;
/// `ToggleLatchBlock.getDelay` (and so the Powered Latch's): 1 game tick.
pub const LATCH_DELAY: i32 = 1;

/// `DiodeBlock.getInputSignal`'s extra: a redstone wire's own power counts even where the wire
/// doesn't connect to the diode.
pub fn input_with_wire(signal: i32, wire_power: i32) -> i32 {
    if signal >= 15 {
        signal
    } else {
        signal.max(wire_power)
    }
}

/// `DiodeBlock.checkTickOnNeighbor`: a tick is wanted when `powered` differs from what the input
/// says and none is waiting already.
pub fn wants_tick(powered: bool, should_turn_on: bool, waiting: bool) -> bool {
    powered != should_turn_on && !waiting
}

/// What `DiodeBlock.tick` does: the new `powered` and whether to schedule another tick.
pub fn diode_tick(powered: bool, should_turn_on: bool) -> (bool, bool) {
    if powered && !should_turn_on {
        (false, false)
    } else if !powered {
        (true, !should_turn_on)
    } else {
        (powered, false)
    }
}

/// `BrassDiodeBlock.getOutputSignal`: powering, flipped by `inverted`.
pub fn brass_output(powering: bool, inverted: bool) -> i32 {
    if powering ^ inverted { 15 } else { 0 }
}

/// `ToggleLatchBlock.getOutputSignal`: powering.
pub fn latch_output(powering: bool) -> i32 {
    if powering { 15 } else { 0 }
}

/// The Powered Toggle Latch's state (`ToggleLatchBlock`): `powered` follows the back input,
/// `powering` is what it gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Toggle {
    pub powered: bool,
    pub powering: bool,
}

/// `ToggleLatchBlock.tick`: `DiodeBlock.tick`, then a rise of `powered` flips `powering`. Answers
/// the new state and whether another tick is wanted.
pub fn toggle_tick(state: Toggle, should_turn_on: bool) -> (Toggle, bool) {
    let (powered, again) = diode_tick(state.powered, should_turn_on);
    let mut powering = state.powering;
    if powered && !state.powered {
        powering = !powering;
    }
    (Toggle { powered, powering }, again)
}

/// The Powered Latch's state (`PoweredLatchBlock`): the back input, the side input, and what it
/// gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Latch {
    pub powered: bool,
    pub powering: bool,
    pub powered_side: bool,
}

/// `PoweredLatchBlock.checkTickOnNeighbor`: a tick is wanted when either input differs from the
/// state and none is waiting.
pub fn latch_wants_tick(state: Latch, back: bool, side: bool, waiting: bool) -> bool {
    !waiting && (state.powered != back || state.powered_side != side)
}

/// `PoweredLatchBlock.tick`: the back input switches it on, the side input off; both are
/// tracked. `back` and `side` are what the inputs say now.
pub fn latch_tick(state: Latch, back: bool, side: bool) -> Latch {
    let mut next = state;
    if state.powered != back {
        next.powered = back;
        if back {
            next.powering = true;
        } else if state.powered_side {
            next.powering = false;
        }
    }
    if state.powered_side != side {
        next.powered_side = side;
        if side {
            next.powering = false;
        } else if state.powered {
            next.powering = true;
        }
    }
    next
}

/// `PoweredLatchBlock.activated`: a click flips it only while the two inputs agree.
pub fn latch_click(state: Latch) -> Option<Latch> {
    if state.powered != state.powered_side {
        return None;
    }
    Some(Latch { powering: !state.powering, ..state })
}

/// `ItemInteractionResult` sounds: `LEVER_CLICK` at 0.3 volume, pitched 0.6 when the click turned
/// something on and 0.5 when it turned it off (`BrassDiodeBlock.toggle`, `ToggleLatchBlock`).
pub fn click_pitch(was_on: bool) -> f32 {
    if !was_on { 0.6 } else { 0.5 }
}

/// `DiodeBlock.FACING` is the side the input comes from; the output goes out of the opposite
/// side (`getSignal` answers for `side == FACING`).
pub fn output_side(facing: Dir) -> Dir {
    facing.opposite()
}

/// The side inputs of a Powered Latch (`PoweredLatchBlock.isPoweredOnSides`): the sides to the
/// left and right of its facing (`getClockWise` and `getCounterClockWise`).
pub fn latch_sides(facing: Dir) -> [Dir; 2] {
    [clockwise(facing), clockwise(facing).opposite()]
}

/// `Direction.getClockWise` about y, seen from above: north to east to south to west.
pub fn clockwise(facing: Dir) -> Dir {
    match facing {
        Dir::North => Dir::East,
        Dir::East => Dir::South,
        Dir::South => Dir::West,
        Dir::West => Dir::North,
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs a brass diode for `ticks` ticks with `powered` held, folding `powering` back in as
    /// the block state would: the states `powering` took, from the first tick on.
    fn run(kind: Brass, max: i32, ticks: usize, powered: impl Fn(usize) -> bool) -> Vec<(i32, bool)> {
        let (mut state, mut powering) = (0, false);
        (0..ticks)
            .map(|tick| {
                let step = brass_step(kind, state, max, powered(tick), powering);
                state = step.state;
                powering = step.powering;
                (state, powering)
            })
            .collect()
    }

    #[test]
    fn defaults_are_creates() {
        assert_eq!(default_value(Brass::Repeater), 2);
        assert_eq!(default_value(Brass::Extender), 2);
        assert_eq!(default_value(Brass::Timer), 20);
        assert_eq!((MIN_VALUE, MAX_VALUE), (2, 72000));
    }

    #[test]
    fn the_steps_stay_in_range_and_come_round() {
        assert!(STEPS.iter().all(|&s| (MIN_VALUE..=MAX_VALUE).contains(&s)));
        assert_eq!(next_step(2), 3);
        assert_eq!(next_step(20), 40);
        assert_eq!(next_step(7), 10);
        assert_eq!(next_step(72000), 2);
        assert_eq!(next_step(500), 600);
        // Every value that can be stored leads somewhere.
        for value in [2, 20, 59, 61, 1000, 71999] {
            let next = next_step(value);
            assert!(next == 2 || next > value);
        }
    }

    #[test]
    fn values_are_written_as_creates_formatter_does() {
        assert_eq!(format(2), "2t");
        assert_eq!(format(59), "59t");
        assert_eq!(format(60), "3s");
        assert_eq!(format(1199), "59s");
        assert_eq!(format(1200), "1m");
        assert_eq!(format(72000), "60m");
    }

    #[test]
    fn progress_is_the_counter_over_the_maximum() {
        assert_eq!(progress(0, 20), 0.0);
        assert_eq!(progress(10, 20), 0.5);
        assert_eq!(progress(40, 20), 1.0);
        assert_eq!(progress(1, 1), 0.5);
        assert_eq!(clamp_state(50, 20), 20);
    }

    #[test]
    fn the_repeater_pulses_after_its_delay() {
        // A signal held from tick 0 with a delay of 5: the counter climbs, `powering` goes true
        // when it reaches max - 1 and false at max + 1.
        let seen = run(Brass::Repeater, 5, 8, |_| true);
        assert_eq!(seen[0], (1, false));
        assert_eq!(seen[3], (4, true));
        assert_eq!(seen[4], (5, true));
        assert_eq!(seen[5], (6, false));
        // It stops at max + 2 while the input is on, so it pulses only once.
        assert_eq!(seen[6], (7, false));
        assert_eq!(seen[7], (7, false));
    }

    #[test]
    fn the_repeater_rests_when_the_input_is_off_and_resets_afterwards() {
        let seen = run(Brass::Repeater, 3, 10, |t| t < 2);
        // Off from tick 2 on it still counts (the counter was running), then resets once past
        // the end with nothing powering.
        assert!(seen.iter().any(|&(s, _)| s == 0) || seen.last().unwrap().0 == 0);
        let idle = run(Brass::Repeater, 3, 5, |_| false);
        assert!(idle.iter().all(|&s| s == (0, false)));
    }

    #[test]
    fn the_extender_holds_its_output_for_the_set_time() {
        // A one tick pulse in: output goes on at once and holds for `max` ticks.
        let seen = run(Brass::Extender, 4, 8, |t| t == 0);
        assert_eq!(seen[0], (4, true));
        assert_eq!(seen[1], (3, true));
        assert_eq!(seen[2], (2, true));
        assert_eq!(seen[3], (1, true));
        assert_eq!(seen[4], (0, false));
        assert_eq!(seen[5], (0, false));
        // While powered it keeps reloading.
        let held = run(Brass::Extender, 4, 5, |_| true);
        assert!(held.iter().all(|&s| s == (4, true)));
    }

    #[test]
    fn the_timer_pulses_every_interval_and_resets_when_powered() {
        let seen = run(Brass::Timer, 5, 12, |_| false);
        let pulses: Vec<usize> = (0..seen.len()).filter(|&i| seen[i].1).collect();
        // State: 1,2,3,4,0,1,2,3,4,0,...: powering while state <= 1.
        assert_eq!(seen[0], (1, true));
        assert_eq!(seen[1], (2, false));
        assert_eq!(seen[4], (0, true));
        assert_eq!(pulses, vec![0, 4, 5, 9, 10]);
        let held = run(Brass::Timer, 5, 6, |_| true);
        assert!(held.iter().all(|&s| s == (0, false)));
        // The shortest timer pulses on every other tick: state == 0.
        let quick = run(Brass::Timer, 2, 4, |_| false);
        assert_eq!(quick, vec![(1, false), (0, true), (1, false), (0, true)]);
    }

    #[test]
    fn a_diode_turns_on_after_its_delay_and_off_again() {
        assert!(wants_tick(false, true, false));
        assert!(!wants_tick(false, true, true));
        assert!(!wants_tick(true, true, false));
        assert_eq!(diode_tick(false, true), (true, false));
        // Turned on while the input already went away: it schedules its own turning off.
        assert_eq!(diode_tick(false, false), (true, true));
        assert_eq!(diode_tick(true, false), (false, false));
        assert_eq!(diode_tick(true, true), (true, false));
    }

    #[test]
    fn a_toggle_latch_flips_on_each_rising_edge() {
        let mut state = Toggle::default();
        (state, _) = toggle_tick(state, true);
        assert_eq!(state, Toggle { powered: true, powering: true });
        (state, _) = toggle_tick(state, true);
        assert_eq!(state, Toggle { powered: true, powering: true });
        (state, _) = toggle_tick(state, false);
        assert_eq!(state, Toggle { powered: false, powering: true });
        (state, _) = toggle_tick(state, true);
        assert_eq!(state, Toggle { powered: true, powering: false });
    }

    #[test]
    fn a_powered_latch_is_set_by_the_back_and_reset_by_the_side() {
        let mut state = Latch::default();
        state = latch_tick(state, true, false);
        assert_eq!(state, Latch { powered: true, powering: true, powered_side: false });
        // The back goes away: it stays on.
        state = latch_tick(state, false, false);
        assert!(state.powering);
        // The side switches it off.
        state = latch_tick(state, false, true);
        assert!(!state.powering && state.powered_side);
        // The side going away leaves it off.
        state = latch_tick(state, false, false);
        assert!(!state.powering);
        // Back and side together: the side wins as it is handled last.
        let both = latch_tick(Latch::default(), true, true);
        assert!(!both.powering);
        // The side going while the back is still held turns it back on.
        let again = latch_tick(both, true, false);
        assert!(again.powering);
    }

    #[test]
    fn a_latch_wants_a_tick_when_an_input_changed() {
        let state = Latch::default();
        assert!(!latch_wants_tick(state, false, false, false));
        assert!(latch_wants_tick(state, true, false, false));
        assert!(latch_wants_tick(state, false, true, false));
        assert!(!latch_wants_tick(state, true, true, true));
    }

    #[test]
    fn a_powered_latch_can_be_clicked_only_while_its_inputs_agree() {
        let off = Latch::default();
        assert_eq!(latch_click(off).unwrap().powering, true);
        let back = Latch { powered: true, powering: true, powered_side: false };
        assert_eq!(latch_click(back), None);
        let both = Latch { powered: true, powering: false, powered_side: true };
        assert_eq!(latch_click(both).unwrap().powering, true);
    }

    #[test]
    fn outputs_follow_the_states() {
        assert_eq!(brass_output(true, false), 15);
        assert_eq!(brass_output(false, false), 0);
        assert_eq!(brass_output(false, true), 15);
        assert_eq!(brass_output(true, true), 0);
        assert_eq!(latch_output(true), 15);
        assert_eq!(latch_output(false), 0);
    }

    #[test]
    fn the_wire_counts_unless_the_signal_is_full() {
        assert_eq!(input_with_wire(0, 7), 7);
        assert_eq!(input_with_wire(15, 3), 15);
        assert_eq!(input_with_wire(4, 2), 4);
    }

    #[test]
    fn sides_and_turns_are_clockwise_from_above() {
        assert_eq!(clockwise(Dir::North), Dir::East);
        assert_eq!(clockwise(Dir::West), Dir::North);
        assert_eq!(latch_sides(Dir::North), [Dir::East, Dir::West]);
        assert_eq!(output_side(Dir::South), Dir::North);
        assert_eq!(click_pitch(false), 0.6);
        assert_eq!(click_pitch(true), 0.5);
    }
}
