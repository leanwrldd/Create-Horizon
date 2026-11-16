//! The pieces a pipe draws on each of its sides: the arm that joins the next pipe, the rim of an open
//! end, the drain flange where it meets a tank, and the casing of a junction. No game types.
//!
//! Create draws them from a baked model wrapper (`PipeAttachmentModel`) that asks the block entity's
//! transport behaviour for an `AttachmentTypes` per side. Here that choice is a pure rule: the glue
//! reads what is beside each side, [`attachment`] chooses, and [`encode`] puts the six choices in the
//! block entity's NBT.
//!
//! Ported from Create `content/fluids/FluidTransportBehaviour` (`getRenderedRimAttachment`,
//! `AttachmentTypes`), `FluidPipeBlockEntity` and `StraightPipeBlockEntity` (their overrides for the
//! pipes, the glass pipe, the valve and the smart pipe), `PumpBlockEntity` (the pump's),
//! `FluidPipeBlock` (`shouldDrawRim`, `canConnectTo`, `shouldDrawCasing`) and `AllPartialModels`
//! (`PIPE_ATTACHMENTS`, `FLUID_PIPE_CASING`).
//!
//! Known differences from Create: a hose pulley is not in the port, so its exclusion from the drain
//! (`!AllBlocks.HOSE_PULLEY.has(facingState)`) is not made; the glue refreshes the choices every few
//! ticks, not on each neighbour's update.

use super::fluid_pipe::{Sides, straight_axis};
use crate::{Axis, Dir};

/// `FluidTransportBehaviour.AttachmentTypes.ComponentPartials`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Partial {
    Connection,
    RimConnector,
    Rim,
    Drain,
}

impl Partial {
    /// The folder of its models: `AllPartialModels.PIPE_ATTACHMENTS` is
    /// `block("fluid_pipe/" + Lang.asId(type.name()) + "/" + <direction>)`.
    pub const fn folder(self) -> &'static str {
        match self {
            Partial::Connection => "connection",
            Partial::RimConnector => "rim_connector",
            Partial::Rim => "rim",
            Partial::Drain => "drain",
        }
    }

    pub const ALL: [Partial; 4] =
        [Partial::Connection, Partial::RimConnector, Partial::Rim, Partial::Drain];
}

/// `FluidTransportBehaviour.AttachmentTypes`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attachment {
    None,
    Connection,
    DetailedConnection,
    Rim,
    PartialRim,
    Drain,
    PartialDrain,
}

impl Attachment {
    /// The enum's constructor arguments.
    pub const fn partials(self) -> &'static [Partial] {
        match self {
            Attachment::None => &[],
            Attachment::Connection => &[Partial::Connection],
            Attachment::DetailedConnection => &[Partial::RimConnector],
            Attachment::Rim => &[Partial::RimConnector, Partial::Rim],
            Attachment::PartialRim => &[Partial::Rim],
            Attachment::Drain => &[Partial::RimConnector, Partial::Drain],
            Attachment::PartialDrain => &[Partial::Drain],
        }
    }

    /// `AttachmentTypes.withoutConnector`.
    pub const fn without_connector(self) -> Attachment {
        match self {
            Attachment::Rim => Attachment::PartialRim,
            Attachment::Drain => Attachment::PartialDrain,
            other => other,
        }
    }

    /// The enum's ordinal, which is what the block entity saves.
    pub const fn id(self) -> u8 {
        match self {
            Attachment::None => 0,
            Attachment::Connection => 1,
            Attachment::DetailedConnection => 2,
            Attachment::Rim => 3,
            Attachment::PartialRim => 4,
            Attachment::Drain => 5,
            Attachment::PartialDrain => 6,
        }
    }

    pub const fn from_id(id: u8) -> Option<Attachment> {
        Some(match id {
            0 => Attachment::None,
            1 => Attachment::Connection,
            2 => Attachment::DetailedConnection,
            3 => Attachment::Rim,
            4 => Attachment::PartialRim,
            5 => Attachment::Drain,
            6 => Attachment::PartialDrain,
            _ => return None,
        })
    }
}

