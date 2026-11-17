//! The Sequenced Gearshift's instruction machine and the edits that stand in for its screen
//! (`SequencedGearshiftBlockEntity`, `Instruction`, `SequencerInstructions`,
//! `InstructionSpeedModifiers`, `SequencedGearshiftBlock`). How the output side's speed is carried
//! through the network is `create_kinetics::Kind::sequenced` (`Joint::Sequenced`).
//!
//! A sequence is up to five instructions. A rising redstone edge starts it from the first;
//! each instruction turns the output (by an angle, or "a distance" that a piston, pulley or
//! gantry would move), waits (a delay in ticks), waits for another redstone pulse, or ends it.
//! While a turning instruction runs the output side turns at the input speed times the
//! instruction's modifier (forward 2x, forward, back, back 2x); otherwise it stands still.
//!
//! Known differences from Create, for the README:
//! - Create passes the instruction's target on down the network (`SequenceContext`) so a bearing,
//!   piston, pulley or gantry stops at exactly the angle or distance. Here nothing downstream
//!   hears it, so the instruction instead runs for exactly the ticks the turn needs, rounded up
//!   ([`TRIMMED_OVERSHOOT`] where Create adds [`OVERSHOOT`] ticks it later cuts off): the output turns
//!   up to one tick's worth too far.
//! - Create counts the progress of an interrupted instruction at the input's speed whatever the
//!   modifier; here it is counted at the output's, which is what the instruction measures.
//! - A turn at zero speed can't finish: Create's duration overflows to a negative number and the
//!   instruction hangs; here it is simply as long as can be counted.
//! - The screen is replaced by clicks (see [`Cursor`]) and the instruction fields move in
//!   steps rather than by any number.

use crate::{Axis, Dir};

/// The most instructions a sequence holds (`Vector<>(5)`).
pub const MAX_INSTRUCTIONS: usize = 5;
/// Ticks Create adds to a turning instruction because "target will stop early" downstream.
pub const OVERSHOOT: i32 = 2;
/// What this port adds: nothing hears the target, so no extra ticks.
pub const TRIMMED_OVERSHOOT: i32 = 0;

/// `KineticBlockEntity.convertToAngular`: degrees a tick at `speed` RPM.
pub fn to_angular(speed: f32) -> f32 {
    speed * 360.0 / 60.0 / 20.0
}

/// `KineticBlockEntity.convertToLinear`: blocks a tick at `speed` RPM.
pub fn to_linear(speed: f32) -> f32 {
    speed / 512.0
}

/// `SequencerInstructions`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    TurnAngle,
    TurnDistance,
    Delay,
    Await,
    End,
}

impl Op {
    pub const ALL: [Op; 5] = [Op::TurnAngle, Op::TurnDistance, Op::Delay, Op::Await, Op::End];

    /// The enum constant's name, as `NBTHelper.writeEnum` saves it.
    pub const fn name(self) -> &'static str {
        match self {
            Op::TurnAngle => "TURN_ANGLE",
            Op::TurnDistance => "TURN_DISTANCE",
            Op::Delay => "DELAY",
            Op::Await => "AWAIT",
            Op::End => "END",
        }
    }

    pub fn from_name(name: &str) -> Option<Op> {
        Op::ALL.into_iter().find(|op| op.name() == name)
    }

    /// `hasValueParameter`.
    pub const fn has_value(self) -> bool {
        matches!(self, Op::TurnAngle | Op::TurnDistance | Op::Delay)
    }

    /// `hasSpeedParameter`.
    pub const fn has_speed(self) -> bool {
        matches!(self, Op::TurnAngle | Op::TurnDistance)
    }

    /// `maxValue`.
    pub const fn max_value(self) -> i32 {
        match self {
            Op::TurnAngle => 360,
            Op::TurnDistance => 128,
            Op::Delay => 600,
            _ => -1,
        }
    }

    /// `shiftStep`.
    pub const fn shift_step(self) -> i32 {
        match self {
            Op::TurnAngle => 45,
            Op::TurnDistance => 5,
            Op::Delay => 20,
            _ => -1,
        }
    }

    /// `defaultValue`.
    pub const fn default_value(self) -> i32 {
        match self {
            Op::TurnAngle => 90,
            Op::TurnDistance => 5,
            Op::Delay => 10,
            _ => -1,
        }
    }

    /// `needsPropagation`: the instructions a bearing, piston or pulley downstream would stop at.
    pub const fn needs_propagation(self) -> bool {
        matches!(self, Op::TurnAngle | Op::TurnDistance)
    }

    /// The names the screen gives it (`gui.sequenced_gearshift.instruction.*`, the descriptive ones).
    pub const fn label(self) -> &'static str {
        match self {
            Op::TurnAngle => "Turn by angle",
            Op::TurnDistance => "Turn to move Piston/Pulley/Gantry",
            Op::Delay => "Timed Delay",
            Op::Await => "Await new Redstone Pulse",
            Op::End => "End",
        }
    }

    /// The values a click steps through (the screen takes any number up to `max_value`): the
    /// multiples of the shift step for an angle, and short lists for the other two.
    pub const fn steps(self) -> &'static [i32] {
        match self {
            Op::TurnAngle => &[45, 90, 135, 180, 225, 270, 315, 360],
            Op::TurnDistance => &[1, 2, 3, 4, 5, 8, 10, 16, 20, 32, 64, 128],
            Op::Delay => &[10, 20, 40, 60, 100, 200, 300, 400, 600],
            _ => &[],
        }
    }
}

