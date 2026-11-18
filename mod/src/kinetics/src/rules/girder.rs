//! Metal Girders and the Metal Girder Encased Shaft: which sides a girder shows a beam or a pole
//! towards, how the Wrench and the placement assist change them. Ported from Create `mc1.21.1/dev`
//! at 4148fd8 (MIT, the Creators of Create): `GirderBlock`, `GirderEncasedShaftBlock`,
//! `GirderWrenchBehavior`, `GirderPlacementHelper`. No game types: the glue describes each
//! neighbour as a [`Side`].
//!
//! The state is Create's: a girder has `x`, `z`, `top`, `bottom` and `axis`, the encased shaft
//! `top`, `bottom` and its `axis` (`HORIZONTAL_AXIS`, x or z). Waterlogging is left out.
//!
//! What a [`Side`] cannot say (`LevelReader` hands Create a whole block state; Horizon's shape
//! updates hand a mod only a state id), so these differ from Create:
//! - `isConnected` asks a neighbour for a full, solid face on the side towards the girder
//!   (`Block.isFaceFull`, `BlockState.isSolid`): every block that isn't air, a fluid or another
//!   girder counts as one, so a beam stays beside a flower or a torch that Create would let go.
//! - The nixie tube, the placard, the flap display, a bracket on the neighbour and a girdable track
//!   (`NixieTubeBlock`, `PlacardBlock`, `FlapDisplayBlock`, `isFacingBracket`, `GIRDABLE_TRACKS`)
//!   never attach a girder; the wall, lantern, chain and attached-face (button, lever) rules do.
//! - `getBlockSupportShape().isEmpty()` is "air or a fluid".

use crate::{Axis, Dir};

/// `AllBlocks.METAL_GIRDER`.
pub const GIRDER: &str = "create:metal_girder";
/// `AllBlocks.METAL_GIRDER_ENCASED_SHAFT`.
pub const ENCASED_SHAFT: &str = "create:metal_girder_encased_shaft";
/// `AllConfigs.server().equipment.placementAssistRange` (`CEquipment`): 12, with no Extendo Grip
/// bonus here.
pub const PLACEMENT_ASSIST_RANGE: i32 = 12;

/// A girder's or an encased shaft's block state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    /// The encased shaft rather than the girder.
    pub encased: bool,
    /// `GirderBlock.X` and `Z` (false on an encased shaft).
    pub x: bool,
    pub z: bool,
    /// `TOP` and `BOTTOM`.
    pub top: bool,
    pub bottom: bool,
    /// A girder's `AXIS` (`BlockStateProperties.AXIS`), an encased shaft's `HORIZONTAL_AXIS`.
    pub axis: Axis,
}

/// The five properties a girder's connections are kept in (`updateProperty` in `GirderBlock`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prop {
    X,
    Z,
    Top,
    Bottom,
}

impl Prop {
    /// `axis == X ? X : axis == Z ? Z : direction == UP ? TOP : BOTTOM`.
    pub fn of(d: Dir) -> Prop {
        match d {
            Dir::West | Dir::East => Prop::X,
            Dir::North | Dir::South => Prop::Z,
            Dir::Up => Prop::Top,
            Dir::Down => Prop::Bottom,
        }
    }
}

impl State {
    /// `defaultBlockState()`: a pole along y with nothing joined.
    pub const GIRDER: State =
        State { encased: false, x: false, z: false, top: false, bottom: false, axis: Axis::Y };

    pub fn get(&self, prop: Prop) -> bool {
        match prop {
            Prop::X => self.x,
            Prop::Z => self.z,
            Prop::Top => self.top,
            Prop::Bottom => self.bottom,
        }
    }

    pub fn with(mut self, prop: Prop, value: bool) -> State {
        match prop {
            Prop::X => self.x = value,
            Prop::Z => self.z = value,
            Prop::Top => self.top = value,
            Prop::Bottom => self.bottom = value,
        }
        self
    }

    /// `state.cycle(property)`.
    pub fn cycled(self, prop: Prop) -> State {
        let value = self.get(prop);
        self.with(prop, !value)
    }
}

