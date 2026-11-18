//! The placement helpers for Shafts and Cogwheels (`CogwheelBlockItem`'s helpers, `ShaftBlock.PlacementHelper`,
//! `CogWheelBlock.getAxisForPlacement`, Catnip's `PlacementHelpers` ghost and arrow). Every rule is in
//! `create_kinetics::rules::cog_placement`; this reads the world, asks the rules and writes what they say.
//!
//! Written on the new mod API directly (no compat `Hooks`): `lib.rs`'s `Mod` calls into this module from
//! `right_click_block` (both sides), `submit_custom_geometry` and `render_gui_post` (the ghost and the arrow), and
//! `client_tick_post` (`PlacementHelpers.tick`), and `parts.rs` asks [`state_for_placement`] first for the cogwheels.
//!
//! - **Server**, on the click: the helper's spot gets the held block with the helper's axis, its place sound plays
//!   and one item is taken outside creative, and the click is consumed.
//! - **Client**, on the same click: the rules decide the same thing from the client's own reads and the click is
//!   consumed, so the game does not predict a block at the face clicked. (The packet still goes to the server: a
//!   cancelled `RightClickBlock` is answered there too.)
//! - **Ghost**: the shape of the block that would be placed, at 0.85 scale and with Create's breathing alpha, as
//!   translucent quads and an outline; **arrow**: `PlacementHelpers`' faded triangle at the crosshair, turned towards
//!   the target.
//!
//! Known differences from Create, for the README:
//! - The ghost is not a tinted copy of the block's model: `draw_model_instances` refuses translucent models
//!   (`InstanceError::Translucent`) and a tint's alpha reaches only faces with a tint index, so the block's real
//!   boxes (`AllShapes`' pole and gear slab) are drawn as faint white quads with an outline instead.
//! - The arrow is Create's `TRIANGLE` indicator, its fade done with ten nested copies of the fan (the HUD has no
//!   per-vertex alpha). Create's default is the `TEXTURE` sheet, which is Create's art and is not shipped.
//!   The arrow's direction is worked out from the camera (`VecHelper.projectToPlayerView` is Catnip's, unread).
//! - The Extendo Grip's `+4` range is taken from the player's reach (`block_interaction_range` above 6), since this
//!   port gives the grip's reach as a base value, not Create's modifier.
//! - `world.mayInteract`, the `EntityPlaceEvent` (the Wand of Symmetry), the placed-block statistic and trigger,
//!   `ShaftBlock.pickCorrectShaftType` (see the rules' header) and waterlogging are left out.
//! - Only a block that has an `axis` property reads as having `AXIS` (`isValidCogwheelPosition`): a nether portal's
//!   `HORIZONTAL_AXIS` is also called `axis` here.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use create_kinetics::generators::preferred_shaft;
use create_kinetics::rules::cog_placement::{self as rules, Cell, Click, Held, Offset, Piece, World};
use create_kinetics::rules::extendo_grip::{BASE_REACH, SINGLE_RANGE};
use create_kinetics::{Axis, Dir, Pos};

use mod_api::{BlockMetadata, HostBlockStates};

use crate::compat::Hooks;
use crate::kinetic;

/// The module list wants a `Hooks` of every file; the new API's hooks are called from `lib.rs`.
struct Shared;
impl Hooks for Shared {}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

const REACH: &str = "minecraft:block_interaction_range";
/// Two grips make 9.5 and one 7.5; no grip 4.5, or 5 in creative. Anything over this is a grip in a hand.
const EXTENDO_REACH: f64 = BASE_REACH + SINGLE_RANGE / 2.0;

fn lock<T>(mutex: &'static Mutex<T>) -> MutexGuard<'static, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// --- reading the world -------------------------------------------------------------------------------------

/// What each block is to the helpers, by block: a block's name never changes.
static PIECES: Mutex<Option<HashMap<mod_api::BlockKey, Option<Piece>>>> = Mutex::new(None);
/// And each item.
static HELD: Mutex<Option<HashMap<mod_api::ItemKey, Option<Held>>>> = Mutex::new(None);

fn piece_of(host: &dyn mod_api::Host, block: mod_api::BlockKey) -> Option<Piece> {
    let mut table = lock(&PIECES);
    *table.get_or_insert_with(HashMap::new).entry(block).or_insert_with(|| Piece::of_block(&block.id(host)))
}

fn held_of(host: &dyn mod_api::Host, stack: &mod_api::ItemStack) -> Option<Held> {
    let item = stack.item();
    let mut table = lock(&HELD);
    *table.get_or_insert_with(HashMap::new).entry(item).or_insert_with(|| Held::of_item(&item.id(host)))
}

/// The world as the rules read it, on either side: the level of the host's dimension.
struct Reader<'a> {
    host: &'a dyn mod_api::Host,
    level: mod_api::LevelRef,
}