/// `InstructionSpeedModifiers`, in the order of the enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modifier {
    ForwardFast,
    Forward,
    Back,
    BackFast,
}

impl Modifier {
    pub const ALL: [Modifier; 4] = [
        Modifier::ForwardFast,
        Modifier::Forward,
        Modifier::Back,
        Modifier::BackFast,
    ];

    /// `value`: the multiple of the input speed.
    pub const fn value(self) -> i32 {
        match self {
            Modifier::ForwardFast => 2,
            Modifier::Forward => 1,
            Modifier::Back => -1,
            Modifier::BackFast => -2,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Modifier::ForwardFast => "FORWARD_FAST",
            Modifier::Forward => "FORWARD",
            Modifier::Back => "BACK",
            Modifier::BackFast => "BACK_FAST",
        }
    }

    pub fn from_name(name: &str) -> Option<Modifier> {
        Modifier::ALL.into_iter().find(|m| m.name() == name)
    }

    /// `getByModifier`: the one with that value, forward by default.
    pub fn by_value(value: i32) -> Modifier {
        Modifier::ALL
            .into_iter()
            .find(|m| m.value() == value)
            .unwrap_or(Modifier::Forward)
    }

    /// The label drawn on the screen's button.
    pub const fn arrows(self) -> &'static str {
        match self {
            Modifier::ForwardFast => ">>",
            Modifier::Forward => "->",
            Modifier::Back => "<-",
            Modifier::BackFast => "<<",
        }
    }

    /// `gui.sequenced_gearshift.speed.*`.
    pub const fn label(self) -> &'static str {
        match self {
            Modifier::ForwardFast => "Double speed, Forwards",
            Modifier::Forward => "Input speed, Forwards",
            Modifier::Back => "Input speed, Reversed",
            Modifier::BackFast => "Double speed, Reversed",
        }
    }
}

/// One line of a sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Instruction {
    pub op: Op,
    pub modifier: Modifier,
    pub value: i32,
}

impl Instruction {
    /// `new Instruction(op, 1)`: `END` and `AWAIT` keep the value 1.
    pub const fn new(op: Op) -> Self {
        Instruction {
            op,
            modifier: Modifier::Forward,
            value: 1,
        }
    }

    pub const fn with(op: Op, value: i32) -> Self {
        Instruction {
            op,
            modifier: Modifier::Forward,
            value,
        }
    }

    /// `getDuration`: ticks the instruction takes from `progress` at the input `speed`;
    /// -1 for one that waits for redstone. `overshoot` is [`OVERSHOOT`] in Create.
    pub fn duration(self, progress: f32, speed: f32, overshoot: i32) -> i32 {
        let speed = (speed * self.modifier.value() as f32).abs();
        let target = f64::from(self.value) - f64::from(progress);
        // The most that can be counted, for a turn that never ends at no speed.
        let ticks = |per_tick: f64| -> i32 {
            if per_tick <= 0.0 {
                return i32::MAX;
            }
            let ticks = (target / per_tick).ceil();
            if ticks >= f64::from(i32::MAX - overshoot) {
                i32::MAX
            } else {
                (ticks as i32).saturating_add(overshoot)
            }
        };
        match self.op {
            Op::TurnAngle => ticks(f64::from(to_angular(speed))),
            Op::TurnDistance => ticks(f64::from(to_linear(speed))),
            Op::Delay => target as i32,
            Op::Await => -1,
            Op::End => 0,
        }
    }

    /// `getTickProgress`: what one tick adds to the progress at `speed`.
    pub fn tick_progress(self, speed: f32) -> f32 {
        match self.op {
            Op::TurnAngle => to_angular(speed),
            Op::TurnDistance => to_linear(speed),
            Op::Delay => 1.0,
            Op::Await | Op::End => 0.0,
        }
    }

    /// `getSpeedModifier`: what the output turns by while the instruction runs.
    pub fn speed_modifier(self) -> i32 {
        if self.op.needs_propagation() {
            self.modifier.value()
        } else {
            0
        }
    }

