//! Drawing a train's carriages (`CarriageContraptionEntityRenderer`, `ContraptionEntityRenderer`,
//! `OrientedContraptionEntity.applyLocalTransforms`, `StandardBogeyRenderer`).
//!
//! # Why a block entity in an invisible block
//!
//! Horizon gives a mod no entity it can draw (a general entity type, `declare_general_entity`, has a record, a payload
//! and a tick, and no renderer), so a carriage is drawn the way every moving structure of this port is: by a block
//! entity's draw hook. A train runs anywhere on the graph, so the block entity can't stay where it was put: each
//! carriage has an *anchor*, a `create:carriage_anchor` block (no shape, no item, an empty model) that `trains.rs`
//! keeps in an air cell within a few blocks of the carriage and moves when the carriage gets too far from it. The
//! anchor's block entity holds the carriage's structure (Create's `Contraption` layout, as the pistons keep theirs)
//! and its pose; the draw hook turns and moves what it draws by the pose each frame (`translate` and `rotate_about`,
//! no `keep()`, as a moving piston is drawn), so the anchor block itself only changes every few blocks of travel.
//!
//! The server sends the pose as news (`Motion`: the position, the velocity, the yaw and pitch and how they are
//! changing, at a game time) every other tick while the carriage moves and once when it stops; the client runs the
//! pose on from the news (`Motion::at`, at most six ticks) so the carriage moves smoothly between updates. The update
//! tag is the block entity's whole data, structure included (a mod's hook can't tell a chunk being sent from a block
//! change, and `set_client_update_fields`' one-shot fields only ride on top of that tag), so the anchor's data is
//! rewritten and sent only when the carriage stops, when its bogeys change and every 40 ticks while it moves. The
//! news in between goes alone as a message (`POSE_CHANNEL`, `PoseNews`: about 100 bytes) to the players within 96
//! blocks, from each player's own tick (`server_players` can be empty in single player); the client draws from
//! whichever is newer, the message or the block entity's data.
//!
//! The wheels turn with the distance moved (`CarriageBogey.updateAngles`): the server sends each bogey's angle, how it
//! turns each tick, and the bogey's own yaw and pitch; the bogeys are drawn from their wheel angles with the pieces of
//! `StandardBogeyRenderer` (two shafts, the frame and the wheels; the large one with its drive, belt, piston and pin).
//! The bogey blocks themselves are in the structure and are drawn with it, as Create does.
//!
//! Differences from Create: the light is the anchor's, the bogeys' belt texture isn't scrolled (a `shiftUVScrolling`),
//! no coupling between carriages is drawn (`CarriageCouplingRenderer`), no sounds or smoke, and a carriage further
//! than a block entity is drawn (about 64 blocks) from the viewer isn't drawn.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};

use create_kinetics::rules::trains::carriage_pose::{self as pose, BogeyNews, BogeyPart, CENTRE, Motion, Op, CarriagePose, PoseNews};
use create_kinetics::rules::contraption::State;
use create_kinetics::rules::ponder_bogey;
use create_kinetics::rules::trains::bogey;
use create_kinetics::Pos;
use crate::compat::{
    BlockEntityDraw, BlockEntityEvent, BlockEntityRenderBoundingBoxEvent, BlockEntitySubmitEvent, BlockEntityTickEvent,
    BlockEntityTicksEvent, BlockEntityUpdateTagEvent, Compound, Hooks, Host, ModBlockEntity, NetFromServerEvent,
    PlayerTickEvent, StandaloneModelsEvent, Tag,
};

use super::contraption as engine;
use super::trains;
use crate::util;

pub const ANCHOR: &str = "create:carriage_anchor";
/// How far (blocks) a carriage may get from its anchor block before the anchor is moved.
pub const ANCHOR_RADIUS: f64 = 6.0;