/// The side of a block a face-attached block (`FACE`: a button, a lever, a grindstone) is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    Floor,
    Wall,
    Ceiling,
}

/// What the block next to a girder is, as far as the girder's rules care.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Side {
    /// `BlockState.isAir`.
    pub air: bool,
    /// A fluid's block (no support shape, not solid).
    pub fluid: bool,
    /// Another girder or an encased girder shaft.
    pub girder: Option<State>,
    /// `WallBlock.UP` is true (walls, and anything else with an `up` that is on).
    pub up: bool,
    /// `LanternBlock.HANGING`, for a lantern.
    pub hanging: Option<bool>,
    /// `ChainBlock` with `AXIS` y.
    pub chain_y: bool,
    /// `FaceAttachedHorizontalDirectionalBlock.FACE`.
    pub face: Option<Face>,
}

impl Side {
    /// `getBlockSupportShape().isEmpty()`, as far as it can be known.
    pub fn support_empty(&self) -> bool {
        self.air || self.fluid
    }
}

/// `GirderBlock.updateVerticalProperty`: the top or the bottom joins when the block above or below
/// can take a pole.
pub fn update_vertical(state: State, prop: Prop, side: &Side, d: Dir) -> State {
    let mut can_attach = false;
    if !state.encased && state.axis == Axis::Y {
        can_attach = true;
    } else if side.girder.is_some() {
        // `isGirder(sideState)` with or without `isXGirder == isZGirder`.
        can_attach = true;
    } else if side.up {
        can_attach = true;
    } else if side.hanging.is_some_and(|hanging| (d == Dir::Down) == hanging) {
        // `LanternBlock`: a lantern hangs below a girder or stands on top of one.
        can_attach = true;
    } else if side.chain_y {
        can_attach = true;
    } else if let Some(face) = side.face {
        can_attach = (face == Face::Ceiling && d == Dir::Down) || (face == Face::Floor && d == Dir::Up);
    }
    if can_attach { state.with(prop, true) } else { state }
}

/// `GirderBlock.updateState` for a girder (never an encased shaft): the property on side `d` is
/// switched on when the neighbour there asks for it.
pub fn update_state(state: State, d: Dir, side: &Side) -> State {
    let axis = d.axis();
    let prop = Prop::of(d);
    if axis == Axis::Y {
        return update_vertical(state, prop, side, d);
    }
    if state.axis == axis {
        return state.with(prop, true);
    }
    if let Some(other) = side.girder {
        // An encased girder shaft across this axis, or a girder beam on the same property.
        if other.encased && other.axis != axis {
            return state.with(prop, true);
        }
        if !other.encased && other.get(prop) {
            return state.with(prop, true);
        }
    }
    state
}

/// `GirderBlock.isConnected` for the side `d` of a girder or encased shaft (a horizontal side).
pub fn is_connected(state: &State, d: Dir, side: &Side) -> bool {
    let axis = d.axis();
    if !state.encased && !state.get(if axis == Axis::X { Prop::X } else { Prop::Z }) {
        return false;
    }
    if state.encased && state.axis == axis {
        return false;
    }
    if side.air || side.fluid || side.girder.is_some() {
        return false;
    }
    // `isFaceFull(shape, side.getOpposite()) && isSolid()`: any other block, see the header.
    true
}

/// `Iterate.directionsInAxis`: the negative direction first.
pub fn directions_in_axis(axis: Axis) -> [Dir; 2] {
    match axis {
        Axis::X => [Dir::West, Dir::East],
        Axis::Y => [Dir::Down, Dir::Up],
        Axis::Z => [Dir::North, Dir::South],
    }
}