    /// `formatValue`.
    pub fn format_value(self) -> String {
        match self.op {
            Op::TurnAngle => format!("{}\u{b0}", self.value),
            Op::TurnDistance => format!("{}m", self.value),
            Op::Delay if self.value >= 20 => format!("{}s", self.value / 20),
            Op::Delay => format!("{}t", self.value),
            _ => self.value.to_string(),
        }
    }
}

/// `Instruction.createDefault`: turn 90 degrees, end.
pub fn default_program() -> Vec<Instruction> {
    vec![Instruction::with(Op::TurnAngle, 90), Instruction::new(Op::End)]
}

/// A program as the screen holds it: five slots, the unused ones `End`.
pub fn padded(program: &[Instruction]) -> Vec<Instruction> {
    let mut program: Vec<Instruction> = program.iter().copied().take(MAX_INSTRUCTIONS).collect();
    while program.len() < MAX_INSTRUCTIONS {
        program.push(Instruction::new(Op::End));
    }
    program
}

/// What the redstone does to the block's `state` property (0 idle, 1 to 5 the instruction
/// running) after a call: `Some(n)` sets it, `None` leaves it.
pub type State = Option<u8>;

/// The running sequence (`SequencedGearshiftBlockEntity`'s fields).
#[derive(Clone, Debug, PartialEq)]
pub struct Sequence {
    pub instructions: Vec<Instruction>,
    /// `currentInstruction`; -1 when idle.
    pub current: i32,
    /// `currentInstructionDuration`; -1 while waiting for redstone.
    pub duration: i32,
    /// `currentInstructionProgress`.
    pub progress: f32,
    pub timer: i32,
    /// `poweredPreviously`.
    pub powered_previously: bool,
    /// Extra ticks on a turning instruction (not saved): [`TRIMMED_OVERSHOOT`] unless a test says otherwise.
    pub overshoot: i32,
}

impl Default for Sequence {
    fn default() -> Self {
        Sequence {
            instructions: default_program(),
            current: -1,
            duration: -1,
            progress: 0.0,
            timer: 0,
            powered_previously: false,
            overshoot: TRIMMED_OVERSHOOT,
        }
    }
}

impl Sequence {
    pub fn is_idle(&self) -> bool {
        self.current == -1
    }

    pub fn instruction(&self, index: i32) -> Option<Instruction> {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.instructions.get(i).copied())
    }

    /// `getModifier`: how the output side turns compared with the input side now (0 stands
    /// still, which is what a delay, a wait and an idle block do).
    pub fn modifier(&self) -> i32 {
        if self.current >= self.instructions.len() as i32 || self.is_idle() {
            return 0;
        }
        self.instruction(self.current).map_or(0, Instruction::speed_modifier)
    }

    /// `run`: starts instruction `index`, or ends the sequence when there is none or it is `End`.
    /// `signal` is whether redstone power is on the block now (`hasNeighborSignal`).
    pub fn run(&mut self, index: i32, theoretical: f32, signal: bool) -> State {
        match self.instruction(index) {
            Some(instruction) if instruction.op != Op::End => {
                self.duration = instruction.duration(0.0, theoretical, self.overshoot);
                self.current = index;
                self.progress = 0.0;
                self.timer = 0;
                Some((index + 1) as u8)
            }
            _ => {
                self.current = -1;
                self.duration = -1;
                self.progress = 0.0;
                self.timer = 0;
                // The block shows its last instruction while a signal is held.
                (!signal).then_some(0)
            }
        }
    }

    /// `tick` of the block entity: `speed` is the block's own (0 if overstressed).
    pub fn tick(&mut self, speed: f32, theoretical: f32, signal: bool) -> State {
        if self.is_idle() || self.duration < 0 {
            return None;
        }
        if self.timer < self.duration {
            self.timer += 1;
            if let Some(instruction) = self.instruction(self.current) {
                // Counted at the output's speed (see the header).
                self.progress += instruction.tick_progress((speed * instruction.modifier.value() as f32).abs());
            }
            return None;
        }
        self.run(self.current + 1, theoretical, signal)
    }

    /// `onSpeedChanged`: the rest of the instruction is timed again at the new speed, and
    /// a block that has stopped turning ends the sequence.
    pub fn speed_changed(&mut self, previous: f32, speed: f32, theoretical: f32, signal: bool) -> State {
        if self.is_idle() || previous.abs() == speed.abs() {
            return None;
        }
        let instruction = self.instruction(self.current)?;
        if speed == 0.0 {
            return self.run(-1, theoretical, signal);
        }
        self.duration = instruction.duration(self.progress, theoretical, self.overshoot);
        self.timer = 0;
        None
    }

    /// `risingFlank`: a new pulse moves an `Await` on.
    fn rising_flank(&mut self, theoretical: f32, signal: bool) -> State {
        let instruction = self.instruction(self.current)?;
        if self.powered_previously {
            return None;
        }
        self.powered_previously = true;
        match instruction.op {
            Op::Await => self.run(self.current + 1, theoretical, signal),
            _ => None,
        }
    }

    /// `onRedstoneUpdate`: `powered` is the signal now, `running` whether the block's `state`
    /// shows an instruction (`STATE != 0`), `speed` its own speed.
    pub fn redstone(&mut self, powered: bool, running: bool, speed: f32, theoretical: f32) -> State {
        let mut state = None;
        if !self.powered_previously && powered {
            state = self.rising_flank(theoretical, powered);
        }
        self.powered_previously = powered;
        if !self.is_idle() || powered == running {
            return state;
        }
        if !powered {
            return Some(0);
        }
        if speed == 0.0 {
            return state;
        }
        self.run(0, theoretical, powered).or(state)
    }
}