/// The block whose attachments are chosen, with what its transport behaviour says about its sides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Block {
    /// A `FluidPipeBlock` and the sides it has open (`FluidPipeBlockEntity`).
    Pipe(Sides),
    /// An `EncasedPipeBlock` and its open sides (`FluidPipeBlockEntity` too).
    Encased(Sides),
    /// A `GlassFluidPipeBlock` along an axis (`StraightPipeBlockEntity`).
    Glass(Axis),
    /// A `FluidValveBlock` whose water runs along an axis (`StraightPipeBlockEntity`).
    Valve(Axis),
    /// A `SmartFluidPipeBlock` along an axis (`StraightPipeBlockEntity`).
    Smart(Axis),
    /// A `PumpBlock` and its `FACING` (`PumpBlockEntity`).
    Pump(Dir),
}

impl Block {
    /// `FluidTransportBehaviour.canHaveFlowToward` of the block's own behaviour.
    pub fn can_flow(&self, side: Dir) -> bool {
        match self {
            Block::Pipe(sides) | Block::Encased(sides) => sides[side as usize],
            Block::Glass(axis) | Block::Valve(axis) | Block::Smart(axis) => *axis == side.axis(),
            Block::Pump(facing) => facing.axis() == side.axis(),
        }
    }
}

/// What stands beside one side of the block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Beside {
    /// Air, stone, a tank: nothing with a pipe's transport behaviour.
    Other,
    /// A `FluidPipeBlock`, whether it carries a bracket, and its straight axis
    /// (`FluidPropagator.getStraightPipeAxis`).
    Pipe { bracket: bool, straight: Option<Axis> },
    /// An `EncasedPipeBlock`, and whether its side facing the block is open.
    Encased { open_toward_us: bool },
    Glass(Axis),
    Valve(Axis),
    Smart(Axis),
    /// A `PumpBlock` and its `FACING`.
    Pump(Dir),
}

impl Beside {
    /// `IAxisPipe.getAxisOf`: glass pipes, valves and smart pipes.
    fn axis_pipe(self) -> Option<Axis> {
        match self {
            Beside::Glass(axis) | Beside::Valve(axis) | Beside::Smart(axis) => Some(axis),
            _ => None,
        }
    }

    fn is_pipe(self) -> bool {
        matches!(self, Beside::Pipe { .. })
    }
}

/// `FluidTransportBehaviour.getRenderedRimAttachment`, the base of every override. `side` is the
/// direction from the block to what is beside it; `capability` is
/// `FluidPropagator.hasFluidCapability(world, pos beside, side.getOpposite())` (a tank, the spout).
fn base(own: &Block, side: Dir, beside: Beside, capability: bool) -> Attachment {
    if !own.can_flow(side) {
        return Attachment::None;
    }
    // A pump facing us: its own end joins the pipe.
    if beside == Beside::Pump(side.opposite()) {
        return Attachment::None;
    }
    if matches!(beside, Beside::Encased { open_toward_us: true }) {
        return Attachment::Rim;
    }
    if capability {
        return Attachment::Drain;
    }
    Attachment::Rim
}

/// `FluidPipeBlock.shouldDrawRim`: whether the rim is shown on a side with `beside` past it. A pipe
/// beside draws none unless `canConnectTo` refuses it: a bracketed pipe joins only along its straight
/// axis (`FluidPipeBlock.canConnectTo`).
fn should_draw_rim(side: Dir, beside: Beside) -> bool {
    match beside {
        Beside::Pipe { bracket, straight } => bracket && straight != Some(side.axis()),
        _ => true,
    }
}

/// `FluidPipeBlockEntity.StandardPipeFluidTransportBehaviour.getRenderedRimAttachment`.
fn pipe(sides: &Sides, encased: bool, side: Dir, beside: Beside, base: Attachment) -> Attachment {
    if encased && base != Attachment::Drain {
        return Attachment::None;
    }
    if base == Attachment::Rim {
        // A valve, a smart pipe or a pump the pipe flows into: its connector shows.
        let other_flows_toward_us = match beside {
            Beside::Valve(axis) | Beside::Smart(axis) => axis == side.axis(),
            Beside::Pump(facing) => facing.axis() == side.axis(),
            _ => false,
        };
        if !matches!(beside, Beside::Pipe { .. } | Beside::Encased { .. } | Beside::Glass(_))
            && other_flows_toward_us
        {
            return Attachment::DetailedConnection;
        }
        if !should_draw_rim(side, beside) {
            return if straight_axis(sides) == Some(side.axis()) {
                Attachment::Connection
            } else {
                Attachment::DetailedConnection
            };
        }
    }
    if base == Attachment::None && sides[side as usize] {
        return Attachment::DetailedConnection;
    }
    base
}