/// `GirderBlock.updateShape` and `GirderEncasedShaftBlock.updateShape`: the state `state` takes
/// when the neighbour in `direction` changes. `side` answers what the block on each side is.
pub fn update_shape(mut state: State, direction: Dir, side: &dyn Fn(Dir) -> Side) -> State {
    if state.encased {
        if direction.axis() == Axis::Y {
            let prop = if direction == Dir::Up { Prop::Top } else { Prop::Bottom };
            let neighbour = side(direction);
            if neighbour.support_empty() {
                state = state.with(prop, false);
            }
            return update_vertical(state, prop, &neighbour, direction);
        }
        return state;
    }
    let axis = direction.axis();
    if axis != Axis::Y {
        if state.axis != axis {
            let prop = if axis == Axis::X { Prop::X } else { Prop::Z };
            if !is_connected(&state, direction, &side(direction))
                && !is_connected(&state, direction.opposite(), &side(direction.opposite()))
            {
                state = state.with(prop, false);
            }
        }
    } else if state.axis != Axis::Y {
        if side(Dir::Up).support_empty() {
            state = state.with(Prop::Top, false);
        }
        if side(Dir::Down).support_empty() {
            state = state.with(Prop::Bottom, false);
        }
    }
    for d in directions_in_axis(axis) {
        state = update_state(state, d, &side(d));
    }
    state
}

/// `GirderBlock.getStateForPlacement` (waterlogging aside): the beam runs across the face
/// clicked, then every side is looked at.
pub fn for_placement(face_axis: Axis, side: &dyn Fn(Dir) -> Side) -> State {
    let mut state = State {
        x: face_axis == Axis::X,
        z: face_axis == Axis::Z,
        axis: face_axis,
        ..State::GIRDER
    };
    for d in Dir::ALL {
        state = update_state(state, d, &side(d));
    }
    state
}

/// `GirderBlock.useItemOn` with a Shaft: the girder becomes an encased shaft, along z when the
/// beam runs along x or the face clicked looks along z.
pub fn encase_girder(girder: &State, clicked_axis: Axis) -> State {
    State {
        encased: true,
        x: false,
        z: false,
        top: girder.top,
        bottom: girder.bottom,
        axis: if girder.x || clicked_axis == Axis::Z { Axis::Z } else { Axis::X },
    }
}

/// `ShaftBlock.useItemOn` with a Metal Girder: a horizontal shaft takes a girder.
pub fn encase_shaft(shaft_axis: Axis) -> Option<State> {
    if shaft_axis == Axis::Y {
        return None;
    }
    Some(State {
        encased: true,
        x: false,
        z: false,
        top: false,
        bottom: false,
        axis: if shaft_axis == Axis::Z { Axis::Z } else { Axis::X },
    })
}

/// `GirderEncasedShaftBlock.getRotatedBlockState`: what the Wrench turns an encased shaft into.
pub fn girder_of(encased: &State) -> State {
    State {
        encased: false,
        x: encased.axis == Axis::Z,
        z: encased.axis == Axis::X,
        top: encased.top,
        bottom: encased.bottom,
        axis: if encased.axis == Axis::X { Axis::Z } else { Axis::X },
    }
}

// --- the Wrench (`GirderWrenchBehavior`) ------------------------------------------------------

/// `GirderWrenchBehavior.Action`: a single girder's connection, or a pair of girders' joined one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Single,
    Pair,
}

/// `GirderWrenchBehavior.getValidDirections`: the vertical connections the Wrench can toggle on a
/// girder, given the girders (or none) above and below it. Horizontal ones are commented out in
/// Create.
pub fn valid_directions(state: &State, above: Option<State>, below: Option<State>) -> Vec<(Dir, Action)> {
    let mut out = Vec::new();
    if state.encased || (!state.x && !state.z) {
        return out;
    }
    for (d, other) in [(Dir::Down, below), (Dir::Up, above)] {
        match other {
            // The other block is no girder: a lone beam can toggle its end.
            None => {
                if state.x != state.z {
                    out.push((d, Action::Single));
                }
            }
            Some(other) => {
                // This girder is a cross, or the other is a pole or cross: nothing.
                if state.x == state.z || other.encased || other.x == other.z {
                    continue;
                }
                out.push((d, Action::Pair));
            }
        }
    }
    out
}

/// `IPlacementHelper.orderedByDistance(pos, ray, directions)`: the directions by how far each
/// one's unit vector is from the click's offset from the block's centre.
pub fn ordered_by_distance(hit_from_centre: [f64; 3], allowed: &[Dir]) -> Vec<Dir> {
    let mut ranked: Vec<(Dir, f64)> = allowed
        .iter()
        .map(|d| {
            let n = d.normal();
            let dist = (0..3).map(|i| (f64::from(n[i]) - hit_from_centre[i]).powi(2)).sum::<f64>();
            (*d, dist.sqrt())
        })
        .collect();
    ranked.sort_by(|a, b| a.1.total_cmp(&b.1));
    ranked.into_iter().map(|(d, _)| d).collect()
}