impl<'a> Reader<'a> {
    fn new(host: &'a dyn mod_api::Host) -> Option<Self> {
        Some(Self { host, level: host.level(host.dimension())? })
    }
}

impl World for Reader<'_> {
    fn cell(&self, pos: Pos) -> Cell {
        // A position the client doesn't hold: nothing to read and nothing to build on.
        let Some(state) = self.level.block_state(self.host, pos) else { return Cell::SOLID };
        let facts = self.host.block_states();
        let axis = facts
            .property(mod_api::BlockStateRecord::from(&state), "axis")
            .ok()
            .flatten()
            .and_then(Axis::from_name);
        Cell {
            piece: piece_of(self.host, state.block()),
            axis,
            replaceable: facts.replaceable(state).unwrap_or(false),
        }
    }
}

fn dir_of(face: [i32; 3]) -> Option<Dir> {
    Dir::from_normal(face)
}

/// Whether the player's reach carries the Extendo Grip's bonus.
fn extended(host: &dyn mod_api::Host, entity: Option<mod_api::EntityRef>) -> bool {
    entity
        .and_then(|entity| host.living(entity))
        .and_then(|living| living.attribute_value(host, REACH))
        .is_some_and(|reach| reach > EXTENDO_REACH)
}

// --- the click ---------------------------------------------------------------------------------------------

/// `right_click_block` on either side. True when a helper placed the block (the server) or would have (a client):
/// the click is then consumed with `Success`, which stops the game from placing a block at the clicked face.
pub(crate) fn right_click_block(
    host: &mut mod_api::HostSide<'_>,
    event: &mut mod_api::RightClickBlockEvent<'_>,
) -> mod_api::Flow {
    let click = event.click;
    let Some(stack) = click.stack else { return mod_api::Flow::Continue };
    let used = match host {
        mod_api::HostSide::Server(host) => place_on_server(&mut **host, click, stack),
        mod_api::HostSide::Client(host) => {
            let host: &dyn mod_api::ClientHost = &**host;
            let entity = host.camera_entity();
            let sneaking = entity.and_then(|entity| entity.is_shift_key_down(host)).unwrap_or(false);
            decide(host, click, stack, sneaking, entity).is_some()
        }
    };
    if used {
        *event.cancellation_result = mod_api::InteractionResult::Success;
        mod_api::Flow::Cancel
    } else {
        mod_api::Flow::Continue
    }
}

/// `CogwheelBlockItem.onItemUseFirst` and `ShaftBlock.useItemOn` for this click, from whichever side asks.
fn decide(
    host: &dyn mod_api::Host,
    click: &mod_api::BlockClick<'_>,
    stack: &mod_api::ItemStack,
    sneaking: bool,
    entity: Option<mod_api::EntityRef>,
) -> Option<(Held, Offset)> {
    let held = held_of(host, stack)?;
    let reader = Reader::new(host)?;
    let c = Click {
        pos: click.pos,
        face: dir_of(click.hit.face)?,
        hit: click.hit.point,
        sneaking,
        extendo: extended(host, entity),
    };
    rules::on_click(held, &c, &reader).map(|offset| (held, offset))
}

/// The held block's state with the helper's axis.
fn state_of(host: &dyn mod_api::Host, held: Held, axis: Axis) -> Option<mod_api::BlockState> {
    let default = host.block_state_id(held.block())?;
    let record = host.block_states().with(mod_api::BlockStateRecord::from(&default), "axis", axis.name()).ok()?;
    Some(record.into())
}