// --- the stand-in for the screen ---------------------------------------------------------------

/// One thing a click can change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Field {
    #[default]
    Type,
    Value,
    Modifier,
}

/// Where the edits are: the instruction (0 to 4) and which of its fields.
///
/// An empty hand moves it to the next field (a sneaking one to the previous), skipping fields
/// the instruction doesn't have and slots after the first `End`; the Wrench steps the field
/// forward on a face pointing the positive way and back on one pointing the negative way.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Cursor {
    pub slot: usize,
    pub field: Field,
}

impl Cursor {
    /// Saved as one number: `slot * 3 + field`.
    pub fn to_int(self) -> i32 {
        (self.slot * 3 + self.field as usize) as i32
    }

    pub fn from_int(value: i32) -> Cursor {
        let value = value.clamp(0, (MAX_INSTRUCTIONS * 3 - 1) as i32) as usize;
        Cursor {
            slot: value / 3,
            field: [Field::Type, Field::Value, Field::Modifier][value % 3],
        }
    }
}

/// The cursor positions of a program in order: each slot up to and including the first `End`.
pub fn positions(program: &[Instruction]) -> Vec<Cursor> {
    let program = padded(program);
    let mut out = Vec::new();
    for (slot, instruction) in program.iter().enumerate() {
        out.push(Cursor { slot, field: Field::Type });
        if instruction.op.has_value() {
            out.push(Cursor { slot, field: Field::Value });
        }
        if instruction.op.has_speed() {
            out.push(Cursor { slot, field: Field::Modifier });
        }
        if instruction.op == Op::End {
            break;
        }
    }
    out
}

/// Moves the cursor one field on, round the end; one that isn't on a field any more (the
/// instruction changed under it) goes to its slot's type, or the first.
pub fn move_cursor(program: &[Instruction], cursor: Cursor, forward: bool) -> Cursor {
    let all = positions(program);
    let Some(index) = all.iter().position(|c| *c == cursor) else {
        return all
            .iter()
            .copied()
            .find(|c| c.slot == cursor.slot)
            .unwrap_or(all[0]);
    };
    let count = all.len();
    all[if forward { (index + 1) % count } else { (index + count - 1) % count }]
}

fn step_in(list: &[i32], current: i32, forward: bool) -> i32 {
    if forward {
        list.iter().copied().find(|v| *v > current).unwrap_or(list[0])
    } else {
        list.iter().rev().copied().find(|v| *v < current).unwrap_or(list[list.len() - 1])
    }
}

/// Steps the field under the cursor, in place on the five-slot program.
pub fn step_field(program: &mut Vec<Instruction>, cursor: Cursor, forward: bool) {
    *program = padded(program);
    let instruction = &mut program[cursor.slot];
    match cursor.field {
        Field::Type => {
            let index = Op::ALL.iter().position(|op| *op == instruction.op).unwrap_or(0);
            let count = Op::ALL.len();
            let next = Op::ALL[if forward { (index + 1) % count } else { (index + count - 1) % count }];
            instruction.op = next;
            // `SequencedGearshiftScreen`: a new type starts from its default value.
            instruction.value = if next.has_value() { next.default_value() } else { 1 };
        }
        Field::Value => {
            if instruction.op.has_value() {
                instruction.value = step_in(instruction.op.steps(), instruction.value, forward);
            }
        }
        Field::Modifier => {
            if instruction.op.has_speed() {
                let index = Modifier::ALL
                    .iter()
                    .position(|m| *m == instruction.modifier)
                    .unwrap_or(1);
                let count = Modifier::ALL.len();
                instruction.modifier =
                    Modifier::ALL[if forward { (index + 1) % count } else { (index + count - 1) % count }];
            }
        }
    }
}