/// `GirderWrenchBehavior.postProcess`: a beam with only one end joined takes the axis it runs
/// along.
pub fn post_process(state: State) -> State {
    if state.top && state.bottom {
        return state;
    }
    if state.axis != Axis::Y {
        return state;
    }
    State { axis: if state.x { Axis::X } else { Axis::Z }, ..state }
}

// --- the placement assist (`GirderPlacementHelper`) -------------------------------------------

/// `GirderPlacementHelper.canExtendToward`.
pub fn can_extend_toward(state: &State, side: Dir) -> bool {
    let axis = side.axis();
    if !state.encased {
        if !state.x && !state.z {
            return axis == Axis::Y;
        }
        if state.x && state.z {
            return true;
        }
        return axis == if state.x { Axis::X } else { Axis::Z };
    }
    axis != Axis::Y && axis != state.axis
}

/// `GirderPlacementHelper.withAxis`: the new block takes the axis the line runs along.
pub fn with_axis(state: &State, axis: Axis) -> State {
    if !state.encased {
        return State { x: axis == Axis::X, z: axis == Axis::Z, axis, ..*state };
    }
    if axis != Axis::Y {
        return State { axis: if axis == Axis::X { Axis::Z } else { Axis::X }, ..*state };
    }
    *state
}

/// `GirderPlacementHelper.attachedPoles`: how many girders already stand in a line from `pos`
/// towards `direction`.
fn attached_poles(pos: [i32; 3], direction: Dir, read: &dyn Fn([i32; 3]) -> Option<State>) -> i32 {
    let step = |p: [i32; 3]| {
        let n = direction.normal();
        [p[0] + n[0], p[1] + n[1], p[2] + n[2]]
    };
    let mut check = step(pos);
    let mut count = 0;
    while read(check).is_some_and(|state| can_extend_toward(&state, direction)) {
        count += 1;
        check = step(check);
    }
    count
}

/// `GirderPlacementHelper.getOffset`: where a girder item clicked on the girder `state` at `pos`
/// goes, and its state (before neighbours are looked at). `hit_from_centre` is where the click
/// landed relative to the block's centre; `read` answers the girder at a position, `replaceable`
/// whether a block can be built over (`canBeReplaced`).
pub fn extension(
    state: &State,
    pos: [i32; 3],
    hit_from_centre: [f64; 3],
    read: &dyn Fn([i32; 3]) -> Option<State>,
    replaceable: &dyn Fn([i32; 3]) -> bool,
) -> Option<([i32; 3], Axis)> {
    let allowed: Vec<Dir> = Dir::ALL.into_iter().filter(|d| can_extend_toward(state, *d)).collect();
    for dir in ordered_by_distance(hit_from_centre, &allowed) {
        let poles = attached_poles(pos, dir, read);
        if poles >= PLACEMENT_ASSIST_RANGE {
            continue;
        }
        let n = dir.normal();
        let at = [pos[0] + n[0] * (poles + 1), pos[1] + n[1] * (poles + 1), pos[2] + n[2] * (poles + 1)];
        if !replaceable(at) {
            continue;
        }
        return Some((at, dir.axis()));
    }
    None
}

/// What a click with the Wrench changes (`GirderWrenchBehavior.handleClick`): the clicked girder's
/// new state and, for a pair, the one above or below it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WrenchChange {
    pub this: State,
    /// Where the other girder is relative to this one, and its new state.
    pub other: Option<(Dir, State)>,
}