/// `StraightPipeBlockEntity.StraightPipeFluidTransportBehaviour.getRenderedRimAttachment`.
fn straight(own: &Block, axis: Axis, side: Dir, beside: Beside, base: Attachment) -> Attachment {
    let is_valve = matches!(own, Block::Valve(_));
    let is_glass = matches!(own, Block::Glass(_));
    if base == Attachment::Rim && is_valve {
        return Attachment::None;
    }
    if base == Attachment::Rim && !is_glass && matches!(beside, Beside::Glass(_)) {
        return Attachment::PartialRim;
    }
    if base == Attachment::Rim && beside.is_pipe() {
        return Attachment::None;
    }
    if beside.axis_pipe() == Some(axis) {
        return Attachment::None;
    }
    if let Beside::Valve(valve_axis) = beside
        && valve_axis == side.axis()
    {
        return Attachment::None;
    }
    base.without_connector()
}

/// What the block at `own` draws on `side`, given `beside` and `capability` (see [`base`]).
pub fn attachment(own: &Block, side: Dir, beside: Beside, capability: bool) -> Attachment {
    let base = base(own, side, beside, capability);
    match own {
        Block::Pipe(sides) => pipe(sides, false, side, beside, base),
        Block::Encased(sides) => pipe(sides, true, side, beside, base),
        Block::Glass(axis) | Block::Valve(axis) | Block::Smart(axis) => {
            straight(own, *axis, side, beside, base)
        }
        // `PumpBlockEntity`: a rim is the pump's own end, so only a drain shows.
        Block::Pump(_) => {
            if base == Attachment::Rim {
                Attachment::None
            } else {
                base
            }
        }
    }
}

/// `FluidPipeBlock.shouldDrawCasing`: a pipe with more than two connections off some axis, the
/// junction of a tee or a cross, is capped with the casing. Only a `FluidPipeBlock` has one.
pub fn draws_casing(sides: &Sides) -> bool {
    Axis::ALL.into_iter().any(|axis| {
        Dir::ALL.into_iter().filter(|d| d.axis() != axis && sides[*d as usize]).count() > 2
    })
}

/// The six attachments, in `Iterate.directions` order (down, up, north, south, west, east), and
/// whether the casing is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rims {
    pub attachments: [Attachment; 6],
    pub casing: bool,
}

impl Rims {
    pub const NONE: Rims = Rims { attachments: [Attachment::None; 6], casing: false };
}

/// What the block entity saves: one byte per side, then the casing as 0 or 1.
pub fn encode(rims: &Rims) -> Vec<u8> {
    let mut bytes: Vec<u8> = rims.attachments.iter().map(|a| a.id()).collect();
    bytes.push(u8::from(rims.casing));
    bytes
}

pub fn decode(bytes: &[u8]) -> Option<Rims> {
    if bytes.len() != 7 {
        return None;
    }
    let mut attachments = [Attachment::None; 6];
    for (slot, id) in attachments.iter_mut().zip(bytes) {
        *slot = Attachment::from_id(*id)?;
    }
    Some(Rims { attachments, casing: bytes[6] != 0 })
}

/// `AllPartialModels.FLUID_PIPE_CASING`.
pub const CASING_MODEL: &str = "create:block/fluid_pipe/casing";

/// `AllPartialModels.PIPE_ATTACHMENTS` for `partial` on `side`.
pub fn partial_model(partial: Partial, side: Dir) -> String {
    format!("create:block/fluid_pipe/{}/{}", partial.folder(), side.name())
}