// The anchor's saved data: Create's `Contraption` layout for the structure, the rest is ours.
pub const TRAIN: &str = "TrainId";
pub const CARRIAGE: &str = "CarriageId";
pub const POSITION: &str = "Pos";
pub const VELOCITY: &str = "Vel";
pub const YAW: &str = "Yaw";
pub const YAW_RATE: &str = "YawRate";
pub const PITCH: &str = "Pitch";
pub const PITCH_RATE: &str = "PitchRate";
pub const TIME: &str = "Time";
pub const INITIAL_YAW: &str = "InitialYaw";
pub const BOGEYS: &str = "Bogeys";
pub const DERAILED: &str = "Derailed";
/// The local positions of the burners that wear the Train Hat (`BlazeBurnerMovementBehaviour.shouldRenderHat`).
pub const HATS: &str = "Hats";

struct Draw;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Draw)
}

// --- the saved form -------------------------------------------------------------------------------------------------

fn doubles(v: [f64; 3]) -> Tag {
    Tag::List(v.iter().map(|d| Tag::Double(d.to_bits())).collect())
}

fn doubles_of(nbt: &Compound, key: &str) -> Option<[f64; 3]> {
    match util::get(nbt, key) {
        Some(Tag::List(list)) if list.len() == 3 => {
            let at = |i: usize| match &list[i] {
                Tag::Double(bits) => Some(f64::from_bits(*bits)),
                _ => None,
            };
            Some([at(0)?, at(1)?, at(2)?])
        }
        _ => None,
    }
}

/// What the anchor's data says of a bogey.
#[derive(Clone, Debug, PartialEq)]
pub struct BogeyData {
    pub large: bool,
    pub upside_down: bool,
    pub leading: bool,
    pub yaw: f32,
    pub pitch: f32,
    pub wheel: f64,
    pub wheel_rate: f64,
}

/// The pose news and the bogeys, written by `trains.rs` into the anchor's data.
pub fn write_pose(nbt: &mut Compound, motion: &Motion, initial_yaw: f32, spacing: i32, bogeys: &[BogeyData], derailed: bool) {
    nbt.insert(POSITION.into(), doubles(motion.position));
    nbt.insert(VELOCITY.into(), doubles(motion.velocity));
    util::set_float(nbt, YAW, motion.yaw);
    util::set_float(nbt, YAW_RATE, motion.yaw_rate);
    util::set_float(nbt, PITCH, motion.pitch);
    util::set_float(nbt, PITCH_RATE, motion.pitch_rate);
    nbt.insert(TIME.into(), Tag::Long(motion.time));
    util::set_float(nbt, INITIAL_YAW, initial_yaw);
    nbt.insert("Spacing".into(), Tag::Int(spacing));
    nbt.insert(DERAILED.into(), Tag::Byte(i8::from(derailed)));
    let list = bogeys
        .iter()
        .map(|b| {
            let mut c = Compound::new();
            c.insert("Large".into(), Tag::Byte(i8::from(b.large)));
            c.insert("UpsideDown".into(), Tag::Byte(i8::from(b.upside_down)));
            c.insert("Leading".into(), Tag::Byte(i8::from(b.leading)));
            util::set_float(&mut c, "Yaw", b.yaw);
            util::set_float(&mut c, "Pitch", b.pitch);
            c.insert("Wheel".into(), Tag::Double(b.wheel.to_bits()));
            c.insert("WheelRate".into(), Tag::Double(b.wheel_rate.to_bits()));
            Tag::Compound(c)
        })
        .collect();
    nbt.insert(BOGEYS.into(), Tag::List(list));
}

/// Saves which burners wear the hat; the pose's own write leaves it alone.
pub fn write_hats(nbt: &mut Compound, hats: &[Pos]) {
    let list = hats.iter().map(|p| Tag::IntArray(p.to_vec())).collect();
    nbt.insert(HATS.into(), Tag::List(list));
}