/// `GirderWrenchBehavior.handleClick` for the girder `state` with the girders `above` and `below`
/// it (none for another block): `None` when the click does nothing.
pub fn wrench_click(
    state: &State,
    above: Option<State>,
    below: Option<State>,
    hit_from_centre: [f64; 3],
) -> Option<WrenchChange> {
    let valid = valid_directions(state, above, below);
    if valid.is_empty() {
        return None;
    }
    let dirs: Vec<Dir> = valid.iter().map(|(d, _)| *d).collect();
    let dir = *ordered_by_distance(hit_from_centre, &dirs).first()?;
    let action = valid.iter().find(|(d, _)| *d == dir).map_or(Action::Single, |(_, a)| *a);
    let (mine, theirs, other) = if dir == Dir::Up {
        (Prop::Top, Prop::Bottom, above)
    } else {
        (Prop::Bottom, Prop::Top, below)
    };
    let this = post_process(state.cycled(mine));
    let other = match (action, other) {
        (Action::Pair, Some(o)) if !o.encased => Some((dir, post_process(o.cycled(theirs)))),
        _ => None,
    };
    Some(WrenchChange { this, other })
}

impl State {
    /// The state's properties as the game names them: what to write to the block.
    pub fn properties(&self) -> Vec<(&'static str, &'static str)> {
        let b = |v: bool| if v { "true" } else { "false" };
        let mut out = Vec::new();
        if !self.encased {
            out.push(("x", b(self.x)));
            out.push(("z", b(self.z)));
        }
        out.push(("top", b(self.top)));
        out.push(("bottom", b(self.bottom)));
        out.push(("axis", self.axis.name()));
        out
    }
}

/// `Block.updateFromNeighbourShapes`: the state after every neighbour has been asked.
pub fn settle(mut state: State, side: &dyn Fn(Dir) -> Side) -> State {
    for d in Dir::ALL {
        state = update_shape(state, d, side);
    }
    state
}