/// The program on one line for the action bar, with the field under the cursor in brackets:
/// `1: Turn [90°] ->  2: End`.
pub fn describe(program: &[Instruction], cursor: Cursor) -> String {
    let program = padded(program);
    let mut parts = Vec::new();
    for (slot, instruction) in program.iter().enumerate() {
        let focus = |field: Field, text: String| {
            if cursor.slot == slot && cursor.field == field {
                format!("[{text}]")
            } else {
                text
            }
        };
        let mut line = format!("{}: {}", slot + 1, focus(Field::Type, instruction.op.label().to_owned()));
        if instruction.op.has_value() {
            line.push(' ');
            line.push_str(&focus(Field::Value, instruction.format_value()));
        }
        if instruction.op.has_speed() {
            line.push(' ');
            line.push_str(&focus(Field::Modifier, instruction.modifier.arrows().to_owned()));
        }
        parts.push(line);
        if instruction.op == Op::End {
            break;
        }
    }
    parts.join("  ")
}

// --- placement ---------------------------------------------------------------------------------

/// Which way the block turns: its rotation axis from the state (`getRotationAxis`).
pub fn rotation_axis(horizontal_axis: Axis, vertical: bool) -> Axis {
    if vertical { Axis::Y } else { horizontal_axis }
}

/// `SequencedGearshiftBlock.getStateForPlacement` / `withAxis`: the rotation axis a block is
/// placed on (`axis`, from the neighbouring shaft or the way the player looks) as the state's
/// `(axis, vertical)`: a vertical one keeps the horizontal axis the player faces across
/// (`getHorizontalDirection().getAxis()`).
pub fn placed_state(axis: Axis, facing: Dir) -> (Axis, bool) {
    if axis == Axis::Y {
        (facing.axis(), true)
    } else {
        (axis, false)
    }
}