fn hats_of(nbt: &Compound) -> Vec<Pos> {
    match util::get(nbt, HATS) {
        Some(Tag::List(list)) => list
            .iter()
            .filter_map(|t| match t {
                Tag::IntArray(v) if v.len() == 3 => Some([v[0], v[1], v[2]]),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn motion_of(nbt: &Compound) -> Option<Motion> {
    Some(Motion {
        position: doubles_of(nbt, POSITION)?,
        velocity: doubles_of(nbt, VELOCITY)?,
        yaw: util::float(nbt, YAW).unwrap_or(0.0),
        yaw_rate: util::float(nbt, YAW_RATE).unwrap_or(0.0),
        pitch: util::float(nbt, PITCH).unwrap_or(0.0),
        pitch_rate: util::float(nbt, PITCH_RATE).unwrap_or(0.0),
        time: util::long(nbt, TIME),
    })
}

fn bogeys_of(nbt: &Compound) -> Vec<BogeyData> {
    let Some(Tag::List(list)) = util::get(nbt, BOGEYS) else { return Vec::new() };
    list.iter()
        .filter_map(|t| {
            let Tag::Compound(c) = t else { return None };
            let double = |key: &str| match util::get(c, key) {
                Some(Tag::Double(bits)) => f64::from_bits(*bits),
                _ => 0.0,
            };
            Some(BogeyData {
                large: util::boolean(c, "Large"),
                upside_down: util::boolean(c, "UpsideDown"),
                leading: util::boolean(c, "Leading"),
                yaw: util::float(c, "Yaw").unwrap_or(0.0),
                pitch: util::float(c, "Pitch").unwrap_or(0.0),
                wheel: double("Wheel"),
                wheel_rate: double("WheelRate"),
            })
        })
        .collect()
}

// --- drawing ---------------------------------------------------------------------------------------------------------

fn f32s(v: [f64; 3]) -> [f32; 3] {
    [v[0] as f32, v[1] as f32, v[2] as f32]
}

/// Applies Java's `PoseStack` calls in order: the first is the outermost.
fn apply(draw: &mut BlockEntityDraw, ops: &[Op], pivot: [f32; 3]) {
    for op in ops {
        match *op {
            Op::Translate(by) => draw.translate(f32s(by)),
            Op::RotateY(a) => draw.rotate_about([0.0, 1.0, 0.0], a, pivot),
            Op::RotateX(a) => draw.rotate_about([1.0, 0.0, 0.0], a, pivot),
            Op::RotateZ(a) => draw.rotate_about([0.0, 0.0, 1.0], a, pivot),
        }
    }
}

fn draw_structure(host: &dyn Host, draw: &mut BlockEntityDraw, parsed: &engine::Parsed, hats: &[Pos]) {
    for block in &parsed.moving.blocks {
        draw.push();
        draw.translate([block.local[0] as f32, block.local[1] as f32, block.local[2] as f32]);
        // `ControlsBlock.updateShape`: the block is `open` in a contraption.
        if block.state.is(create_kinetics::rules::trains::carriage_contraption::CONTROLS) {
            let mut open: State = block.state.clone();
            open.set("open", "true");
            engine::draw_state(host, draw, &open);
        } else {
            engine::draw_state(host, draw, &block.state);
        }
        // The burner's head, in a hat when it conducts a train that has a schedule (`Hats`, written by `trains.rs`).
        super::burner::draw_in_structure(host, draw, &block.state, block.local, hats.contains(&block.local));
        draw.pop();
    }
}

/// `StandardBogeyRenderer`: the parts under the bogey's own pose.
fn draw_bogey_parts(host: &dyn Host, draw: &mut BlockEntityDraw, large: bool, wheel_angle: f32) {
    for part in pose::standard_bogey(large, wheel_angle) {
        draw.push();
        match part {
            BogeyPart::Shaft { axis, at, angle, .. } => {
                draw.translate(f32s(at));
                // `center().rotateZDegrees(angle).uncenter()` (or X for the secondary shaft).
                let about = if axis == 'x' { [1.0, 0.0, 0.0] } else { [0.0, 0.0, 1.0] };
                draw.rotate_about(about, angle, CENTRE.map(|v| v as f32));
                let axis_name = if axis == 'x' { "x" } else { "z" };
                host.draw_block(draw, "create:shaft", &[("axis", axis_name)]);
            }
            BogeyPart::Model { model, at, angle_x, then, unturn, scale, .. } => {
                draw.translate(f32s(at));
                draw.rotate_about([1.0, 0.0, 0.0], angle_x, [0.0; 3]);
                draw.translate(f32s(then));
                if unturn != 0.0 {
                    draw.rotate_about([1.0, 0.0, 0.0], unturn, [0.0; 3]);
                }
                if scale != 1.0 {
                    draw.scale([scale, scale, scale]);
                }
                host.draw_model(draw, model, 0xFF_FFFF);
            }
        }
        draw.pop();
    }
}

/// A bogey block in a Ponder scene (`BogeyBlockEntityRenderer`): the frame and wheels under the block's centre, turned when the
/// block lies along X, the wheels at the angle `animateBogey` has brought them to (`rules::ponder_bogey`). Only a scene's block
/// entity (the one with the scene's tick) draws it: a bogey in the world has no block entity and is drawn by its carriage.
fn submit_ponder_bogey(host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
    let Some(size) = ponder_bogey::size_of(event.block) else { return };
    if crate::float(&event.entity.nbt, crate::ponder::TICK).is_none() {
        return;
    }
    let distance = crate::float(&event.entity.nbt, crate::ponder::ANIMATED).unwrap_or(0.0);
    let along_x = host.block_state_property(event.state, "axis").as_deref() == Some("x");
    let draw = &mut *event.draw;
    draw.push();
    draw.translate([0.5, 0.5, 0.5]);
    if along_x {
        draw.rotate_about([0.0, 1.0, 0.0], 90.0, [0.0; 3]);
    }
    draw_bogey_parts(host, draw, size.id == bogey::LARGE.id, ponder_bogey::wheel_angle(distance, size));
    draw.pop();
}

// --- the pose news between data updates ------------------------------------------------------------------------------

/// The channel the pose news goes on.
pub const POSE_CHANNEL: &str = "create:carriage_pose";
/// How far (blocks) from an anchor a player is sent its news: a block entity is drawn within 64 blocks.
const NEWS_RANGE: f64 = 96.0;
/// News older than this (ticks) is dropped.
const NEWS_LIFE: i64 = 200;

impl From<&BogeyData> for BogeyNews {
    fn from(b: &BogeyData) -> Self {
        BogeyNews {
            large: b.large,
            upside_down: b.upside_down,
            leading: b.leading,
            yaw: b.yaw,
            pitch: b.pitch,
            wheel: b.wheel,
            wheel_rate: b.wheel_rate,
        }
    }
}

impl From<&BogeyNews> for BogeyData {
    fn from(b: &BogeyNews) -> Self {
        BogeyData {
            large: b.large,
            upside_down: b.upside_down,
            leading: b.leading,
            yaw: b.yaw,
            pitch: b.pitch,
            wheel: b.wheel,
            wheel_rate: b.wheel_rate,
        }
    }
}

/// The server's latest news of each anchor: its dimension, a number that grows with each news, the encoded news.
struct Published {
    dimension: String,
    seq: u64,
    position: [f64; 3],
    time: i64,
    bytes: Vec<u8>,
}

static PUBLISHED: LazyLock<Mutex<HashMap<Pos, Published>>> = LazyLock::new(Mutex::default);
/// What each player has been sent: anchor to news number.
static SENT: LazyLock<Mutex<HashMap<u128, HashMap<Pos, u64>>>> = LazyLock::new(Mutex::default);
static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
/// The client's latest news of each anchor.
static RECEIVED: LazyLock<Mutex<HashMap<Pos, PoseNews>>> = LazyLock::new(Mutex::default);

fn lock<T>(m: &'static LazyLock<Mutex<T>>) -> MutexGuard<'static, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The server: a carriage's new pose, for the players near its anchor (sent from their own ticks).
pub fn publish(dimension: &str, anchor: Pos, motion: &Motion, bogeys: &[BogeyData], derailed: bool) {
    let news = PoseNews { anchor, motion: *motion, derailed, bogeys: bogeys.iter().map(BogeyNews::from).collect() };
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let entry = Published { dimension: dimension.to_owned(), seq, position: motion.position, time: motion.time, bytes: news.encode() };
    lock(&PUBLISHED).insert(anchor, entry);
}

/// The server: an anchor is gone.
pub fn forget(_dimension: &str, anchor: Pos) {
    lock(&PUBLISHED).remove(&anchor);
}

/// The server, in a player's tick: the news near them they haven't had.
fn send_news(host: &mut dyn Host) {
    let Some(player) = host.acting_player() else { return };
    let dimension = host.dimension();
    let now = util::game_time(&*host);
    let mut out = Vec::new();
    {
        let mut published = lock(&PUBLISHED);
        published.retain(|_, p| now - p.time <= NEWS_LIFE);
        if published.is_empty() {
            return;
        }
        let mut sent = lock(&SENT);
        let seen = sent.entry(player.uuid).or_default();
        seen.retain(|anchor, _| published.contains_key(anchor));
        for (anchor, p) in published.iter() {
            if p.dimension != dimension || seen.get(anchor).is_some_and(|s| *s >= p.seq) {
                continue;
            }
            let d = [0, 1, 2].map(|i| p.position[i] - player.position[i]);
            if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] > NEWS_RANGE * NEWS_RANGE {
                continue;
            }
            seen.insert(*anchor, p.seq);
            out.push(p.bytes.clone());
        }
    }
    for bytes in out {
        let _ = host.net_send_to_player(player.uuid, POSE_CHANNEL, &bytes);
    }
}

/// The client: the motion and bogeys to draw an anchor from, the news if it is newer than the block entity's data.
fn latest(pos: Pos, nbt: &Compound) -> Option<(Motion, Vec<BogeyData>)> {
    let saved = motion_of(nbt);
    let received = lock(&RECEIVED);
    match (saved, received.get(&pos)) {
        (Some(saved), Some(news)) if news.motion.time > saved.time => {
            Some((news.motion, news.bogeys.iter().map(BogeyData::from).collect()))
        }
        (None, Some(news)) => Some((news.motion, news.bogeys.iter().map(BogeyData::from).collect())),
        (Some(saved), _) => Some((saved, bogeys_of(nbt))),
        (None, None) => None,
    }
}

fn submit(host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
    let nbt = &event.entity.nbt;
    let Some(parsed) = engine::parsed(event.pos, event.entity) else { return };
    let Some((motion, bogeys)) = latest(event.pos, nbt) else { return };
    let draw = &mut *event.draw;
    let now = draw.game_time as f64 + f64::from(draw.partial);
    let at: CarriagePose = motion.at(now);
    let dt = (now - motion.time as f64).clamp(0.0, pose::LOOKAHEAD);
    let origin = [0, 1, 2].map(|i| at.position[i] - f64::from(event.pos[i]));
    let initial_yaw = util::float(nbt, INITIAL_YAW).unwrap_or(0.0);
    let spacing = util::int(nbt, "Spacing");

    draw.push();
    draw.translate(f32s(origin));
    // The structure, turned about its first block's middle (`applyLocalTransforms`).
    draw.push();
    apply(draw, &pose::carriage_ops(at.yaw, at.pitch, initial_yaw), CENTRE.map(|v| v as f32));
    draw_structure(host, draw, &parsed, &hats_of(nbt));
    draw.pop();
    // The bogeys (`translateBogey`) from the entity's position.
    let leading_upside_down = bogeys.iter().find(|b| b.leading).is_some_and(|b| b.upside_down);
    for b in &bogeys {
        draw.push();
        let ops = pose::bogey_ops(
            b.upside_down,
            leading_upside_down,
            b.leading,
            spacing,
            pose::view_y_rot(at.yaw),
            at.pitch,
            b.yaw,
            b.pitch,
        );
        apply(draw, &ops, [0.0; 3]);
        let wheel = ((b.wheel + b.wheel_rate * dt) % 360.0) as f32;
        draw_bogey_parts(host, draw, b.large, wheel);
        draw.pop();
    }
    draw.pop();
}

impl Hooks for Draw {
    /// The server: a player's tick sends them the carriage news near them.
    fn player_tick(&self, host: &mut dyn Host, _: &mut PlayerTickEvent) {
        send_news(host);
    }

    /// The client: a carriage's pose news, kept for its anchor's draw.
    fn net_from_server(&self, _: &mut dyn Host, event: &mut NetFromServerEvent<'_>) {
        if event.channel != POSE_CHANNEL {
            return;
        }
        let Some(news) = PoseNews::decode(event.data) else { return };
        let mut received = lock(&RECEIVED);
        if received.len() > 1024 {
            let newest = news.motion.time;
            received.retain(|_, old| newest - old.motion.time <= NEWS_LIFE);
        }
        if received.get(&news.anchor).is_none_or(|old| old.motion.time < news.motion.time) {
            received.insert(news.anchor, news);
        }
    }

    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        for model in [pose::FRAME, pose::SMALL_WHEELS, pose::PIN, pose::PISTON, pose::DRIVE, pose::LARGE_WHEELS, pose::BELT] {
            event.models.push(model.to_owned());
        }
    }

    fn block_entity(&self, _: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        (event.block == ANCHOR).then(|| (ANCHOR.to_owned(), Compound::new()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ANCHOR {
            event.ticks = true;
        }
    }

    /// An anchor whose carriage the server doesn't know (a train lost to a world that didn't keep it) goes.
    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ANCHOR {
            return;
        }
        trains::anchor_tick(host, event);
    }

    /// The whole data: the structure (the form a client reads, with the shapes the server knows) and the pose.
    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        if event.entity.id != ANCHOR {
            return None;
        }
        let mut tag = event.entity.nbt.clone();
        tag.insert(engine::CONTRAPTION.into(), engine::client_contraption(event.entity));
        Some(tag)
    }

    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ANCHOR {
            submit_ponder_bogey(host, event);
            return;
        }
        submit(host, event);
    }

    /// The anchor may be [`ANCHOR_RADIUS`] blocks from the carriage and the carriage reaches its extent further.
    fn block_entity_render_bounding_box(&self, _: &dyn Host, event: &mut BlockEntityRenderBoundingBoxEvent<'_>) {
        if event.block != ANCHOR {
            return;
        }
        let reach = engine::parsed(event.pos, event.entity)
            .map_or(8, |p| p.extent.iter().map(|e| e.abs()).max().unwrap_or(0) + 4) as f32
            + ANCHOR_RADIUS as f32
            + 2.0;
        for i in 0..3 {
            event.bounds[i] = event.bounds[i].min(-reach);
            event.bounds[i + 3] = event.bounds[i + 3].max(1.0 + reach);
        }
    }
}

/// Where the carriage is at game time `t` and a tick before, for the solid module (`OrientedContraptionEntity`).
pub fn solid_sample(entity: &ModBlockEntity, t: f64) -> Option<crate::solid::Sample> {
    if entity.id != ANCHOR {
        return None;
    }
    let motion = motion_of(&entity.nbt)?;
    let initial_yaw = util::float(&entity.nbt, INITIAL_YAW).unwrap_or(0.0);
    let (now, prev) = (motion.at(t), motion.at(t - 1.0));
    Some(crate::solid::Sample::Oriented {
        now: now.position,
        prev: prev.position,
        initial_yaw,
        yaw: now.yaw,
        prev_yaw: prev.yaw,
        pitch: now.pitch,
        prev_pitch: prev.pitch,
    })
}

/// The cell a block of `pos` is in, as a `Pos`.
pub fn cell(position: [f64; 3]) -> Pos {
    [position[0].floor() as i32, position[1].floor() as i32, position[2].floor() as i32]
}