/// Where a player's look meets the block at `pos`, relative to the block's centre
/// (`BlockHitResult.getLocation() - center`): the eye at `eye`, looking along `yaw` and `pitch`
/// (degrees). The click's face centre when the look misses the block (a hit point `use_item_on`
/// does not carry).
pub fn hit_from_centre(eye: [f64; 3], yaw: f32, pitch: f32, pos: [i32; 3], face: Dir) -> [f64; 3] {
    let (yaw, pitch) = (f64::from(yaw).to_radians(), f64::from(pitch).to_radians());
    let look = [-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos()];
    let (mut near, mut far) = (0.0_f64, f64::INFINITY);
    for i in 0..3 {
        let lo = f64::from(pos[i]);
        if look[i].abs() < 1e-9 {
            if eye[i] < lo || eye[i] > lo + 1.0 {
                near = f64::INFINITY;
            }
            continue;
        }
        let (a, b) = ((lo - eye[i]) / look[i], (lo + 1.0 - eye[i]) / look[i]);
        near = near.max(a.min(b));
        far = far.min(a.max(b));
    }
    if near.is_finite() && near <= far {
        return std::array::from_fn(|i| eye[i] + look[i] * near - (f64::from(pos[i]) + 0.5));
    }
    face.normal().map(|v| f64::from(v) * 0.5)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nothing(_: Dir) -> Side {
        Side { air: true, ..Side::default() }
    }

    fn solid() -> Side {
        Side::default()
    }

    #[test]
    fn a_girder_placed_on_a_side_face_is_a_pole_across_it() {
        // Clicked on a face looking along x: the beam runs along x, joined at both ends.
        let state = for_placement(Axis::X, &nothing);
        assert!(state.x && !state.z && state.axis == Axis::X);
        // `state.axis == axis` switches on the west and east property (both X).
        assert!(!state.top && !state.bottom);
    }

    #[test]
    fn a_pole_placed_on_a_floor_is_joined_up_and_down() {
        let state = for_placement(Axis::Y, &nothing);
        assert_eq!(state, State { top: true, bottom: true, ..State::GIRDER });
        assert!(!state.x && !state.z);
    }

    #[test]
    fn a_beam_beside_a_girder_that_runs_the_same_way_joins_it() {
        let neighbour = State { x: true, axis: Axis::X, ..State::GIRDER };
        let side = move |d: Dir| {
            if d == Dir::East { Side { girder: Some(neighbour), ..Side::default() } } else { Side::default() }
        };
        // A state across a different axis whose east side has an x girder takes `x`.
        let state = update_state(State { axis: Axis::Z, ..State::GIRDER }, Dir::East, &side(Dir::East));
        assert!(state.x);
        let state = update_state(State { axis: Axis::Z, ..State::GIRDER }, Dir::West, &side(Dir::West));
        assert!(!state.x);
    }

    #[test]
    fn an_encased_shaft_across_the_axis_joins_a_beam() {
        let encased = State { encased: true, axis: Axis::X, ..State::GIRDER };
        let side = Side { girder: Some(encased), ..Side::default() };
        // The shaft runs along x: a beam along z reaches out to it from north and south, not
        // along x.
        assert!(update_state(State { axis: Axis::Y, ..State::GIRDER }, Dir::North, &side).z);
        assert!(!update_state(State { axis: Axis::Y, ..State::GIRDER }, Dir::East, &side).x);
    }

    #[test]
    fn losing_its_neighbours_cuts_a_beam_short_unless_it_runs_that_way() {
        // A girder with x joined and axis z (it was placed on a z face) loses the neighbour on its
        // east: both x sides are open, so x goes off.
        let state = State { x: true, axis: Axis::Z, ..State::GIRDER };
        let after = update_shape(state, Dir::East, &nothing);
        assert!(!after.x);
        // One side solid: it stays.
        let solid_west = |d: Dir| if d == Dir::West { solid() } else { nothing(d) };
        assert!(update_shape(state, Dir::East, &solid_west).x);
        // The axis it was placed along always keeps its property.
        let along = State { x: true, axis: Axis::X, ..State::GIRDER };
        assert!(update_shape(along, Dir::East, &nothing).x);
    }

    #[test]
    fn a_beam_loses_its_ends_when_there_is_nothing_above_or_below() {
        let state = State { x: true, axis: Axis::X, top: true, bottom: true, ..State::GIRDER };
        let after = update_shape(state, Dir::Up, &nothing);
        assert!(!after.top && !after.bottom);
        // A pole (axis y) keeps both ends however empty it is.
        let pole = State { top: true, bottom: true, ..State::GIRDER };
        assert!(update_shape(pole, Dir::Up, &nothing).top);
    }

    #[test]
    fn walls_lanterns_chains_and_buttons_take_a_pole() {
        let beam = State { x: true, axis: Axis::X, ..State::GIRDER };
        let wall = Side { up: true, ..Side::default() };
        assert!(update_vertical(beam, Prop::Top, &wall, Dir::Up).top);
        // A lantern hangs from below (d = down: hanging) or stands on top (d = up: not hanging).
        let hanging = Side { hanging: Some(true), ..Side::default() };
        let standing = Side { hanging: Some(false), ..Side::default() };
        assert!(update_vertical(beam, Prop::Bottom, &hanging, Dir::Down).bottom);
        assert!(!update_vertical(beam, Prop::Top, &hanging, Dir::Up).top);
        assert!(update_vertical(beam, Prop::Top, &standing, Dir::Up).top);
        let chain = Side { chain_y: true, ..Side::default() };
        assert!(update_vertical(beam, Prop::Top, &chain, Dir::Up).top);
        let button = Side { face: Some(Face::Ceiling), ..Side::default() };
        assert!(update_vertical(beam, Prop::Bottom, &button, Dir::Down).bottom);
        assert!(!update_vertical(beam, Prop::Top, &button, Dir::Up).top);
    }

    #[test]
    fn an_encased_shaft_keeps_the_ends_that_have_something() {
        let encased = State { encased: true, axis: Axis::Z, top: true, ..State::GIRDER };
        // Nothing above: the top goes.
        let none = |_: Dir| Side { air: true, ..Side::default() };
        assert!(!update_shape(encased, Dir::Up, &none).top);
        // A girder above: it stays (and `bottom` isn't touched by a change above).
        let above = |d: Dir| {
            if d == Dir::Up { Side { girder: Some(State::GIRDER), ..Side::default() } } else { none(d) }
        };
        assert!(update_shape(encased, Dir::Up, &above).top);
        // A horizontal change leaves it alone.
        assert_eq!(update_shape(encased, Dir::East, &none), encased);
    }

    #[test]
    fn a_shaft_clicked_into_a_girder_turns_across_the_beam() {
        let along_x = State { x: true, axis: Axis::X, top: true, ..State::GIRDER };
        let encased = encase_girder(&along_x, Axis::X);
        assert!(encased.encased && encased.axis == Axis::Z && encased.top);
        // A pole clicked on a z face: the shaft runs along z; on an x face, along x.
        assert_eq!(encase_girder(&State::GIRDER, Axis::Z).axis, Axis::Z);
        assert_eq!(encase_girder(&State::GIRDER, Axis::X).axis, Axis::X);
        assert_eq!(encase_shaft(Axis::Y), None);
        assert_eq!(encase_shaft(Axis::Z).unwrap().axis, Axis::Z);
        assert_eq!(encase_shaft(Axis::X).unwrap().axis, Axis::X);
    }

    #[test]
    fn the_wrench_turns_an_encased_shaft_back_into_a_beam_across_it() {
        let encased = State { encased: true, axis: Axis::Z, bottom: true, ..State::GIRDER };
        let girder = girder_of(&encased);
        assert!(girder.x && !girder.z && girder.axis == Axis::X && girder.bottom);
        let encased = State { encased: true, axis: Axis::X, ..State::GIRDER };
        let girder = girder_of(&encased);
        assert!(!girder.x && girder.z && girder.axis == Axis::Z);
    }

    #[test]
    fn the_wrench_toggles_the_ends_of_a_single_beam() {
        let beam = State { x: true, axis: Axis::X, ..State::GIRDER };
        assert_eq!(
            valid_directions(&beam, None, None),
            vec![(Dir::Down, Action::Single), (Dir::Up, Action::Single)]
        );
        // Poles and crosses have nothing to toggle.
        assert!(valid_directions(&State::GIRDER, None, None).is_empty());
        let cross = State { x: true, z: true, ..State::GIRDER };
        assert!(valid_directions(&cross, None, None).is_empty());
        // Two beams above each other toggle together.
        let pair = valid_directions(&beam, Some(beam), None);
        assert_eq!(pair, vec![(Dir::Down, Action::Single), (Dir::Up, Action::Pair)]);
        // A beam under a cross: nothing on that side.
        assert!(!valid_directions(&beam, Some(cross), None).iter().any(|(d, _)| *d == Dir::Up));
    }

    #[test]
    fn the_nearest_end_to_the_click_is_toggled() {
        let order = ordered_by_distance([0.0, 0.3, 0.0], &[Dir::Down, Dir::Up]);
        assert_eq!(order, vec![Dir::Up, Dir::Down]);
        let order = ordered_by_distance([0.1, -0.2, 0.0], &[Dir::Down, Dir::Up]);
        assert_eq!(order, vec![Dir::Down, Dir::Up]);
    }

    #[test]
    fn a_beam_with_one_end_joined_takes_the_axis_it_runs_along() {
        let beam = State { x: true, axis: Axis::Y, top: true, ..State::GIRDER };
        assert_eq!(post_process(beam).axis, Axis::X);
        let both = State { top: true, bottom: true, ..beam };
        assert_eq!(post_process(both).axis, Axis::Y);
        let z = State { z: true, axis: Axis::Y, ..State::GIRDER };
        assert_eq!(post_process(z).axis, Axis::Z);
        let already = State { x: true, axis: Axis::Z, ..State::GIRDER };
        assert_eq!(post_process(already).axis, Axis::Z);
    }

    #[test]
    fn a_line_of_girders_extends_along_its_beam() {
        let pole = State::GIRDER;
        assert!(can_extend_toward(&pole, Dir::Up) && !can_extend_toward(&pole, Dir::East));
        let beam = State { x: true, axis: Axis::X, ..State::GIRDER };
        assert!(can_extend_toward(&beam, Dir::West) && !can_extend_toward(&beam, Dir::North));
        let cross = State { x: true, z: true, ..State::GIRDER };
        assert!(Dir::ALL.iter().all(|d| can_extend_toward(&cross, *d)));
        let encased = State { encased: true, axis: Axis::Z, ..State::GIRDER };
        assert!(can_extend_toward(&encased, Dir::East) && !can_extend_toward(&encased, Dir::South));
        assert!(!can_extend_toward(&encased, Dir::Up));
    }

    #[test]
    fn an_extended_girder_takes_the_axis_of_the_line() {
        let beam = State { x: true, axis: Axis::X, ..State::GIRDER };
        let next = with_axis(&beam, Axis::Z);
        assert!(!next.x && next.z && next.axis == Axis::Z);
        let encased = State { encased: true, axis: Axis::Z, ..State::GIRDER };
        // The line runs along x: the shaft runs across it, along z.
        assert_eq!(with_axis(&encased, Axis::X).axis, Axis::Z);
        assert_eq!(with_axis(&encased, Axis::Z).axis, Axis::X);
        assert_eq!(with_axis(&encased, Axis::Y), encased);
    }

    #[test]
    fn extending_a_line_stops_at_the_range_and_at_solid_blocks() {
        let beam = State { x: true, axis: Axis::X, ..State::GIRDER };
        // Two girders already east of the clicked one: the new one is the third beyond.
        let read = |p: [i32; 3]| if (1..=2).contains(&p[0]) && p[1] == 0 && p[2] == 0 { Some(beam) } else { None };
        let free = |_: [i32; 3]| true;
        let hit = [0.45, 0.0, 0.0];
        assert_eq!(extension(&beam, [0, 0, 0], hit, &read, &free), Some(([3, 0, 0], Axis::X)));
        // Blocked there: the other end (west) is tried.
        let blocked = |p: [i32; 3]| p != [3, 0, 0];
        assert_eq!(extension(&beam, [0, 0, 0], hit, &read, &blocked), Some(([-1, 0, 0], Axis::X)));
        // A line already at the range is skipped.
        let long = |p: [i32; 3]| if (1..=20).contains(&p[0]) && p[1] == 0 && p[2] == 0 { Some(beam) } else { None };
        assert_eq!(extension(&beam, [0, 0, 0], hit, &long, &free), Some(([-1, 0, 0], Axis::X)));
    }

    #[test]
    fn a_wrench_click_on_a_pair_toggles_both_girders() {
        let beam = State { x: true, axis: Axis::Y, ..State::GIRDER };
        let change = wrench_click(&beam, Some(beam), None, [0.0, 0.4, 0.0]).unwrap();
        assert!(change.this.top);
        // One end joined: the beam takes the axis it runs along.
        assert_eq!(change.this.axis, Axis::X);
        let (dir, other) = change.other.unwrap();
        assert_eq!(dir, Dir::Up);
        assert!(other.bottom);
        // A lone beam toggles only itself; a pole does nothing.
        let single = wrench_click(&beam, None, None, [0.0, -0.4, 0.0]).unwrap();
        assert!(single.this.bottom && single.other.is_none());
        assert_eq!(wrench_click(&State::GIRDER, None, None, [0.0; 3]), None);
    }

    #[test]
    fn the_look_ray_meets_the_block_it_points_at() {
        // Eye 3 blocks south of the block, level with its upper part, looking north.
        let hit = hit_from_centre([0.5, 0.8, 3.5], 180.0, 0.0, [0, 0, 0], Dir::South);
        assert!(hit[0].abs() < 1e-6 && (hit[1] - 0.3).abs() < 1e-6 && (hit[2] - 0.5).abs() < 1e-6);
        // A miss gives the face's centre.
        let miss = hit_from_centre([10.5, 0.8, 3.5], 180.0, 0.0, [0, 0, 0], Dir::Up);
        assert_eq!(miss, [0.0, 0.5, 0.0]);
    }

    #[test]
    fn a_settled_girder_follows_what_is_around_it() {
        let state = State { x: true, axis: Axis::X, top: true, bottom: true, ..State::GIRDER };
        let air = |_: Dir| Side { air: true, ..Side::default() };
        let settled = settle(state, &air);
        assert!(settled.x && !settled.top && !settled.bottom);
        assert_eq!(state.properties().len(), 5);
        assert_eq!(State { encased: true, axis: Axis::Z, ..State::GIRDER }.properties().len(), 3);
    }
}