/// `PlacementOffset.placeInWorld` on the server.
fn place_on_server(
    host: &mut dyn mod_api::ServerHost,
    click: &mod_api::BlockClick<'_>,
    stack: &mod_api::ItemStack,
) -> bool {
    let Some(player) = host.player(click.player) else { return false };
    // `player.mayBuild()` (`ShaftBlock.useItemOn`): an adventure or spectator player builds nothing.
    if matches!(player.game_mode(&*host), Some(mod_api::GameMode::Adventure | mod_api::GameMode::Spectator)) {
        return false;
    }
    let sneaking = player.is_secondary_use_active(&*host).unwrap_or(false);
    let Some((held, offset)) = decide(&*host, click, stack, sneaking, Some(player.entity())) else { return false };
    let Some(state) = state_of(&*host, held, offset.axis) else { return false };
    let Some(level) = host.level(host.dimension()) else { return false };
    // `world.setBlockAndUpdate(newPos, state)` and `Block.setPlacedBy`.
    if player.place_block(host, level, offset.pos, state, true).is_err() {
        return false;
    }
    // `soundtype.getPlaceSound()` at `(volume + 1) / 2` and `pitch * 0.8`, `SoundSource.BLOCKS`.
    let sound = host.block_states().sound(state);
    if let Some(sound) = sound {
        let at = [f64::from(offset.pos[0]) + 0.5, f64::from(offset.pos[1]) + 0.5, f64::from(offset.pos[2]) + 0.5];
        level.play_sound(host, &sound.place, at, mod_api::SoundSource::Blocks, (sound.volume + 1.0) / 2.0, sound.pitch * 0.8);
    }
    // `if (!player.isCreative()) context.getItemInHand().shrink(1);`
    if player.game_mode(&*host) != Some(mod_api::GameMode::Creative) {
        let slot = match click.hand {
            mod_api::Hand::Main => mod_api::EquipmentSlot::MainHand,
            mod_api::Hand::Off => mod_api::EquipmentSlot::OffHand,
        };
        if let Some(living) = host.living(player.entity()) {
            let rest = living.equipment(&*host, slot).and_then(|held| held.shrink(1));
            let _ = living.set_equipment(host, slot, rest);
        }
    }
    true
}

// --- the cogwheel item's own placement ---------------------------------------------------------------------

/// `CogWheelBlock.getStateForPlacement` (its `getAxisForPlacement`) for the cogwheel and the large cogwheel, when
/// the click is not a helper's. None for every other block, so the old modules answer as before.
pub(crate) fn state_for_placement(
    level: &dyn mod_api::LevelHost,
    event: &mut mod_api::StateForPlacementEvent<'_>,
) -> Option<Option<mod_api::BlockState>> {
    let large = match Piece::of_block(level.state_block(event.state)?)? {
        Piece::Cog { large } => large,
        _ => return None,
    };
    let context = event.context;
    let pos: Pos = [context.pos.x, context.pos.y, context.pos.z];
    let clicked_face = crate::dir(context.face);
    // What a position holds, read through the level (names, and the `axis` of a block that has one).
    let read = |at: Pos| {
        let state = level.block_state(at)?;
        let name = level.state_block(state)?;
        let axis = Axis::ALL
            .into_iter()
            .find(|axis| level.state_with_property(state, "axis", axis.name()) == Some(state));
        Some((name, axis, state))
    };
    let axis = rules::axis_for_placement(
        large,
        context.sneaking,
        clicked_face,
        || {
            // `AllBlocks.ROTATION_SPEED_CONTROLLER.has(stateBelow)`: its `HORIZONTAL_AXIS` is its `axis` here.
            let (name, axis, _) = read([pos[0], pos[1] - 1, pos[2]])?;
            (name == "create:rotation_speed_controller").then_some(axis).flatten()
        },
        || {
            // `ICogWheel.isSmallCog(placedAgainst)`: the block clicked, `pos.relative(face.opposite())`.
            let n = clicked_face.opposite().normal();
            let (name, axis, _) = read([pos[0] + n[0], pos[1] + n[1], pos[2] + n[2]])?;
            Piece::of_block(name).filter(|piece| piece.is_small_cog()).and(axis)
        },
        || {
            // `RotatedPillarKineticBlock.getPreferredAxis`: the shaft ends beside the spot, from the table of what
            // the network knows (a block placed in the last moments isn't in it).
            let dimension = match event.dimension {
                mod_api::LevelDimension::Nether => "minecraft:the_nether",
                mod_api::LevelDimension::End => "minecraft:the_end",
                _ => "minecraft:overworld",
            };
            let networks = kinetic::networks();
            preferred_shaft(|side| {
                let n = side.normal();
                networks.kind_at(dimension, [pos[0] + n[0], pos[1] + n[1], pos[2] + n[2]])
            })
            .map(Dir::axis)
        },
    );
    level.state_with_property(event.state, "axis", axis.name()).map(Some)
}