/// `SequencedGearshiftBlock.hasAnalogOutputSignal`: the comparator reads the state, 0 to 5.
pub fn comparator(state: u8) -> i32 {
    i32::from(state.min(5))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sequence(program: Vec<Instruction>) -> Sequence {
        Sequence {
            instructions: program,
            overshoot: OVERSHOOT,
            ..Sequence::default()
        }
    }

    #[test]
    fn the_default_program_is_a_quarter_turn_then_the_end() {
        let program = default_program();
        assert_eq!(program[0], Instruction::with(Op::TurnAngle, 90));
        assert_eq!(program[1].op, Op::End);
        assert_eq!(Sequence::default().modifier(), 0);
    }

    #[test]
    fn a_turn_lasts_as_long_as_the_angle_needs_at_the_modified_speed() {
        // Create: ceil(90 / 4.8) + 2 = 21 at 16 RPM.
        let turn = Instruction::with(Op::TurnAngle, 90);
        assert_eq!(turn.duration(0.0, 16.0, OVERSHOOT), 21);
        assert_eq!(turn.duration(0.0, 16.0, TRIMMED_OVERSHOOT), 19);
        // Double speed halves it, back turns at the same rate, a half done turn is half as long.
        let fast = Instruction { modifier: Modifier::ForwardFast, ..turn };
        assert_eq!(fast.duration(0.0, 16.0, 0), 10);
        let back = Instruction { modifier: Modifier::BackFast, ..turn };
        assert_eq!(back.duration(0.0, -16.0, 0), 10);
        assert_eq!(turn.duration(45.0, 16.0, 0), 10);
        // A distance: 5 blocks at 512 / 512 = 1 block a tick at 512 RPM would be 5 ticks;
        // at 64 RPM a tick is 0.125 blocks, so 40.
        let distance = Instruction::with(Op::TurnDistance, 5);
        assert_eq!(distance.duration(0.0, 64.0, 0), 40);
        // A delay is its value, a wait has no end of its own.
        assert_eq!(Instruction::with(Op::Delay, 60).duration(0.0, 16.0, 2), 60);
        assert_eq!(Instruction::with(Op::Delay, 60).duration(20.0, 16.0, 2), 40);
        assert_eq!(Instruction::new(Op::Await).duration(0.0, 16.0, 2), -1);
        assert_eq!(Instruction::new(Op::End).duration(0.0, 16.0, 2), 0);
        // No speed, no end.
        assert_eq!(turn.duration(0.0, 0.0, 0), i32::MAX);
    }

    #[test]
    fn only_turning_instructions_turn_the_output() {
        let mut seq = sequence(vec![
            Instruction { modifier: Modifier::Back, ..Instruction::with(Op::TurnAngle, 90) },
            Instruction::with(Op::Delay, 20),
            Instruction::new(Op::Await),
            Instruction { modifier: Modifier::ForwardFast, ..Instruction::with(Op::TurnDistance, 2) },
            Instruction::new(Op::End),
        ]);
        assert_eq!(seq.modifier(), 0);
        assert_eq!(seq.run(0, 16.0, true), Some(1));
        assert_eq!(seq.modifier(), -1);
        seq.run(1, 16.0, true);
        assert_eq!(seq.modifier(), 0);
        seq.run(2, 16.0, true);
        assert_eq!(seq.modifier(), 0);
        seq.run(3, 16.0, true);
        assert_eq!(seq.modifier(), 2);
    }

    #[test]
    fn a_pulse_starts_it_and_it_runs_to_the_end() {
        let mut seq = sequence(default_program());
        // Powered, idle, state 0, turning: starts.
        assert_eq!(seq.redstone(true, false, 16.0, 16.0), Some(1));
        assert_eq!(seq.current, 0);
        assert_eq!(seq.duration, 21);
        // It counts 21 ticks, then runs the next: the End. The signal is still on: the state stays.
        for _ in 0..21 {
            assert_eq!(seq.tick(16.0, 16.0, true), None);
        }
        assert!((seq.progress - 21.0 * 4.8).abs() < 1e-3);
        assert_eq!(seq.tick(16.0, 16.0, true), None);
        assert!(seq.is_idle());
        // While the signal is held it does not start again...
        assert_eq!(seq.redstone(true, true, 16.0, 16.0), None);
        assert!(seq.is_idle());
        // ...and a falling edge shows idle again.
        assert_eq!(seq.redstone(false, true, 16.0, 16.0), Some(0));
        // Then a new pulse starts over.
        assert_eq!(seq.redstone(true, false, 16.0, 16.0), Some(1));
    }

    #[test]
    fn it_does_not_start_without_a_turning_input() {
        let mut seq = sequence(default_program());
        assert_eq!(seq.redstone(true, false, 0.0, 0.0), None);
        assert!(seq.is_idle());
    }

    #[test]
    fn the_end_with_the_signal_off_resets_the_state() {
        let mut seq = sequence(vec![Instruction::with(Op::Delay, 3)]);
        seq.run(0, 16.0, true);
        for _ in 0..3 {
            seq.tick(16.0, 16.0, false);
        }
        // Out of instructions: ends, and with no signal shows idle.
        assert_eq!(seq.tick(16.0, 16.0, false), Some(0));
        assert!(seq.is_idle());
        assert_eq!(seq.duration, -1);
    }

    #[test]
    fn an_await_waits_for_a_new_rising_edge() {
        let mut seq = sequence(vec![
            Instruction::new(Op::Await),
            Instruction::with(Op::Delay, 10),
            Instruction::new(Op::End),
        ]);
        seq.run(0, 16.0, true);
        seq.powered_previously = true;
        // Nothing happens however long it waits, and the held signal is not a new pulse.
        for _ in 0..100 {
            assert_eq!(seq.tick(16.0, 16.0, true), None);
        }
        assert_eq!(seq.redstone(true, true, 16.0, 16.0), None);
        assert_eq!(seq.current, 0);
        // The signal drops and comes back: a rising edge moves it on.
        assert_eq!(seq.redstone(false, true, 16.0, 16.0), None);
        assert_eq!(seq.current, 0);
        assert_eq!(seq.redstone(true, true, 16.0, 16.0), Some(2));
        assert_eq!(seq.current, 1);
    }

    #[test]
    fn a_change_of_speed_times_the_rest_again_and_stopping_ends_it() {
        let mut seq = sequence(default_program());
        seq.overshoot = 0;
        seq.run(0, 16.0, true);
        for _ in 0..10 {
            seq.tick(16.0, 16.0, true);
        }
        // 48 degrees done, 42 to go at 32 RPM (9.6 a tick): 5 ticks.
        assert_eq!(seq.speed_changed(16.0, 32.0, 32.0, true), None);
        assert_eq!(seq.duration, 5);
        assert_eq!(seq.timer, 0);
        // The same speed the other way round is no change.
        assert_eq!(seq.speed_changed(32.0, -32.0, -32.0, true), None);
        assert_eq!(seq.duration, 5);
        // Standing still ends it (the signal being on, the state is left).
        assert_eq!(seq.speed_changed(32.0, 0.0, 0.0, true), None);
        assert!(seq.is_idle());
        // Without a signal it shows idle.
        seq.run(0, 16.0, false);
        assert_eq!(seq.speed_changed(16.0, 0.0, 0.0, false), Some(0));
    }

    #[test]
    fn the_progress_is_counted_at_the_output_speed() {
        let mut seq = sequence(vec![Instruction {
            modifier: Modifier::ForwardFast,
            ..Instruction::with(Op::TurnAngle, 180)
        }]);
        seq.run(0, 16.0, true);
        seq.tick(16.0, 16.0, true);
        assert!((seq.progress - 9.6).abs() < 1e-3);
    }

    #[test]
    fn the_cursor_visits_each_field_up_to_the_first_end() {
        let program = default_program();
        let all = positions(&program);
        assert_eq!(
            all,
            vec![
                Cursor { slot: 0, field: Field::Type },
                Cursor { slot: 0, field: Field::Value },
                Cursor { slot: 0, field: Field::Modifier },
                Cursor { slot: 1, field: Field::Type },
            ]
        );
        let mut cursor = Cursor::default();
        let mut seen = Vec::new();
        for _ in 0..5 {
            cursor = move_cursor(&program, cursor, true);
            seen.push(cursor);
        }
        assert_eq!(seen[2], Cursor { slot: 1, field: Field::Type });
        // Round to the start, and backwards from it round to the end.
        assert_eq!(seen[3], Cursor::default());
        assert_eq!(move_cursor(&program, Cursor::default(), false), Cursor { slot: 1, field: Field::Type });
    }

    #[test]
    fn a_cursor_off_its_field_goes_back_to_the_slot() {
        let mut program = default_program();
        let cursor = Cursor { slot: 0, field: Field::Modifier };
        program[0].op = Op::Delay;
        assert_eq!(
            move_cursor(&program, cursor, true),
            Cursor { slot: 0, field: Field::Type }
        );
        assert_eq!(Cursor::from_int(cursor.to_int()), cursor);
        assert_eq!(Cursor::from_int(99).slot, 4);
    }

    #[test]
    fn a_click_steps_the_field_under_the_cursor() {
        let mut program = default_program();
        let value = Cursor { slot: 0, field: Field::Value };
        step_field(&mut program, value, true);
        assert_eq!(program[0].value, 135);
        step_field(&mut program, value, false);
        step_field(&mut program, value, false);
        assert_eq!(program[0].value, 45);
        // Round the ends.
        step_field(&mut program, value, false);
        assert_eq!(program[0].value, 360);
        step_field(&mut program, value, true);
        assert_eq!(program[0].value, 45);
        // A value that isn't a step (set by `/data`) goes to the next one up.
        program[0].value = 100;
        step_field(&mut program, value, true);
        assert_eq!(program[0].value, 135);

        let modifier = Cursor { slot: 0, field: Field::Modifier };
        step_field(&mut program, modifier, true);
        assert_eq!(program[0].modifier, Modifier::Back);
        step_field(&mut program, modifier, false);
        step_field(&mut program, modifier, false);
        assert_eq!(program[0].modifier, Modifier::ForwardFast);
        // The program grows to five slots.
        assert_eq!(program.len(), MAX_INSTRUCTIONS);
    }

    #[test]
    fn a_new_type_starts_from_its_default() {
        let mut program = default_program();
        let kind = Cursor::default();
        step_field(&mut program, kind, true);
        assert_eq!(program[0].op, Op::TurnDistance);
        assert_eq!(program[0].value, 5);
        step_field(&mut program, kind, true);
        assert_eq!((program[0].op, program[0].value), (Op::Delay, 10));
        step_field(&mut program, kind, true);
        assert_eq!(program[0].op, Op::Await);
        step_field(&mut program, kind, false);
        step_field(&mut program, kind, false);
        step_field(&mut program, kind, false);
        assert_eq!((program[0].op, program[0].value), (Op::TurnAngle, 90));
        // A slot after the End can be given an instruction, and the cursor then reaches it.
        let second = Cursor { slot: 1, field: Field::Type };
        step_field(&mut program, second, false); // End -> Await
        assert_eq!(program[1].op, Op::Await);
        assert_eq!(positions(&program).last().unwrap().slot, 2);
    }

    #[test]
    fn the_action_bar_line_marks_the_field() {
        let program = default_program();
        let line = describe(&program, Cursor { slot: 0, field: Field::Value });
        assert_eq!(line, "1: Turn by angle [90\u{b0}] ->  2: End");
        let line = describe(&program, Cursor { slot: 1, field: Field::Type });
        assert_eq!(line, "1: Turn by angle 90\u{b0} ->  2: [End]");
        assert_eq!(Instruction::with(Op::Delay, 60).format_value(), "3s");
        assert_eq!(Instruction::with(Op::Delay, 10).format_value(), "10t");
        assert_eq!(Instruction::with(Op::TurnDistance, 5).format_value(), "5m");
    }

    #[test]
    fn modifiers_and_names_round_trip() {
        for op in Op::ALL {
            assert_eq!(Op::from_name(op.name()), Some(op));
        }
        for modifier in Modifier::ALL {
            assert_eq!(Modifier::from_name(modifier.name()), Some(modifier));
            assert_eq!(Modifier::by_value(modifier.value()), modifier);
        }
        assert_eq!(Modifier::by_value(7), Modifier::Forward);
        assert_eq!(
            Modifier::ALL.map(Modifier::value),
            [2, 1, -1, -2],
        );
    }

    // --- the gearshift in a network ---

    use crate::{Kind, Node, Pos, solve};
    use std::collections::BTreeMap;

    fn motor(facing: Dir, rpm: f32) -> Node {
        Node {
            kind: Kind::Motor(facing),
            generated: rpm,
            capacity: 1000.0,
            impact: 0.0,
        }
    }

    /// A motor in the west, a shaft, the gearshift at x = 2 along x, a shaft either side of it.
    fn line(modifier: i32) -> BTreeMap<Pos, Node> {
        BTreeMap::from([
            ([0, 0, 0], motor(Dir::East, 16.0)),
            ([1, 0, 0], Node::passive(Kind::Shaft(Axis::X))),
            ([2, 0, 0], Node::passive(Kind::sequenced(Axis::X, modifier))),
            ([3, 0, 0], Node::passive(Kind::Shaft(Axis::X))),
            ([4, 0, 0], Node::passive(Kind::Shaft(Axis::X))),
        ])
    }

    #[test]
    fn the_output_side_turns_by_the_modifier() {
        for (modifier, out) in [(1, 16.0), (2, 32.0), (-1, -16.0), (-2, -32.0), (0, 0.0)] {
            let solution = solve(&line(modifier));
            // The block's speed is the input end's.
            assert_eq!(solution.speeds[&[2, 0, 0]], 16.0, "{modifier}");
            assert_eq!(solution.speeds[&[1, 0, 0]], 16.0);
            assert_eq!(solution.speeds[&[3, 0, 0]], out, "{modifier}");
            assert_eq!(solution.speeds[&[4, 0, 0]], out, "{modifier}");
            assert!(solution.breaks.is_empty());
        }
    }

    #[test]
    fn the_input_is_the_side_the_power_comes_in_by() {
        // Power from the east end instead: the west end is now the output.
        let nodes = BTreeMap::from([
            ([-2, 0, 0], Node::passive(Kind::Shaft(Axis::X))),
            ([-1, 0, 0], Node::passive(Kind::Shaft(Axis::X))),
            ([0, 0, 0], Node::passive(Kind::sequenced(Axis::X, 2))),
            ([1, 0, 0], Node::passive(Kind::Shaft(Axis::X))),
            ([2, 0, 0], motor(Dir::West, 16.0)),
        ]);
        let solution = solve(&nodes);
        assert_eq!(solution.speeds[&[0, 0, 0]], 16.0);
        assert_eq!(solution.speeds[&[-1, 0, 0]], 32.0);
        assert_eq!(solution.speeds[&[-2, 0, 0]], 32.0);
    }

    #[test]
    fn a_standing_output_has_no_speed_to_ask_for() {
        // A motor on the output side of an idle gearshift is a network of its own.
        let mut nodes = line(0);
        nodes.insert([4, 0, 0], motor(Dir::West, 64.0));
        let solution = solve(&nodes);
        assert!(solution.breaks.is_empty());
        assert_eq!(solution.speeds[&[3, 0, 0]], 64.0);
        assert_eq!(solution.speeds[&[1, 0, 0]], 16.0);
    }

    #[test]
    fn both_sides_can_be_driven_when_they_agree() {
        // 16 in, 32 asked on the far side of a x2 gearshift: both ends are satisfied.
        let mut nodes = line(2);
        nodes.insert([4, 0, 0], motor(Dir::West, 32.0));
        let solution = solve(&nodes);
        assert!(solution.breaks.is_empty());
        assert_eq!(solution.speeds[&[3, 0, 0]], 32.0);
        // A faster one on the far side drives the whole network, the input side included.
        nodes.insert([4, 0, 0], motor(Dir::West, 64.0));
        let solution = solve(&nodes);
        assert_eq!(solution.speeds[&[3, 0, 0]], 64.0);
    }

    #[test]
    fn placement_keeps_the_axis_the_player_faces_across_when_vertical() {
        assert_eq!(placed_state(Axis::X, Dir::North), (Axis::X, false));
        assert_eq!(placed_state(Axis::Y, Dir::North), (Axis::Z, true));
        assert_eq!(placed_state(Axis::Y, Dir::East), (Axis::X, true));
        assert_eq!(rotation_axis(Axis::Z, true), Axis::Y);
        assert_eq!(rotation_axis(Axis::Z, false), Axis::Z);
        assert_eq!(comparator(3), 3);
        assert_eq!(comparator(9), 5);
    }
}
