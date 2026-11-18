//! The placement helpers for Shafts and Cogwheels: a cogwheel, large cogwheel or shaft held and aimed at an
//! existing one goes where it meshes (cogs) or where the line goes on (shafts), instead of at the face clicked.
//! Create shows the spot as a ghost and snaps the right click to it.
//!
//! Ported from Create `mc1.21.1-6.0.10` (MIT, the Creators of Create):
//! - `content/kinetics/simpleRelays/CogwheelBlockItem.java`: `onItemUseFirst`, `SmallCogHelper`, `LargeCogHelper`,
//!   `DiagonalCogHelper`, `IntegratedLargeCogHelper`, `IntegratedSmallCogHelper`;
//! - `content/kinetics/simpleRelays/CogWheelBlock.java`: `isValidCogwheelPosition`, `getAxisForPlacement`;
//! - `content/kinetics/simpleRelays/ShaftBlock.java`: `useItemOn` and its `PlacementHelper`;
//!   `content/kinetics/steamEngine/PoweredShaftBlock.java`: its `useItemOn`;
//! - `foundation/placement/PoleHelper.java` (the base of the shaft's helper);
//! - `content/kinetics/simpleRelays/ICogWheel.java`: the size and "dedicated" predicates;
//! - `AllShapes.java`: `SIX_VOXEL_POLE`, `SMALL_GEAR_SHAPE`, `LARGE_GEAR_SHAPE`.
//!
//! `IPlacementHelper`, `PlacementOffset` and `PlacementHelpers` are Catnip's (`net.createmod.catnip.placement`), which
//! is not in the Java tree here. They were read from the code Create 6.0.10 took them from, Create's own
//! `foundation/placement` of the `mc1.20.1/0.5.1` branch (`IPlacementHelper.java`, `PlacementOffset.java`,
//! `PlacementHelpers.java`, `foundation/utility/ghost/GhostBlocks.java`, `GhostBlockRenderer.java`); Catnip moved
//! those classes over, so what a 6.0.10 detail differs in, if any, is unchecked.
//!
//! The flow, as Create does it:
//! - **Use** (`on_click`): `CogwheelBlockItem.onItemUseFirst` runs first for a cogwheel item (a helper whose state
//!   predicate matches the clicked block and the player isn't sneaking decides the click; a helper that finds no
//!   spot answers PASS and the click goes on, without trying a second helper); then the clicked block's own
//!   `useItemOn`, which for a Shaft or Powered Shaft runs the shaft's pole helper when the player isn't sneaking.
//! - **Ghost** (`ghost`): `PlacementHelpers.checkHelpers` does nothing while sneaking; it keeps the helpers whose item
//!   predicate matches what is held, then the ones whose state predicate matches the looked-at block, and takes the
//!   first that finds a spot (so a second helper IS tried here), for the main hand and then the off hand.
//!
//! What the glue does with the answer (`machines/cog_placement.rs`): the server places the block, plays its place
//! sound and takes one item outside creative; a client only consumes the click.
//!
//! Left out, on purpose: `ShaftBlock.pickCorrectShaftType` (the placed shaft turning into a Powered Shaft where a
//! Steam Engine wants one; `machines/steam_engine.rs` converts a plain shaft there within a tick anyway), the
//! `EntityPlaceEvent` Create posts (the Wand of Symmetry listens to it), `world.mayInteract` and
//! `WATERLOGGED` (this port's shafts and cogs have no such property).

use crate::{Axis, Dir, Pos};

/// `AllConfigs.server().equipment.placementAssistRange` (`CEquipment`): `i(12, 3, ...)`.
pub const PLACEMENT_ASSIST_RANGE: i32 = 12;
/// `PoleHelper.getOffset`: `range += 4` while the player's reach carries the Extendo Grip's single-range modifier.
pub const EXTENDO_RANGE_BONUS: i32 = 4;

// --- what a block is to the helpers --------------------------------------------------------------------------

/// The blocks the helpers care about. Everything else is `None` in a [`Cell`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Piece {
    /// `AllBlocks.SHAFT` (`ShaftBlock`).
    Shaft,
    /// `AllBlocks.POWERED_SHAFT` (`PoweredShaftBlock`).
    PoweredShaft,
    /// `AllBlocks.COGWHEEL` and `LARGE_COGWHEEL` (`CogWheelBlock`, `isDedicatedCogWheel`).
    Cog { large: bool },
    /// The four encased cogwheels (`EncasedCogwheelBlock`): an `ICogWheel` that is not dedicated.
    EncasedCog { large: bool },
}

impl Piece {
    /// The block with the id `create:...`.
    pub fn of_block(id: &str) -> Option<Piece> {
        Some(match id {
            "create:shaft" => Piece::Shaft,
            "create:powered_shaft" => Piece::PoweredShaft,
            "create:cogwheel" => Piece::Cog { large: false },
            "create:large_cogwheel" => Piece::Cog { large: true },
            "create:andesite_encased_cogwheel" | "create:brass_encased_cogwheel" => Piece::EncasedCog { large: false },
            "create:andesite_encased_large_cogwheel" | "create:brass_encased_large_cogwheel" => {
                Piece::EncasedCog { large: true }
            }
            _ => return None,
        })
    }

    /// `ICogWheel.isSmallCog(block)`.
    pub fn is_small_cog(self) -> bool {
        matches!(self, Piece::Cog { large: false } | Piece::EncasedCog { large: false })
    }

    /// `ICogWheel.isLargeCog(block)`.
    pub fn is_large_cog(self) -> bool {
        matches!(self, Piece::Cog { large: true } | Piece::EncasedCog { large: true })
    }

    /// `ICogWheel.isDedicatedCogWheel(block)`: only `CogWheelBlock` says true.
    pub fn is_dedicated_cog(self) -> bool {
        matches!(self, Piece::Cog { .. })
    }

    /// `PoleHelper`'s state test in `ShaftBlock.PlacementHelper`: `instanceof AbstractSimpleShaftBlock` (a shaft or a
    /// dedicated cog) `|| instanceof PoweredShaftBlock`.
    fn in_pole_line(self) -> bool {
        matches!(self, Piece::Shaft | Piece::PoweredShaft | Piece::Cog { .. })
    }
}

/// The held item, when it is one the helpers act for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Held {
    /// `create:shaft`.
    Shaft,
    /// `create:cogwheel` or `create:large_cogwheel` (`CogwheelBlockItem`).
    Cog { large: bool },
}

impl Held {
    /// The item with the id `create:...`.
    pub fn of_item(id: &str) -> Option<Held> {
        Some(match id {
            "create:shaft" => Held::Shaft,
            "create:cogwheel" => Held::Cog { large: false },
            "create:large_cogwheel" => Held::Cog { large: true },
            _ => return None,
        })
    }

    /// The block the item places.
    pub fn block(self) -> &'static str {
        match self {
            Held::Shaft => "create:shaft",
            Held::Cog { large: false } => "create:cogwheel",
            Held::Cog { large: true } => "create:large_cogwheel",
        }
    }

    /// `ShaftBlock.PlacementHelper.getItemPredicate`: a `BlockItem` of an `AbstractSimpleShaftBlock`.
    fn matches_pole_helper(self) -> bool {
        true
    }

    /// `SmallCogHelper` and `LargeCogHelper`'s `getItemPredicate` (`isSmallCogItem`/`isLargeCogItem` and
    /// `isDedicatedCogItem`), and the integrated helpers' (the same two).
    fn matches_cog_helpers(self) -> Option<bool> {
        match self {
            Held::Cog { large } => Some(large),
            Held::Shaft => None,
        }
    }
}