// --- the client: what PlacementHelpers keeps between frames --------------------------------------------------

/// `PlacementHelpers`' statics: the ticks the indicator has been up, the helper's target, the last target (the arrow
/// keeps pointing at it while it fades), and the arrow's angle.
struct Shown {
    animation_tick: i32,
    target: Option<(Held, Offset)>,
    last_target: Option<Pos>,
    angle: f32,
}

static SHOWN: Mutex<Shown> = Mutex::new(Shown { animation_tick: 0, target: None, last_target: None, angle: 0.0 });

/// `PlacementHelpers.tick`: once a client tick, the helper the crosshair and the hands pick (if any) and the counter.
pub(crate) fn client_tick(host: &dyn mod_api::ClientHost) {
    let target = find_target(host);
    let mut shown = lock(&SHOWN);
    shown.animation_tick = rules::animation_step(shown.animation_tick, target.is_some());
    if let Some((_, offset)) = &target {
        shown.last_target = Some(offset.pos);
    }
    shown.target = target;
}

/// `PlacementHelpers.checkHelpers`: nothing while sneaking; for the main hand, then the off hand, the first helper
/// that finds a spot for the block the crosshair is on.
fn find_target(host: &dyn mod_api::ClientHost) -> Option<(Held, Offset)> {
    let mod_api::CrosshairTarget::Block { pos, hit, .. } = host.crosshair_target()? else { return None };
    let entity = host.camera_entity()?;
    if entity.is_shift_key_down(host).unwrap_or(false) {
        return None;
    }
    let living = host.living(entity)?;
    let reader = Reader::new(host)?;
    let click = Click {
        pos: [pos.x, pos.y, pos.z],
        face: dir_of(hit.face)?,
        hit: hit.point,
        sneaking: false,
        extendo: extended(host, Some(entity)),
    };
    for slot in [mod_api::EquipmentSlot::MainHand, mod_api::EquipmentSlot::OffHand] {
        let Some(stack) = living.equipment(host, slot) else { continue };
        let Some(held) = held_of(host, &stack) else { continue };
        if let Some(offset) = rules::ghost(held, &click, &reader) {
            return Some((held, offset));
        }
    }
    None
}

// --- the ghost ---------------------------------------------------------------------------------------------

/// The corners of a box, `[x0, y0, z0, x1, y1, z1]`, as `(x, y, z)` bits.
fn corner(b: &[f64; 6], i: usize) -> [f64; 3] {
    [b[3 * (i & 1)], b[1 + 3 * ((i >> 1) & 1)], b[2 + 3 * ((i >> 2) & 1)]]
}

/// A box's six faces, each a ring of four corners.
const FACES: [[usize; 4]; 6] =
    [[0, 2, 6, 4], [1, 3, 7, 5], [0, 1, 5, 4], [2, 3, 7, 6], [0, 1, 3, 2], [4, 5, 7, 6]];
/// And its twelve edges.
const EDGES: [[usize; 2]; 12] =
    [[0, 1], [2, 3], [4, 5], [6, 7], [0, 2], [1, 3], [4, 6], [5, 7], [0, 4], [1, 5], [2, 6], [3, 7]];

/// `ClientHost`-side `submit_custom_geometry`: the ghost of the block a helper would place, drawn from the target
/// the last tick found.
pub(crate) fn submit_custom_geometry(_: &dyn mod_api::ClientHost, event: &mut mod_api::SubmitCustomGeometryEvent<'_>) {
    let (held, offset, animation) = {
        let shown = lock(&SHOWN);
        let Some((held, offset)) = shown.target else { return };
        (held, offset, shown.animation_tick)
    };
    // `GhostBlocks.getBreathingAlpha` (from the clock, in milliseconds) `* .75f * PlacementHelpers.getCurrentAlpha()`.
    let millis = (event.view.game_time as f64 + f64::from(event.view.partial_tick)) * 50.0;
    let alpha = rules::breathing_alpha(millis) * rules::GHOST_ALPHA * rules::current_alpha(animation);
    let argb = |a: f32| (((a.clamp(0.0, 1.0) * 255.0) as u32) << 24) | 0x00E8_E8E8;
    let (fill, line) = (argb(alpha), argb(alpha * 1.8));
    let at = offset.pos.map(f64::from);
    let (mut quads, mut lines) = (Vec::new(), Vec::new());
    for local in rules::ghost_boxes(held, offset.axis) {
        // `ms.translate(.5); ms.scale(.85); ms.translate(-.5)`, then to the block's place in the world.
        let scaled = [0, 1, 2].map(|i| (0.5 + (local[i] - 0.5) * rules::GHOST_SCALE, 0.5 + (local[i + 3] - 0.5) * rules::GHOST_SCALE));
        let b = [scaled[0].0, scaled[1].0, scaled[2].0, scaled[0].1, scaled[1].1, scaled[2].1];
        let world = |i: usize| {
            let c = corner(&b, i);
            [at[0] + c[0], at[1] + c[1], at[2] + c[2]]
        };
        for face in FACES {
            quads.push(mod_api::WorldQuad::new(face.map(world), fill, mod_api::WorldDepth::Tested));
        }
        for [from, to] in EDGES {
            lines.push(mod_api::WorldLine::new(world(from), world(to), line, 2.0, mod_api::WorldDepth::Tested));
        }
    }
    let _ = event.draw.quads(&quads);
    let _ = event.draw.lines(&lines);
}