/// Every model the pipes can draw, for the `standalone_models` hook.
pub fn all_models() -> Vec<String> {
    let mut models = vec![CASING_MODEL.to_owned()];
    for partial in Partial::ALL {
        for side in Dir::ALL {
            models.push(partial_model(partial, side));
        }
    }
    models
}

/// The models to draw for `rims`, in `PipeAttachmentModel.addQuads` order: each side's partials, then
/// the casing.
pub fn models(rims: &Rims) -> Vec<String> {
    let mut models = Vec::new();
    for side in Dir::ALL {
        for partial in rims.attachments[side as usize].partials() {
            models.push(partial_model(*partial, side));
        }
    }
    if rims.casing {
        models.push(CASING_MODEL.to_owned());
    }
    models
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::fluid_pipe::axis_sides;

    fn sides(open: &[Dir]) -> Sides {
        Dir::ALL.map(|d| open.contains(&d))
    }

    fn pipe_along(axis: Axis) -> Block {
        Block::Pipe(axis_sides(axis))
    }

    const PIPE: Beside = Beside::Pipe { bracket: false, straight: None };

    #[test]
    fn a_side_that_is_closed_draws_nothing() {
        let own = pipe_along(Axis::X);
        assert_eq!(attachment(&own, Dir::Up, Beside::Other, false), Attachment::None);
        assert_eq!(attachment(&own, Dir::Up, PIPE, true), Attachment::None);
    }

    #[test]
    fn an_open_end_facing_nothing_has_a_rim() {
        let own = pipe_along(Axis::X);
        let rim = attachment(&own, Dir::East, Beside::Other, false);
        assert_eq!(rim, Attachment::Rim);
        assert_eq!(rim.partials(), [Partial::RimConnector, Partial::Rim]);
    }

    #[test]
    fn a_pipe_joined_to_a_pipe_along_its_run_gets_the_plain_connection() {
        let own = pipe_along(Axis::X);
        let straight = Beside::Pipe { bracket: false, straight: Some(Axis::X) };
        assert_eq!(attachment(&own, Dir::East, straight, false), Attachment::Connection);
        assert_eq!(attachment(&own, Dir::West, PIPE, false), Attachment::Connection);
    }

    #[test]
    fn a_corner_joined_to_a_pipe_gets_the_detailed_connection() {
        let own = Block::Pipe(sides(&[Dir::East, Dir::Down]));
        assert_eq!(attachment(&own, Dir::East, PIPE, false), Attachment::DetailedConnection);
        assert_eq!(attachment(&own, Dir::Down, PIPE, false), Attachment::DetailedConnection);
        // The sides it hasn't open stay empty.
        assert_eq!(attachment(&own, Dir::Up, PIPE, false), Attachment::None);
    }

    #[test]
    fn a_bracketed_pipe_beside_is_not_joined_across_its_run() {
        let own = pipe_along(Axis::X);
        // The neighbour runs along z: with its bracket on, the pipe doesn't join it.
        let crossing = Beside::Pipe { bracket: true, straight: Some(Axis::Z) };
        assert_eq!(attachment(&own, Dir::East, crossing, false), Attachment::Rim);
        // Along x, or without the bracket, it does.
        let along = Beside::Pipe { bracket: true, straight: Some(Axis::X) };
        assert_eq!(attachment(&own, Dir::East, along, false), Attachment::Connection);
        let bare = Beside::Pipe { bracket: false, straight: Some(Axis::Z) };
        assert_eq!(attachment(&own, Dir::East, bare, false), Attachment::Connection);
    }

    #[test]
    fn a_tank_beside_a_pipe_gets_the_drain() {
        let own = pipe_along(Axis::X);
        let drain = attachment(&own, Dir::East, Beside::Other, true);
        assert_eq!(drain, Attachment::Drain);
        assert_eq!(drain.partials(), [Partial::RimConnector, Partial::Drain]);
    }

    #[test]
    fn a_pipe_joins_a_valve_a_smart_pipe_and_a_pump_it_flows_into() {
        let own = pipe_along(Axis::X);
        for beside in [Beside::Valve(Axis::X), Beside::Smart(Axis::X), Beside::Pump(Dir::East)] {
            assert_eq!(attachment(&own, Dir::East, beside, false), Attachment::DetailedConnection);
        }
        // Across them, they have no end toward it: a rim.
        let own = pipe_along(Axis::Y);
        for beside in [Beside::Valve(Axis::X), Beside::Smart(Axis::Z), Beside::Pump(Dir::North)] {
            assert_eq!(attachment(&own, Dir::Up, beside, false), Attachment::Rim);
        }
    }

    #[test]
    fn a_pump_facing_the_pipe_leaves_its_open_side_a_detailed_connection() {
        let own = pipe_along(Axis::X);
        // The pump at the east side, its FACING pointing west at the pipe.
        assert_eq!(attachment(&own, Dir::East, Beside::Pump(Dir::West), false), Attachment::DetailedConnection);
        // A side that wasn't open stays empty.
        assert_eq!(attachment(&own, Dir::Up, Beside::Pump(Dir::Down), false), Attachment::None);
    }

    #[test]
    fn an_encased_pipe_beside_gives_a_rim_even_with_a_tank_behind_it() {
        let own = pipe_along(Axis::X);
        let open = Beside::Encased { open_toward_us: true };
        assert_eq!(attachment(&own, Dir::East, open, true), Attachment::Rim);
        // Closed toward the pipe it is a wall, and a tank in it is a drain.
        let closed = Beside::Encased { open_toward_us: false };
        assert_eq!(attachment(&own, Dir::East, closed, false), Attachment::Rim);
        assert_eq!(attachment(&own, Dir::East, closed, true), Attachment::Drain);
    }

    #[test]
    fn an_encased_pipe_draws_only_a_drain() {
        let own = Block::Encased(axis_sides(Axis::X));
        assert_eq!(attachment(&own, Dir::East, Beside::Other, false), Attachment::None);
        assert_eq!(attachment(&own, Dir::East, PIPE, false), Attachment::None);
        assert_eq!(attachment(&own, Dir::East, Beside::Other, true), Attachment::Drain);
        // Its own pump-facing and closed sides are none either.
        assert_eq!(attachment(&own, Dir::East, Beside::Pump(Dir::West), false), Attachment::None);
        assert_eq!(attachment(&own, Dir::Up, Beside::Other, true), Attachment::None);
    }

    #[test]
    fn a_glass_pipe_has_a_partial_rim_and_nothing_beside_a_pipe_or_its_own_kind() {
        let own = Block::Glass(Axis::X);
        assert_eq!(attachment(&own, Dir::East, Beside::Other, false), Attachment::PartialRim);
        assert_eq!(attachment(&own, Dir::East, PIPE, false), Attachment::None);
        assert_eq!(attachment(&own, Dir::East, Beside::Glass(Axis::X), false), Attachment::None);
        assert_eq!(attachment(&own, Dir::East, Beside::Smart(Axis::X), false), Attachment::None);
        assert_eq!(attachment(&own, Dir::East, Beside::Valve(Axis::X), false), Attachment::None);
        // A glass pipe along another axis is not in line: the rim shows.
        assert_eq!(attachment(&own, Dir::East, Beside::Glass(Axis::Z), false), Attachment::PartialRim);
        // Past the glass pipe's own sides, nothing.
        assert_eq!(attachment(&own, Dir::Up, Beside::Other, true), Attachment::None);
    }

    #[test]
    fn a_glass_pipe_beside_a_tank_has_a_partial_drain() {
        let own = Block::Glass(Axis::X);
        let drain = attachment(&own, Dir::East, Beside::Other, true);
        assert_eq!(drain, Attachment::PartialDrain);
        assert_eq!(drain.partials(), [Partial::Drain]);
    }

    #[test]
    fn a_valve_draws_no_rim_but_a_drain() {
        let own = Block::Valve(Axis::X);
        assert_eq!(attachment(&own, Dir::East, Beside::Other, false), Attachment::None);
        assert_eq!(attachment(&own, Dir::East, Beside::Glass(Axis::X), false), Attachment::None);
        assert_eq!(attachment(&own, Dir::East, Beside::Other, true), Attachment::PartialDrain);
    }

    #[test]
    fn a_smart_pipe_has_a_partial_rim_and_joins_a_glass_pipe_across_its_axis() {
        let own = Block::Smart(Axis::Z);
        assert_eq!(attachment(&own, Dir::North, Beside::Other, false), Attachment::PartialRim);
        // A glass pipe beside a smart pipe (not a glass pipe itself): `PARTIAL_RIM` before the axis rule,
        // so even in line it shows its rim.
        assert_eq!(attachment(&own, Dir::North, Beside::Glass(Axis::Z), false), Attachment::PartialRim);
        // A smart pipe in line with another: none.
        assert_eq!(attachment(&own, Dir::North, Beside::Smart(Axis::Z), false), Attachment::None);
        // A pipe: none.
        assert_eq!(attachment(&own, Dir::North, PIPE, false), Attachment::None);
        // An encased pipe open to it: a partial rim.
        let encased = Beside::Encased { open_toward_us: true };
        assert_eq!(attachment(&own, Dir::North, encased, false), Attachment::PartialRim);
    }

    #[test]
    fn a_pump_draws_only_a_drain() {
        let own = Block::Pump(Dir::East);
        assert_eq!(attachment(&own, Dir::East, Beside::Other, false), Attachment::None);
        assert_eq!(attachment(&own, Dir::West, PIPE, false), Attachment::None);
        assert_eq!(attachment(&own, Dir::East, Beside::Other, true), Attachment::Drain);
        // Its sides off the axis are closed.
        assert_eq!(attachment(&own, Dir::Up, Beside::Other, true), Attachment::None);
        // Another pump facing it joins end to end.
        assert_eq!(attachment(&own, Dir::East, Beside::Pump(Dir::West), true), Attachment::None);
    }

    #[test]
    fn only_a_tee_or_a_cross_has_the_casing() {
        assert!(!draws_casing(&sides(&[Dir::East, Dir::West])));
        assert!(!draws_casing(&sides(&[Dir::East, Dir::Down])));
        // A tee: three sides off the north axis (east, west, down).
        assert!(draws_casing(&sides(&[Dir::East, Dir::West, Dir::Down])));
        // Three off the up axis: east, west and north.
        assert!(draws_casing(&sides(&[Dir::East, Dir::West, Dir::North])));
        assert!(draws_casing(&sides(&[Dir::East, Dir::West, Dir::North, Dir::South, Dir::Up])));
        assert!(draws_casing(&[true; 6]));
        assert!(!draws_casing(&[false; 6]));
    }

    #[test]
    fn the_choice_survives_the_nbt() {
        let rims = Rims {
            attachments: [
                Attachment::None,
                Attachment::Connection,
                Attachment::DetailedConnection,
                Attachment::Rim,
                Attachment::PartialRim,
                Attachment::Drain,
            ],
            casing: true,
        };
        assert_eq!(decode(&encode(&rims)), Some(rims));
        let last = Rims { attachments: [Attachment::PartialDrain; 6], casing: false };
        assert_eq!(decode(&encode(&last)), Some(last));
        assert_eq!(decode(&[0; 6]), None);
        assert_eq!(decode(&[9, 0, 0, 0, 0, 0, 0]), None);
    }

    #[test]
    fn the_models_follow_create_s_partials() {
        let mut rims = Rims::NONE;
        rims.attachments[Dir::East as usize] = Attachment::Rim;
        rims.attachments[Dir::Down as usize] = Attachment::Connection;
        rims.casing = true;
        assert_eq!(
            models(&rims),
            [
                "create:block/fluid_pipe/connection/down",
                "create:block/fluid_pipe/rim_connector/east",
                "create:block/fluid_pipe/rim/east",
                "create:block/fluid_pipe/casing",
            ]
        );
        assert!(models(&Rims::NONE).is_empty());
    }

    #[test]
    fn every_partial_model_is_listed_once() {
        let all = all_models();
        assert_eq!(all.len(), 25);
        for partial in Partial::ALL {
            for side in Dir::ALL {
                assert!(all.contains(&partial_model(partial, side)));
            }
        }
        let mut sorted = all.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), all.len());
    }
}