/// What a position holds, as far as the helpers read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    /// Which of the blocks above it is, if any.
    pub piece: Option<Piece>,
    /// `BlockStateProperties.AXIS` of any block that has it (a log, a chain, a shaft): `hasProperty(AXIS)`.
    pub axis: Option<Axis>,
    /// `BlockState.canBeReplaced()` (air, grass, water, snow layers).
    pub replaceable: bool,
}

impl Cell {
    /// Nothing there that matters, and room to build.
    pub const AIR: Cell = Cell { piece: None, axis: None, replaceable: true };
    /// A block that is nothing to the helpers and can't be built over (or a position that can't be read).
    pub const SOLID: Cell = Cell { piece: None, axis: None, replaceable: false };
}

/// The world as a helper reads it.
pub trait World {
    /// The block at `pos`.
    fn cell(&self, pos: Pos) -> Cell;
}

/// The click (or the look) a helper answers: `getOffset(player, world, state, pos, ray)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Click {
    /// The looked-at or clicked block (`ray.getBlockPos()`).
    pub pos: Pos,
    /// `ray.getDirection()`, the face hit.
    pub face: Dir,
    /// `ray.getLocation()`, in world coordinates.
    pub hit: [f64; 3],
    /// `player.isShiftKeyDown()`.
    pub sneaking: bool,
    /// The player's reach carries the Extendo Grip's modifier (`PoleHelper`'s `range += 4`).
    pub extendo: bool,
}

/// Which helper found the spot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Helper {
    /// `ShaftBlock.PlacementHelper`.
    Pole,
    /// `SmallCogHelper` (on a small cog; on a large one it is `DiagonalCogHelper`'s rule, still this helper).
    SmallCog,
    /// `LargeCogHelper` (likewise).
    LargeCog,
    /// `IntegratedSmallCogHelper`.
    IntegratedSmall,
    /// `IntegratedLargeCogHelper`.
    IntegratedLarge,
}

/// `PlacementOffset.success(newPos, s -> s.setValue(AXIS, axis))`: where the held block goes and its `axis`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Offset {
    pub pos: Pos,
    pub axis: Axis,
    pub helper: Helper,
}

// --- IPlacementHelper's helpers ------------------------------------------------------------------------------