// --- the arrow ---------------------------------------------------------------------------------------------

/// How many nested copies of the fan make the fade from the apex to the rim.
const FADE_LAYERS: u32 = 10;

/// `PlacementHelpers.afterRenderOverlayLayer` and `fadedArrow`: after the crosshair, a triangle pointing the way
/// the ghost is, `a = progress * progress` at its tip and 0 at its base.
pub(crate) fn render_gui_post(host: &dyn mod_api::ClientHost, event: &mut mod_api::RenderGuiPostEvent<'_>) {
    let (animation, last) = {
        let shown = lock(&SHOWN);
        let Some(last) = shown.last_target else { return };
        (shown.animation_tick, last)
    };
    if animation <= 0 {
        return;
    }
    let Some(camera) = host.camera() else { return };
    // The target's centre in the camera's own frame: right, up and forward (`yaw` 0 looks south).
    let eye = [camera.position[0], camera.position[1] + f64::from(camera.eye_height), camera.position[2]];
    let d = [
        f64::from(last[0]) + 0.5 - eye[0],
        f64::from(last[1]) + 0.5 - eye[1],
        f64::from(last[2]) + 0.5 - eye[2],
    ];
    let (yaw, pitch) = (f64::from(camera.yaw).to_radians(), f64::from(camera.pitch).to_radians());
    let right = [-yaw.cos(), 0.0, -yaw.sin()];
    let forward = [-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos()];
    let up = [-yaw.sin() * pitch.sin(), pitch.cos(), yaw.cos() * pitch.sin()];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let wanted = rules::target_angle(dot(d, right), dot(d, up), dot(d, forward) < 0.0);
    let angle = {
        let mut shown = lock(&SHOWN);
        // `if (animationTick < 10) angle.setValue(targetAngle);` then `chase(.25f, EXP)` and `tickChaser()`.
        if shown.animation_tick < 10 {
            shown.angle = wanted;
        }
        shown.angle = rules::chase_angle(shown.angle, wanted);
        shown.angle
    };
    let progress = rules::current_alpha(animation);
    let a = progress * progress;
    let [width, height] = event.draw.size();
    let (centre, (sin, cos)) = ([width / 2.0, height / 2.0], angle.to_radians().sin_cos());
    // `r = g = b = .8f`; `ms.mulPose(Axis.ZP.rotationDegrees(angle))` turns clockwise on the screen.
    let layer_alpha = a / FADE_LAYERS as f32;
    let colour = ((((layer_alpha * 255.0).round() as u32).min(255)) << 24) | 0x00CC_CCCC;
    let apex = rules::ARROW_APEX;
    for layer in 1..=FADE_LAYERS {
        // Copies shrunk towards the apex: the tip is covered by all of them, the rim by the largest alone.
        let scale = layer as f32 / FADE_LAYERS as f32;
        let place = |p: [f32; 2]| {
            let (x, y) = (apex[0] + (p[0] - apex[0]) * scale, apex[1] + (p[1] - apex[1]) * scale);
            [centre[0] + x * cos - y * sin, centre[1] + x * sin + y * cos]
        };
        for pair in rules::ARROW_RIM.windows(2) {
            // One triangle of the fan, as a quad with a repeated corner.
            let (tip, from, to) = (place(apex), place(pair[0]), place(pair[1]));
            event.draw.quad([tip, from, to, to], colour);
        }
    }
}