fn add(a: Pos, b: Pos) -> Pos {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn relative(pos: Pos, dir: Dir) -> Pos {
    add(pos, dir.normal())
}

/// `IPlacementHelper.orderedByDistance(pos, hit, includeDirection)`: the directions `keep` lets through, by how far
/// each one's unit vector (`Vec3.atLowerCornerOf(dir.getNormal())`) is from the hit's offset from the block's centre
/// (`VecHelper.getCenterOf(pos)`), nearest first; Java's `sorted` is stable, so ties stay in `Direction.values()`
/// order (down, up, north, south, west, east).
pub fn ordered_by_distance(pos: Pos, hit: [f64; 3], keep: impl Fn(Dir) -> bool) -> Vec<Dir> {
    let centre_to_hit = [
        hit[0] - (f64::from(pos[0]) + 0.5),
        hit[1] - (f64::from(pos[1]) + 0.5),
        hit[2] - (f64::from(pos[2]) + 0.5),
    ];
    let mut ranked: Vec<(Dir, f64)> = Dir::ALL
        .into_iter()
        .filter(|d| keep(*d))
        .map(|d| {
            let n = d.normal();
            let dist = (0..3).map(|i| (f64::from(n[i]) - centre_to_hit[i]).powi(2)).sum::<f64>().sqrt();
            (d, dist)
        })
        .collect();
    ranked.sort_by(|a, b| a.1.total_cmp(&b.1));
    ranked.into_iter().map(|(d, _)| d).collect()
}

/// `orderedByDistanceOnlyAxis(pos, hit, axis)`.
fn only_axis(pos: Pos, hit: [f64; 3], axis: Axis) -> Vec<Dir> {
    ordered_by_distance(pos, hit, |d| d.axis() == axis)
}

/// `orderedByDistanceExceptAxis(pos, hit, axis)`.
fn except_axis(pos: Pos, hit: [f64; 3], axis: Axis) -> Vec<Dir> {
    ordered_by_distance(pos, hit, |d| d.axis() != axis)
}

/// `CogWheelBlock.isValidCogwheelPosition(large, world, pos, cogAxis)`: no large cog (and, for a large one, no small
/// cog either) touches it from the sides around its axis, except one whose own `AXIS` points at it.
pub fn is_valid_cogwheel_position(large: bool, world: &dyn World, pos: Pos, cog_axis: Axis) -> bool {
    for facing in Dir::ALL {
        if facing.axis() == cog_axis {
            continue;
        }
        let neighbour = world.cell(relative(pos, facing));
        if neighbour.axis == Some(facing.axis()) {
            continue;
        }
        if let Some(piece) = neighbour.piece {
            if piece.is_large_cog() || (large && piece.is_small_cog()) {
                return false;
            }
        }
    }
    true
}

/// `DiagonalCogHelper.hitOnShaft`: `AllShapes.SIX_VOXEL_POLE.get(axis).bounds().inflate(0.001)` contains the hit
/// taken relative to `hit.align(Iterate.axisSet)` (each coordinate floored, so the cell the HIT is in, which is the
/// next block over for a hit on the upper face). `AABB.contains` is half-open: min inclusive, max exclusive.
pub fn hit_on_shaft(axis: Axis, hit: [f64; 3]) -> bool {
    // `forAxis`: the pole's cross section is 5/16 to 11/16 on the two other axes, the whole block along `axis`.
    (0..3).all(|i| {
        let local = hit[i] - hit[i].floor();
        let (lo, hi) = if i == axis.index() { (0.0, 1.0) } else { (5.0 / 16.0, 11.0 / 16.0) };
        local >= lo - 0.001 && local < hi + 0.001
    })
}

// --- the cogwheel helpers --------------------------------------------------------------------------------------

/// `DiagonalCogHelper.getStatePredicate`: `isSmallCog(s) || isLargeCog(s)`.
fn diagonal_state(piece: Piece) -> bool {
    piece.is_small_cog() || piece.is_large_cog()
}

/// `IntegratedSmallCogHelper` and `IntegratedLargeCogHelper`'s `getStatePredicate`: `!isDedicatedCogWheel && isSmallCog`.
fn integrated_state(piece: Piece) -> bool {
    !piece.is_dedicated_cog() && piece.is_small_cog()
}

/// `DiagonalCogHelper.getOffset`: a cog of the other size, diagonal in the plane of the clicked one.
fn diagonal(item_large: bool, click: &Click, clicked: Piece, axis: Axis, world: &dyn World) -> Option<Offset> {
    let helper = if item_large { Helper::LargeCog } else { Helper::SmallCog };
    let closest = *except_axis(click.pos, click.hit, axis).first()?;
    for dir in ordered_by_distance(click.pos, click.hit, |d| d.axis() != axis && d.axis() != closest.axis()) {
        let new_pos = relative(relative(click.pos, dir), closest);
        if !world.cell(new_pos).replaceable {
            continue;
        }
        if !is_valid_cogwheel_position(clicked.is_large_cog(), world, new_pos, axis) {
            continue;
        }
        return Some(Offset { pos: new_pos, axis, helper });
    }
    None
}

/// `SmallCogHelper.getOffset`.
fn small_cog(click: &Click, clicked: Piece, axis: Axis, world: &dyn World) -> Option<Offset> {
    if hit_on_shaft(axis, click.hit) {
        return None;
    }
    if !clicked.is_large_cog() {
        for dir in except_axis(click.pos, click.hit, axis) {
            let new_pos = relative(click.pos, dir);
            if !is_valid_cogwheel_position(false, world, new_pos, axis) {
                continue;
            }
            if !world.cell(new_pos).replaceable {
                continue;
            }
            return Some(Offset { pos: new_pos, axis, helper: Helper::SmallCog });
        }
        return None;
    }
    diagonal(false, click, clicked, axis, world)
}

/// `LargeCogHelper.getOffset`.
fn large_cog(click: &Click, clicked: Piece, axis: Axis, world: &dyn World) -> Option<Offset> {
    if hit_on_shaft(axis, click.hit) {
        return None;
    }
    if clicked.is_large_cog() {
        let side = *only_axis(click.pos, click.hit, axis).first()?;
        for dir in except_axis(click.pos, click.hit, axis) {
            let new_pos = relative(relative(click.pos, dir), side);
            if !is_valid_cogwheel_position(true, world, new_pos, dir.axis()) {
                continue;
            }
            if !world.cell(new_pos).replaceable {
                continue;
            }
            return Some(Offset { pos: new_pos, axis: dir.axis(), helper: Helper::LargeCog });
        }
        return None;
    }
    diagonal(true, click, clicked, axis, world)
}

/// `IntegratedSmallCogHelper.getOffset`. `new_axis` is the encased cog's axis: its `HORIZONTAL_FACING` and `FACING`
/// branches never apply to the state the predicate lets in (an encased small cog has only `AXIS`).
fn integrated_small(click: &Click, new_axis: Axis, world: &dyn World) -> Option<Offset> {
    if click.face.axis() == new_axis {
        return None;
    }
    for d in except_axis(click.pos, click.hit, new_axis) {
        let new_pos = relative(click.pos, d);
        if !world.cell(new_pos).replaceable {
            continue;
        }
        if !is_valid_cogwheel_position(false, world, new_pos, new_axis) {
            return None;
        }
        return Some(Offset { pos: new_pos, axis: new_axis, helper: Helper::IntegratedSmall });
    }
    None
}

/// `IntegratedLargeCogHelper.getOffset`.
fn integrated_large(click: &Click, new_axis: Axis, world: &dyn World) -> Option<Offset> {
    let face = click.face;
    if face.axis() == new_axis {
        return None;
    }
    for d in ordered_by_distance(click.pos, click.hit, |d| d.axis() != face.axis() && d.axis() != new_axis) {
        let new_pos = relative(relative(click.pos, face), d);
        if !world.cell(new_pos).replaceable {
            continue;
        }
        // Create asks `isValidCogwheelPosition(false, ...)` here too, for the large cog.
        if !is_valid_cogwheel_position(false, world, new_pos, new_axis) {
            return None;
        }
        return Some(Offset { pos: new_pos, axis: new_axis, helper: Helper::IntegratedLarge });
    }
    None
}

// --- the shaft's helper ----------------------------------------------------------------------------------------

/// `PoleHelper.attachedPoles`: the shafts and cogs of axis `direction.axis()` in a line from `pos`.
fn attached_poles(world: &dyn World, pos: Pos, direction: Dir) -> i32 {
    let mut check = relative(pos, direction);
    let mut count = 0;
    loop {
        let cell = world.cell(check);
        // `matchesAxis`: the state predicate, then `axisFunction.apply(state) == axis`.
        if cell.piece.is_some_and(Piece::in_pole_line) && cell.axis == Some(direction.axis()) {
            count += 1;
            check = relative(check, direction);
        } else {
            return count;
        }
    }
}

/// `PoleHelper.getOffset` through `ShaftBlock.PlacementHelper.getOffset`: a shaft or cog goes on at the end of the
/// line of shafts and cogs along `axis`, the end nearest the hit first, with the same axis.
fn pole(click: &Click, axis: Axis, world: &dyn World) -> Option<Offset> {
    for dir in ordered_by_distance(click.pos, click.hit, |d| d.axis() == axis) {
        let range = PLACEMENT_ASSIST_RANGE + if click.extendo { EXTENDO_RANGE_BONUS } else { 0 };
        let poles = attached_poles(world, click.pos, dir);
        if poles >= range {
            continue;
        }
        let n = dir.normal();
        let step = poles + 1;
        let new_pos = add(click.pos, [n[0] * step, n[1] * step, n[2] * step]);
        if world.cell(new_pos).replaceable {
            return Some(Offset { pos: new_pos, axis, helper: Helper::Pole });
        }
    }
    None
}

// --- the flows ------------------------------------------------------------------------------------------------------

/// The looked-at block's piece and rotation axis, when it has both.
fn looked(world: &dyn World, pos: Pos) -> Option<(Piece, Axis)> {
    let cell = world.cell(pos);
    Some((cell.piece?, cell.axis?))
}

/// The pole helper (`ShaftBlock.placementHelperId`) for `held` on the clicked `piece`; None when its predicates fail.
fn pole_helper(held: Held, click: &Click, piece: Piece, axis: Axis, world: &dyn World) -> Option<Offset> {
    // `getStatePredicate`: `AllBlocks.SHAFT::has` or `POWERED_SHAFT::has`; `getItemPredicate`: see `Held`.
    if !matches!(piece, Piece::Shaft | Piece::PoweredShaft) || !held.matches_pole_helper() {
        return None;
    }
    pole(click, axis, world)
}

/// `CogwheelBlockItem`'s main helper (`SmallCogHelper` or `LargeCogHelper`, by the item) for the clicked `piece`;
/// None when its predicates fail, `Some(None)` when they pass and it finds no spot.
fn cog_helper(held: Held, click: &Click, piece: Piece, axis: Axis, world: &dyn World) -> Option<Option<Offset>> {
    let large = held.matches_cog_helpers()?;
    if !diagonal_state(piece) {
        return None;
    }
    Some(if large { large_cog(click, piece, axis, world) } else { small_cog(click, piece, axis, world) })
}

/// `CogwheelBlockItem`'s integrated helper for the clicked `piece`; same convention as [`cog_helper`].
fn integrated_helper(held: Held, click: &Click, piece: Piece, axis: Axis, world: &dyn World) -> Option<Option<Offset>> {
    let large = held.matches_cog_helpers()?;
    if !integrated_state(piece) {
        return None;
    }
    Some(if large { integrated_large(click, axis, world) } else { integrated_small(click, axis, world) })
}

/// What a right click with `held` on the block at `click.pos` does: the spot the block is placed at, or None when
/// the click goes on as usual (PASS).
///
/// `CogwheelBlockItem.onItemUseFirst` (cog items only), then the clicked block's `useItemOn`
/// (`ShaftBlock`, `PoweredShaftBlock`). Both leave a sneaking player alone.
pub fn on_click(held: Held, click: &Click, world: &dyn World) -> Option<Offset> {
    if click.sneaking {
        return None;
    }
    let (piece, axis) = looked(world, click.pos)?;
    if matches!(held, Held::Cog { .. }) {
        // `if (helper.matchesState(state) && ...) return helper.getOffset(...).placeInWorld(...).result();`
        if let Some(answer) = cog_helper(held, click, piece, axis, world) {
            return answer;
        }
        if let Some(answer) = integrated_helper(held, click, piece, axis, world) {
            return answer;
        }
    }
    pole_helper(held, click, piece, axis, world)
}

/// The spot the placement ghost shows while `held` is aimed at `click.pos` (`PlacementHelpers.checkHelpers` for one
/// hand): nothing while sneaking, else the first helper, in registration order (the shaft's, then the cogwheel
/// item's main and integrated ones), whose item and state predicates match and that finds a spot.
pub fn ghost(held: Held, click: &Click, world: &dyn World) -> Option<Offset> {
    if click.sneaking {
        return None;
    }
    let (piece, axis) = looked(world, click.pos)?;
    if let Some(offset) = pole_helper(held, click, piece, axis, world) {
        return Some(offset);
    }
    if let Some(Some(offset)) = cog_helper(held, click, piece, axis, world) {
        return Some(offset);
    }
    integrated_helper(held, click, piece, axis, world).flatten()
}

// --- the cogwheel item's own placement (`CogWheelBlock.getAxisForPlacement`) -------------------------------------

/// `CogWheelBlock.getAxisForPlacement(context)`, with each read of the world asked only when Create reaches it.
///
/// - `below_controller`: the `HORIZONTAL_AXIS` of a Rotation Speed Controller in the block below, if there is one;
/// - `placed_against_cog`: the axis of the block clicked (`pos.relative(face.opposite())`) when it is a small cog
///   (`ICogWheel.isSmallCog`, encased ones too);
/// - `preferred`: `RotatedPillarKineticBlock.getPreferredAxis` (the axis the neighbouring shaft ends agree on).
pub fn axis_for_placement(
    large: bool,
    sneaking: bool,
    clicked_face: Dir,
    below_controller: impl FnOnce() -> Option<Axis>,
    placed_against_cog: impl FnOnce() -> Option<Axis>,
    preferred: impl FnOnce() -> Option<Axis>,
) -> Axis {
    // `if (player != null && player.isShiftKeyDown()) return context.getClickedFace().getAxis();`
    if sneaking {
        return clicked_face.axis();
    }
    // `if (ROTATION_SPEED_CONTROLLER.has(stateBelow) && isLargeCog()) return HORIZONTAL_AXIS == X ? Z : X;`
    if large {
        if let Some(horizontal) = below_controller() {
            return if horizontal == Axis::X { Axis::Z } else { Axis::X };
        }
    }
    // `if (ICogWheel.isSmallCog(placedAgainst)) return placedAgainst's rotation axis;`
    if let Some(axis) = placed_against_cog() {
        return axis;
    }
    // `preferredAxis != null ? preferredAxis : context.getClickedFace().getAxis()`
    preferred().unwrap_or_else(|| clicked_face.axis())
}

// --- the ghost and the arrow (`GhostBlocks`, `TransparentGhostBlockRenderer`, `PlacementHelpers`) --------------

/// `TransparentGhostBlockRenderer`: `ms.scale(.85f, .85f, .85f)` about the block's centre.
pub const GHOST_SCALE: f64 = 0.85;
/// `TransparentGhostBlockRenderer`: `params.alphaSupplier.get() * .75f * PlacementHelpers.getCurrentAlpha()`.
pub const GHOST_ALPHA: f32 = 0.75;

/// `GhostBlocks.getBreathingAlpha`: `0.55 - 0.2 * cos(2 pi * (millis % 2500) / 2500)`.
pub fn breathing_alpha(millis: f64) -> f32 {
    let period = 2500.0;
    let timer = millis.rem_euclid(period);
    (0.55 - 0.2 * ((2.0 / period) * std::f64::consts::PI * timer).cos()) as f32
}

/// `PlacementHelpers.tick`'s counter: up by one while a helper has a target (to 10), down by two when it has none.
pub fn animation_step(animation_tick: i32, has_target: bool) -> i32 {
    if has_target {
        if animation_tick < 10 { animation_tick + 1 } else { animation_tick }
    } else if animation_tick > 0 {
        (animation_tick - 2).max(0)
    } else {
        animation_tick
    }
}

/// `PlacementHelpers.getCurrentAlpha`: `min(animationTick / 10f, 1f)`.
pub fn current_alpha(animation_tick: i32) -> f32 {
    (animation_tick as f32 / 10.0).min(1.0)
}

/// The box a ghost of `held` is drawn with, in block-local coordinates, for the axis the block would have
/// (`AllShapes.SIX_VOXEL_POLE`, `SMALL_GEAR_SHAPE`, `LARGE_GEAR_SHAPE` `.add(SIX_VOXEL_POLE)`, `forAxis()`), before
/// [`GHOST_SCALE`]. Each is `[x0, y0, z0, x1, y1, z1]` in blocks.
pub fn ghost_boxes(held: Held, axis: Axis) -> Vec<[f64; 6]> {
    // Written for axis Y, in sixteenths.
    let pole = [5.0, 0.0, 5.0, 11.0, 16.0, 11.0];
    let mut boxes = match held {
        Held::Shaft => vec![pole],
        // `cuboid(2, 6, 2, 14, 10, 14)` and `cuboid(0, 6, 0, 16, 10, 16)`.
        Held::Cog { large: false } => vec![[2.0, 6.0, 2.0, 14.0, 10.0, 14.0], pole],
        Held::Cog { large: true } => vec![[0.0, 6.0, 0.0, 16.0, 10.0, 16.0], pole],
    };
    for b in &mut boxes {
        // `forAxis`: the Y shape turned so its long side runs along `axis`.
        *b = match axis {
            Axis::Y => *b,
            Axis::X => [b[1], b[0], b[2], b[4], b[3], b[5]],
            Axis::Z => [b[0], b[2], b[1], b[3], b[5], b[4]],
        };
    }
    boxes.into_iter().map(|b| b.map(|v| v / 16.0)).collect()
}

/// The angle `PlacementHelpers.drawDirectionIndicator` points at, in degrees clockwise from straight up, for the
/// target's direction on screen (`x` to the right, `y` up; both zero means no direction and gives 0). A target behind
/// the camera (`behind`) points the opposite way (`target.reverse()`).
pub fn target_angle(x: f64, y: f64, behind: bool) -> f32 {
    let (x, y) = if behind { (-x, -y) } else { (x, y) };
    let length = (x * x + y * y).sqrt();
    if length == 0.0 {
        return 0.0;
    }
    let (nx, ny) = (x / length, y / length);
    // `acos(norm.dot((0, 1, 0)))`, `if (norm.x < 0) 360 - angle`.
    let mut angle = ny.clamp(-1.0, 1.0).acos().to_degrees();
    if nx < 0.0 {
        angle = 360.0 - angle;
    }
    angle as f32
}

/// One `LerpedFloat.angular().chase(target, .25f, Chaser.EXP)` step: a quarter of the way along the shortest turn.
pub fn chase_angle(value: f32, target: f32) -> f32 {
    let mut diff = (target - value).rem_euclid(360.0);
    if diff > 180.0 {
        diff -= 360.0;
    }
    if diff.abs() < 1e-3 {
        return target.rem_euclid(360.0);
    }
    (value + diff * 0.25).rem_euclid(360.0)
}

/// The triangle `PlacementHelpers.fadedArrow` fans out, pointing up, before the angle turns it and `indicatorScale`
/// (1) scales it: the apex (alpha `a`) and the rim points (alpha 0), in GUI pixels from the crosshair.
pub const ARROW_APEX: [f32; 2] = [0.0, -(10.0 + ARROW_LENGTH)];
/// `float length = 10`.
pub const ARROW_LENGTH: f32 = 10.0;
/// The rim of the fan: `(-9, -3)`, `(-6, -6)`, `(-3, -8)`, `(0, -8.5)`, `(3, -8)`, `(6, -6)`, `(9, -3)`.
pub const ARROW_RIM: [[f32; 2]; 7] =
    [[-9.0, -3.0], [-6.0, -6.0], [-3.0, -8.0], [0.0, -8.5], [3.0, -8.0], [6.0, -6.0], [9.0, -3.0]];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Cells by position; everything else is air.
    #[derive(Default)]
    struct Grid(HashMap<Pos, Cell>);

    impl Grid {
        fn with(mut self, pos: Pos, piece: Piece, axis: Axis) -> Self {
            self.0.insert(pos, Cell { piece: Some(piece), axis: Some(axis), replaceable: false });
            self
        }
        fn solid(mut self, pos: Pos) -> Self {
            self.0.insert(pos, Cell::SOLID);
            self
        }
        fn log(mut self, pos: Pos, axis: Axis) -> Self {
            self.0.insert(pos, Cell { piece: None, axis: Some(axis), replaceable: false });
            self
        }
    }

    impl World for Grid {
        fn cell(&self, pos: Pos) -> Cell {
            self.0.get(&pos).copied().unwrap_or(Cell::AIR)
        }
    }

    const SMALL: Piece = Piece::Cog { large: false };
    const LARGE: Piece = Piece::Cog { large: true };

    /// A click on the block at the origin: the hit is `at` blocks from its centre (`x`, `y`, `z`).
    fn click(face: Dir, at: [f64; 3]) -> Click {
        Click { pos: [0, 0, 0], face, hit: [0.5 + at[0], 0.5 + at[1], 0.5 + at[2]], sneaking: false, extendo: false }
    }

    // --- IPlacementHelper.orderedByDistance ---

    #[test]
    fn directions_are_ordered_by_distance_from_the_hit_and_ties_keep_the_enum_order() {
        // Hit on the +x side of the centre: east first. Distance to a unit vector of the hit's own side is smallest.
        let order = ordered_by_distance([0, 0, 0], [0.9, 0.5, 0.5], |_| true);
        assert_eq!(order[0], Dir::East);
        assert_eq!(*order.last().unwrap(), Dir::West);
        // Dead centre: all six are 1 away; the order is `Direction.values()`.
        let order = ordered_by_distance([0, 0, 0], [0.5, 0.5, 0.5], |_| true);
        assert_eq!(order, Dir::ALL.to_vec());
        // The filter drops directions before the ranking: only the axis.
        let order = ordered_by_distance([3, 4, 5], [3.5, 4.5, 5.9], |d| d.axis() == Axis::Z);
        assert_eq!(order, vec![Dir::South, Dir::North]);
    }

    // --- isValidCogwheelPosition ---

    #[test]
    fn a_cog_position_is_invalid_beside_a_large_cog_unless_its_axis_points_at_it() {
        // A large cog east of the spot, along z: the spot (axis y) is beside its side, so invalid.
        let world = Grid::default().with([1, 0, 0], LARGE, Axis::Z);
        assert!(!is_valid_cogwheel_position(false, &world, [0, 0, 0], Axis::Y));
        // The same cog with its axis x (pointing at the spot): `facing.getAxis() == AXIS` skips it.
        let world = Grid::default().with([1, 0, 0], LARGE, Axis::X);
        assert!(is_valid_cogwheel_position(false, &world, [0, 0, 0], Axis::Y));
        // Neighbours along the cog's own axis are never asked: the large cog above a y cog.
        let world = Grid::default().with([0, 1, 0], LARGE, Axis::Z);
        assert!(is_valid_cogwheel_position(false, &world, [0, 0, 0], Axis::Y));
    }

    #[test]
    fn a_large_cog_position_is_also_invalid_beside_a_small_cog_but_a_small_one_is_not() {
        let world = Grid::default().with([0, 0, 1], SMALL, Axis::X);
        assert!(!is_valid_cogwheel_position(true, &world, [0, 0, 0], Axis::Y));
        assert!(is_valid_cogwheel_position(false, &world, [0, 0, 0], Axis::Y));
        // An encased small cog counts as a small cog (`ICogWheel.isSmallCog`).
        let world = Grid::default().with([0, 0, 1], Piece::EncasedCog { large: false }, Axis::X);
        assert!(!is_valid_cogwheel_position(true, &world, [0, 0, 0], Axis::Y));
        // Any block with an AXIS along the direction to it is skipped, whatever it is: a log pointing along z.
        let world = Grid::default().log([0, 0, 1], Axis::Z);
        assert!(is_valid_cogwheel_position(true, &world, [0, 0, 0], Axis::Y));
    }

    // --- hitOnShaft ---

    #[test]
    fn a_hit_in_the_six_voxel_pole_is_on_the_shaft() {
        // Axis y: the pole is 5/16 to 11/16 on x and z. The block's middle is in it; its edge is not.
        assert!(hit_on_shaft(Axis::Y, [10.5, 64.9, -3.5]));
        assert!(!hit_on_shaft(Axis::Y, [10.1, 64.5, -3.5]));
        // Axis x: the pole runs along x, so a hit near the x edge is still on it but near the y edge is not.
        assert!(hit_on_shaft(Axis::X, [0.05, 0.5, 0.5]));
        assert!(!hit_on_shaft(Axis::X, [0.5, 0.1, 0.5]));
        // `inflate(0.001)`: 5/16 - 0.0005 is in, 5/16 - 0.002 is out.
        assert!(hit_on_shaft(Axis::Y, [5.0 / 16.0 - 0.0005, 0.5, 0.5]));
        assert!(!hit_on_shaft(Axis::Y, [5.0 / 16.0 - 0.002, 0.5, 0.5]));
        // Aligned to the cell the hit is in: a hit exactly on the top face of block 0 has local y 0, which a pole
        // along x (5/16 to 11/16 on y) doesn't contain.
        assert!(!hit_on_shaft(Axis::X, [0.5, 1.0, 0.5]));
    }

    // --- SmallCogHelper on a small cog ---

    #[test]
    fn a_small_cog_on_a_small_cog_goes_beside_it_on_the_side_nearest_the_hit() {
        // Cog at the origin, axis y. Hit on its +x edge (not on the shaft), item small.
        let world = Grid::default().with([0, 0, 0], SMALL, Axis::Y);
        let c = click(Dir::Up, [0.45, 0.0, 0.1]);
        let offset = ghost(Held::Cog { large: false }, &c, &world).unwrap();
        assert_eq!(offset, Offset { pos: [1, 0, 0], axis: Axis::Y, helper: Helper::SmallCog });
        // The right click does the same.
        assert_eq!(on_click(Held::Cog { large: false }, &c, &world), Some(offset));
    }

    #[test]
    fn a_small_cog_helper_skips_a_side_that_is_not_free_or_not_valid_and_takes_the_next_nearest() {
        // East is taken by stone: the next nearest to a hit at (+0.45, 0, +0.1) is south (z+).
        let world = Grid::default().with([0, 0, 0], SMALL, Axis::Y).solid([1, 0, 0]);
        let c = click(Dir::Up, [0.45, 0.0, 0.1]);
        let offset = on_click(Held::Cog { large: false }, &c, &world).unwrap();
        assert_eq!(offset.pos, [0, 0, 1]);
        // East is free but a large cog beside that spot (south of it, side-on) makes it invalid: also skipped.
        let world = Grid::default().with([0, 0, 0], SMALL, Axis::Y).with([2, 0, 0], LARGE, Axis::Z);
        let offset = on_click(Held::Cog { large: false }, &c, &world).unwrap();
        assert_eq!(offset.pos, [0, 0, 1]);
    }

    #[test]
    fn a_cog_helper_does_nothing_on_the_shaft_of_a_cog_and_the_click_goes_on() {
        let world = Grid::default().with([0, 0, 0], SMALL, Axis::Y);
        // The top face's middle is in the pole.
        let c = click(Dir::Up, [0.0, 0.5, 0.0]);
        assert_eq!(on_click(Held::Cog { large: false }, &c, &world), None);
        assert_eq!(ghost(Held::Cog { large: false }, &c, &world), None);
    }

    #[test]
    fn a_sneaking_player_gets_no_cog_helper_and_no_ghost() {
        let world = Grid::default().with([0, 0, 0], SMALL, Axis::Y);
        let mut c = click(Dir::Up, [0.45, 0.0, 0.1]);
        c.sneaking = true;
        assert_eq!(on_click(Held::Cog { large: false }, &c, &world), None);
        assert_eq!(ghost(Held::Cog { large: false }, &c, &world), None);
    }

    // --- LargeCogHelper on a large cog ---

    #[test]
    fn a_large_cog_on_a_large_cog_goes_diagonally_across_the_side_nearest_the_hit_at_right_angles() {
        // Large cog at the origin, axis y; the item is large. Hit on the +y side (the cog's slab is thin, the click on
        // the top face near the +x edge), so `side` is up, the nearest direction along y; the nearest horizontal
        // direction is east: the new cog goes at east+up, with the axis of that direction (x).
        let world = Grid::default().with([0, 0, 0], LARGE, Axis::Y);
        let c = click(Dir::Up, [0.45, 0.3, 0.1]);
        let offset = ghost(Held::Cog { large: true }, &c, &world).unwrap();
        assert_eq!(offset, Offset { pos: [1, 1, 0], axis: Axis::X, helper: Helper::LargeCog });
        assert_eq!(on_click(Held::Cog { large: true }, &c, &world), Some(offset));
    }

    #[test]
    fn a_large_cog_helper_takes_the_next_direction_when_the_first_is_blocked() {
        let world = Grid::default().with([0, 0, 0], LARGE, Axis::Y).solid([1, 1, 0]);
        let c = click(Dir::Up, [0.45, 0.3, 0.1]);
        // Next nearest horizontal direction to the hit is south (z+): new cog at south+up, axis z.
        let offset = on_click(Held::Cog { large: true }, &c, &world).unwrap();
        assert_eq!(offset, Offset { pos: [0, 1, 1], axis: Axis::Z, helper: Helper::LargeCog });
    }

    #[test]
    fn a_large_cog_on_a_large_cog_below_the_slab_goes_down() {
        let world = Grid::default().with([0, 0, 0], LARGE, Axis::Y);
        let c = click(Dir::Down, [0.0, -0.3, 0.45]);
        // `side` = down; nearest horizontal direction to the hit (+z) is south: at south+down with axis z.
        let offset = on_click(Held::Cog { large: true }, &c, &world).unwrap();
        assert_eq!(offset, Offset { pos: [0, -1, 1], axis: Axis::Z, helper: Helper::LargeCog });
    }

    // --- DiagonalCogHelper: cogs of different sizes ---

    #[test]
    fn a_large_cog_on_a_small_cog_goes_diagonally_in_the_same_plane_with_the_same_axis() {
        // Small cog at the origin, axis y; the item is large. `closest` is east (hit near +x); the directions are
        // the ones not on east's axis: north and south, nearest to the hit first (south, the hit is at +z).
        let world = Grid::default().with([0, 0, 0], SMALL, Axis::Y);
        let c = click(Dir::Up, [0.45, 0.0, 0.1]);
        let offset = ghost(Held::Cog { large: true }, &c, &world).unwrap();
        assert_eq!(offset, Offset { pos: [1, 0, 1], axis: Axis::Y, helper: Helper::LargeCog });
        // The validity check is made with the clicked cog's size (small): a large cog across the way at
        // (2, 0, 1) side-on is a large cog: invalid, so north is taken instead.
        let world = Grid::default().with([0, 0, 0], SMALL, Axis::Y).with([2, 0, 1], LARGE, Axis::Z);
        let offset = ghost(Held::Cog { large: true }, &c, &world).unwrap();
        assert_eq!(offset.pos, [1, 0, -1]);
    }

    #[test]
    fn a_small_cog_on_a_large_cog_goes_diagonally_and_is_checked_as_a_large_one() {
        // Large cog at the origin, axis y, item small. closest = east; south first.
        let world = Grid::default().with([0, 0, 0], LARGE, Axis::Y);
        let c = click(Dir::Up, [0.45, 0.0, 0.1]);
        let offset = ghost(Held::Cog { large: false }, &c, &world).unwrap();
        assert_eq!(offset, Offset { pos: [1, 0, 1], axis: Axis::Y, helper: Helper::SmallCog });
        // `isValidCogwheelPosition(ICogWheel.isLargeCog(state)=true, ...)`: a SMALL cog beside the spot, side-on,
        // makes it invalid for a small item on a large cog. (2, 0, 1) east of the spot, axis z.
        let world = Grid::default().with([0, 0, 0], LARGE, Axis::Y).with([2, 0, 1], SMALL, Axis::Z);
        let offset = ghost(Held::Cog { large: false }, &c, &world).unwrap();
        assert_eq!(offset.pos, [1, 0, -1]);
    }

    #[test]
    fn the_diagonal_helper_gives_up_when_every_diagonal_is_taken() {
        let world = Grid::default().with([0, 0, 0], SMALL, Axis::Y).solid([1, 0, 1]).solid([1, 0, -1]);
        let c = click(Dir::Up, [0.45, 0.0, 0.1]);
        assert_eq!(on_click(Held::Cog { large: true }, &c, &world), None);
    }

    // --- the integrated helpers (encased cogs) ---

    #[test]
    fn the_main_cog_helper_handles_an_encased_small_cog_so_the_integrated_one_is_only_a_ghost_fallback() {
        // An encased small cog, axis y, a small cog item: the main helper matches its state (it is a small cog), so
        // the click is its: beside it. The integrated helper is never asked by `onItemUseFirst`.
        let world = Grid::default().with([0, 0, 0], Piece::EncasedCog { large: false }, Axis::Y);
        let c = click(Dir::Up, [0.45, 0.0, 0.1]);
        let offset = on_click(Held::Cog { large: false }, &c, &world).unwrap();
        assert_eq!(offset.helper, Helper::SmallCog);
        assert_eq!(offset.pos, [1, 0, 0]);
        // Hit on its shaft: the main helper answers PASS and `onItemUseFirst` returns, with no second helper.
        let c = click(Dir::Up, [0.0, 0.5, 0.0]);
        assert_eq!(on_click(Held::Cog { large: false }, &c, &world), None);
    }

    #[test]
    fn the_ghost_falls_back_to_the_integrated_small_helper_when_the_main_one_finds_nothing() {
        // A hit inside the shaft's tube on a face that is not along the cog's axis (a click on a block shape that is
        // not a full cube): `hitOnShaft` fails the main helper; the integrated small one then runs. Encased small
        // cog along x at the origin, face north, hit at the centre: every direction ties, so `Direction.values()`
        // order decides and the first one except x is down.
        let world = Grid::default().with([0, 0, 0], Piece::EncasedCog { large: false }, Axis::X);
        let c = Click { pos: [0, 0, 0], face: Dir::North, hit: [0.5, 0.5, 0.5], sneaking: false, extendo: false };
        assert!(hit_on_shaft(Axis::X, c.hit));
        let offset = ghost(Held::Cog { large: false }, &c, &world).unwrap();
        assert_eq!(offset, Offset { pos: [0, -1, 0], axis: Axis::X, helper: Helper::IntegratedSmall });
        // The right click doesn't: the main helper matched the state, found nothing, and `onItemUseFirst` returned.
        assert_eq!(on_click(Held::Cog { large: false }, &c, &world), None);
    }

    #[test]
    fn the_integrated_small_helper_places_beside_the_clicked_cog_on_the_nearest_side() {
        // Called directly: encased small cog along x at the origin, clicked on its north face, hit towards +y.
        // `orderedByDistanceExceptAxis(pos, hit, x)`: the nearest of up, down, north, south to the hit (0.5, 0.9, 0.0):
        // north is 0.5 away in z and 0.4 in y -> distance sqrt(0.5^2... ): compute: centre (0.5,0.5,0.5), hit - centre
        // = (0, 0.4, -0.5). Distances: up (0,1,0): sqrt(0.6^2+0.5^2)=0.781; north (0,0,-1): sqrt(0.4^2+0.5^2)=0.640;
        // so north first.
        let world = Grid::default().with([0, 0, 0], Piece::EncasedCog { large: false }, Axis::X);
        let c = Click { pos: [0, 0, 0], face: Dir::North, hit: [0.5, 0.9, 0.0], sneaking: false, extendo: false };
        let offset = integrated_small(&c, Axis::X, &world).unwrap();
        assert_eq!(offset, Offset { pos: [0, 0, -1], axis: Axis::X, helper: Helper::IntegratedSmall });
        // A face on the cog's own axis fails.
        let c = Click { face: Dir::East, ..c };
        assert_eq!(integrated_small(&c, Axis::X, &world), None);
    }

    #[test]
    fn the_integrated_small_helper_fails_outright_when_the_first_free_spot_is_invalid() {
        // North of the cog is free but a large cog sits side-on beside it: `return PlacementOffset.fail()`, with no
        // second direction tried.
        let world = Grid::default()
            .with([0, 0, 0], Piece::EncasedCog { large: false }, Axis::X)
            .with([0, 0, -2], LARGE, Axis::Y);
        let c = Click { pos: [0, 0, 0], face: Dir::North, hit: [0.5, 0.9, 0.0], sneaking: false, extendo: false };
        assert_eq!(integrated_small(&c, Axis::X, &world), None);
        // A blocked first spot is skipped instead (`continue`): north is stone, so up is next.
        let world = Grid::default().with([0, 0, 0], Piece::EncasedCog { large: false }, Axis::X).solid([0, 0, -1]);
        let offset = integrated_small(&c, Axis::X, &world).unwrap();
        assert_eq!(offset.pos, [0, 1, 0]);
    }

    #[test]
    fn the_integrated_large_helper_goes_one_block_out_of_the_face_and_across() {
        // Encased small cog along x at the origin, clicked on its north face; the item is large.
        // face = north (axis z), newAxis = x: the directions not on z or x are up and down; the hit is above the
        // centre, so up first. newPos = pos + north + up.
        let world = Grid::default().with([0, 0, 0], Piece::EncasedCog { large: false }, Axis::X);
        let c = Click { pos: [0, 0, 0], face: Dir::North, hit: [0.5, 0.9, 0.0], sneaking: false, extendo: false };
        let offset = integrated_large(&c, Axis::X, &world).unwrap();
        assert_eq!(offset, Offset { pos: [0, 1, -1], axis: Axis::X, helper: Helper::IntegratedLarge });
        // The same face as the cog's axis fails.
        let c = Click { face: Dir::West, ..c };
        assert_eq!(integrated_large(&c, Axis::X, &world), None);
    }

    #[test]
    fn the_integrated_helpers_only_match_an_encased_small_cog_and_a_dedicated_cog_item() {
        assert!(integrated_state(Piece::EncasedCog { large: false }));
        assert!(!integrated_state(SMALL));
        assert!(!integrated_state(Piece::EncasedCog { large: true }));
        assert!(!integrated_state(Piece::Shaft));
        // A shaft item has no cog helper at all.
        let world = Grid::default().with([0, 0, 0], SMALL, Axis::Y);
        let c = click(Dir::Up, [0.45, 0.0, 0.1]);
        assert_eq!(ghost(Held::Shaft, &c, &world), None);
        assert_eq!(on_click(Held::Shaft, &c, &world), None);
    }

    // --- the shaft's pole helper ---

    #[test]
    fn a_shaft_item_on_a_shaft_extends_the_line_at_the_end_nearest_the_hit() {
        // Shaft along x at the origin; the hit is on its +x half.
        let world = Grid::default().with([0, 0, 0], Piece::Shaft, Axis::X);
        let c = click(Dir::North, [0.4, 0.0, -0.3]);
        let offset = on_click(Held::Shaft, &c, &world).unwrap();
        assert_eq!(offset, Offset { pos: [1, 0, 0], axis: Axis::X, helper: Helper::Pole });
        assert_eq!(ghost(Held::Shaft, &c, &world), Some(offset));
        // On the -x half it goes the other way.
        let c = click(Dir::North, [-0.4, 0.0, -0.3]);
        assert_eq!(on_click(Held::Shaft, &c, &world).unwrap().pos, [-1, 0, 0]);
    }

    #[test]
    fn a_shaft_line_is_followed_to_its_end_and_counts_shafts_and_cogs_and_powered_shafts() {
        // Along y: shaft, then a cog, a powered shaft, then free air above. Clicked on the bottom shaft, hit on top.
        let world = Grid::default()
            .with([0, 0, 0], Piece::Shaft, Axis::Y)
            .with([0, 1, 0], SMALL, Axis::Y)
            .with([0, 2, 0], Piece::PoweredShaft, Axis::Y);
        let c = click(Dir::East, [0.2, 0.4, 0.0]);
        assert_eq!(on_click(Held::Shaft, &c, &world).unwrap().pos, [0, 3, 0]);
        // A cog item goes there too: `ShaftBlock`'s helper takes any `AbstractSimpleShaftBlock` item, with the
        // shaft's axis.
        let offset = on_click(Held::Cog { large: true }, &c, &world).unwrap();
        assert_eq!((offset.pos, offset.axis), ([0, 3, 0], Axis::Y));
        // An encased cog or a shaft pointing another way ends the line (`matchesAxis`): the cog above is along x.
        let world = Grid::default()
            .with([0, 0, 0], Piece::Shaft, Axis::Y)
            .with([0, 1, 0], SMALL, Axis::X);
        // The next block up isn't in the line, but it is also not replaceable, so up fails and down is tried.
        let offset = on_click(Held::Shaft, &c, &world).unwrap();
        assert_eq!(offset.pos, [0, -1, 0]);
    }

    #[test]
    fn a_shaft_line_stops_at_the_range_and_the_extendo_grip_adds_four() {
        let mut world = Grid::default().with([0, 0, 0], Piece::Shaft, Axis::X);
        for x in 1..=11 {
            world = world.with([x, 0, 0], Piece::Shaft, Axis::X);
        }
        // 11 shafts after the clicked one on the +x side: 11 < 12, so the spot is at x = 12.
        let mut c = click(Dir::North, [0.4, 0.0, -0.3]);
        assert_eq!(on_click(Held::Shaft, &c, &world).unwrap().pos, [12, 0, 0]);
        // One more makes 12: at the range, so the +x end is skipped and the -x end is used.
        world = world.with([12, 0, 0], Piece::Shaft, Axis::X);
        assert_eq!(on_click(Held::Shaft, &c, &world).unwrap().pos, [-1, 0, 0]);
        // The Extendo Grip: range 16, the +x end has room again.
        c.extendo = true;
        assert_eq!(on_click(Held::Shaft, &c, &world).unwrap().pos, [13, 0, 0]);
    }

    #[test]
    fn the_pole_helper_tries_the_other_end_when_the_nearest_is_not_replaceable_and_fails_when_both_are_blocked() {
        let world = Grid::default().with([0, 0, 0], Piece::Shaft, Axis::Z).solid([0, 0, 1]);
        let c = click(Dir::Up, [0.0, 0.2, 0.4]);
        assert_eq!(on_click(Held::Shaft, &c, &world).unwrap().pos, [0, 0, -1]);
        let world = world.solid([0, 0, -1]);
        assert_eq!(on_click(Held::Shaft, &c, &world), None);
        assert_eq!(ghost(Held::Shaft, &c, &world), None);
    }

    #[test]
    fn the_pole_helper_applies_to_a_shaft_and_a_powered_shaft_only_never_a_cog_for_the_use_click() {
        // A shaft item on a cog: `CogWheelBlock.useItemOn` has no helper, and the cog item helpers need a cog item.
        let world = Grid::default().with([0, 0, 0], SMALL, Axis::Y);
        let c = click(Dir::Up, [0.45, 0.0, 0.1]);
        assert_eq!(on_click(Held::Shaft, &c, &world), None);
        // A cog item on a powered shaft: `PoweredShaftBlock.useItemOn` runs the shaft's helper.
        let world = Grid::default().with([0, 0, 0], Piece::PoweredShaft, Axis::Z);
        let c = click(Dir::Up, [0.0, 0.2, 0.4]);
        let offset = on_click(Held::Cog { large: false }, &c, &world).unwrap();
        assert_eq!((offset.pos, offset.axis, offset.helper), ([0, 0, 1], Axis::Z, Helper::Pole));
        // A sneaking player gets nothing.
        let mut c = c;
        c.sneaking = true;
        assert_eq!(on_click(Held::Cog { large: false }, &c, &world), None);
    }

    #[test]
    fn a_cog_item_on_a_shaft_uses_the_pole_helper_not_the_cog_ones() {
        // `onItemUseFirst`: no cog helper matches a shaft's state, so the click reaches `ShaftBlock.useItemOn`.
        let world = Grid::default().with([0, 0, 0], Piece::Shaft, Axis::X);
        let c = click(Dir::Up, [0.4, 0.2, 0.0]);
        let offset = on_click(Held::Cog { large: true }, &c, &world).unwrap();
        assert_eq!((offset.pos, offset.axis, offset.helper), ([1, 0, 0], Axis::X, Helper::Pole));
    }

    // --- getAxisForPlacement ---

    #[test]
    fn a_cog_placed_while_sneaking_takes_the_clicked_faces_axis() {
        let axis = axis_for_placement(false, true, Dir::East, || panic!("not asked"), || panic!("not asked"), || panic!("not asked"));
        assert_eq!(axis, Axis::X);
    }

    #[test]
    fn a_large_cog_above_a_speed_controller_turns_across_it() {
        // `HORIZONTAL_AXIS == X ? Z : X`.
        let on_x = axis_for_placement(true, false, Dir::Up, || Some(Axis::X), || panic!("not asked"), || panic!("not asked"));
        assert_eq!(on_x, Axis::Z);
        let on_z = axis_for_placement(true, false, Dir::Up, || Some(Axis::Z), || panic!("not asked"), || panic!("not asked"));
        assert_eq!(on_z, Axis::X);
        // A small cog ignores the controller and goes on to the next rule.
        let small = axis_for_placement(false, false, Dir::Up, || panic!("not asked"), || Some(Axis::Z), || None);
        assert_eq!(small, Axis::Z);
    }

    #[test]
    fn a_cog_placed_against_a_small_cog_takes_its_axis_before_the_preferred_one() {
        let axis = axis_for_placement(true, false, Dir::North, || None, || Some(Axis::X), || panic!("not asked"));
        assert_eq!(axis, Axis::X);
    }

    #[test]
    fn a_cog_takes_the_preferred_axis_and_otherwise_the_clicked_faces() {
        let preferred = axis_for_placement(false, false, Dir::North, || None, || None, || Some(Axis::Y));
        assert_eq!(preferred, Axis::Y);
        let clicked = axis_for_placement(false, false, Dir::North, || None, || None, || None);
        assert_eq!(clicked, Axis::Z);
    }

    // --- the ghost and the arrow ---

    #[test]
    fn the_breathing_alpha_swings_between_a_third_and_three_quarters_over_2500_ms() {
        assert!((breathing_alpha(0.0) - 0.35).abs() < 1e-6);
        assert!((breathing_alpha(1250.0) - 0.75).abs() < 1e-6);
        assert!((breathing_alpha(2500.0) - 0.35).abs() < 1e-6);
        assert!((breathing_alpha(625.0) - 0.55).abs() < 1e-6);
    }

    #[test]
    fn the_indicator_fades_in_over_ten_ticks_and_out_in_five() {
        let mut tick = 0;
        for _ in 0..4 {
            tick = animation_step(tick, true);
        }
        assert_eq!(tick, 4);
        assert!((current_alpha(tick) - 0.4).abs() < 1e-6);
        for _ in 0..20 {
            tick = animation_step(tick, true);
        }
        assert_eq!(tick, 10);
        assert_eq!(current_alpha(tick), 1.0);
        let mut ticks = 0;
        while tick > 0 {
            tick = animation_step(tick, false);
            ticks += 1;
        }
        assert_eq!(ticks, 5);
        assert_eq!(animation_step(0, false), 0);
    }

    #[test]
    fn the_arrow_points_at_the_target_on_screen() {
        // Straight up is 0; right is 90; down 180; left 270.
        assert!((target_angle(0.0, 1.0, false)).abs() < 1e-4);
        assert!((target_angle(1.0, 0.0, false) - 90.0).abs() < 1e-4);
        assert!((target_angle(0.0, -1.0, false) - 180.0).abs() < 1e-4);
        assert!((target_angle(-1.0, 0.0, false) - 270.0).abs() < 1e-4);
        // Behind the camera the direction is reversed.
        assert!((target_angle(1.0, 0.0, true) - 270.0).abs() < 1e-4);
        assert_eq!(target_angle(0.0, 0.0, false), 0.0);
    }

    #[test]
    fn the_arrow_turns_a_quarter_of_the_way_by_the_short_side() {
        assert!((chase_angle(0.0, 80.0) - 20.0).abs() < 1e-4);
        // From 350 to 10 the short way is 20 degrees forward.
        assert!((chase_angle(350.0, 10.0) - 355.0).abs() < 1e-4);
        // And back the other way.
        assert!((chase_angle(10.0, 350.0) - 5.0).abs() < 1e-4);
        // Close enough snaps.
        assert_eq!(chase_angle(90.0, 90.0), 90.0);
    }

    #[test]
    fn the_ghost_boxes_follow_the_shapes_of_the_blocks() {
        let close = |a: [f64; 6], b: [f64; 6]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9);
        // The shaft along y: 5 to 11 sixteenths.
        let shaft = ghost_boxes(Held::Shaft, Axis::Y);
        assert_eq!(shaft.len(), 1);
        assert!(close(shaft[0], [5.0 / 16.0, 0.0, 5.0 / 16.0, 11.0 / 16.0, 1.0, 11.0 / 16.0]));
        // The small cog along x: the slab is 6 to 10 on x, 2 to 14 on y and z, and the pole runs along x.
        let cog = ghost_boxes(Held::Cog { large: false }, Axis::X);
        assert_eq!(cog.len(), 2);
        assert!(close(cog[0], [6.0 / 16.0, 2.0 / 16.0, 2.0 / 16.0, 10.0 / 16.0, 14.0 / 16.0, 14.0 / 16.0]));
        assert!(close(cog[1], [0.0, 5.0 / 16.0, 5.0 / 16.0, 1.0, 11.0 / 16.0, 11.0 / 16.0]));
        // The large cog along z: the slab spans the block on x and y.
        let large = ghost_boxes(Held::Cog { large: true }, Axis::Z);
        assert!(close(large[0], [0.0, 0.0, 6.0 / 16.0, 1.0, 1.0, 10.0 / 16.0]));
    }

    #[test]
    fn items_and_blocks_are_told_apart_by_id() {
        assert_eq!(Held::of_item("create:shaft"), Some(Held::Shaft));
        assert_eq!(Held::of_item("create:large_cogwheel"), Some(Held::Cog { large: true }));
        assert_eq!(Held::of_item("create:andesite_casing"), None);
        assert_eq!(Held::Cog { large: false }.block(), "create:cogwheel");
        assert_eq!(Piece::of_block("create:powered_shaft"), Some(Piece::PoweredShaft));
        assert_eq!(Piece::of_block("create:brass_encased_large_cogwheel"), Some(Piece::EncasedCog { large: true }));
        assert_eq!(Piece::of_block("minecraft:oak_log"), None);
        assert!(Piece::EncasedCog { large: false }.is_small_cog());
        assert!(!Piece::EncasedCog { large: false }.is_dedicated_cog());
        assert!(Piece::Cog { large: true }.is_dedicated_cog());
    }
}
