//! The original Create catalog and compiled scene instructions.

use std::collections::{BTreeMap, BTreeSet};

use crate::compat::{
    BlockStateId, Host, HudBatch, ModBlockEntity, NativeSceneBlock, NativeSceneView,
    serde_json::{self, Value},
};

use create_kinetics::Dir;
use create_kinetics::belt::{Ending, Part as BeltPart, Slope as BeltSlope};
use create_kinetics::rules::ponder_belt::{self as belt_items, Action as BeltAction, Segment as BeltSegment};
use create_kinetics::rules::arm::Phase as ArmPhase;
use create_kinetics::rules::arm_points::{Mode as ArmMode, Point as ArmPoint, PointType as ArmPointType};
use create_kinetics::rules::ponder_anim::{self as anim, Instruction};
use create_kinetics::rules::ponder_arm::{Arm, Instruction as ArmInstruction};
use create_kinetics::rules::ponder_particles::{
    Kind as ParticleKind, Particle, Rng, Spawn, emission_runs, glue_burst, particle, within_block,
};

use super::scene::Pos;
use super::screen::Showing;

/// The longest an item or block break particle lives: `(int)(4 / 0.1)` ticks.
const GLUE_LIFE: f32 = 40.0;
/// The longest any emitted particle lives (the large smoke: `(int)(8 / 0.2) * 2.5`).
const EMITTED_LIFE: f32 = 200.0;

const ASSETS: &str = "assets/create/ponder/";

/// Decoding from `serde_json::Value` by hand: the mod's own `serde` is a different crate instance from the one the
/// game's `Value` implements `Deserialize` with, so `#[derive(Deserialize)]` and `from_value` cannot be used here.
trait FromJson: Sized {
    fn from_json(value: &Value) -> Option<Self>;
}

fn read<T: FromJson>(row: &[Value], index: usize) -> Option<T> {
    T::from_json(row.get(index)?)
}

fn field<T: FromJson>(value: &Value, name: &str) -> Option<T> {
    T::from_json(value.get(name)?)
}

fn field_or<T: FromJson + Default>(value: &Value, name: &str) -> T {
    field(value, name).unwrap_or_default()
}

impl FromJson for u32 {
    fn from_json(value: &Value) -> Option<Self> {
        u32::try_from(value.as_u64()?).ok()
    }
}

impl FromJson for usize {
    fn from_json(value: &Value) -> Option<Self> {
        usize::try_from(value.as_u64()?).ok()
    }
}

impl FromJson for i32 {
    fn from_json(value: &Value) -> Option<Self> {
        i32::try_from(value.as_i64()?).ok()
    }
}

impl FromJson for f32 {
    fn from_json(value: &Value) -> Option<Self> {
        Some(value.as_f64()? as f32)
    }
}

impl FromJson for bool {
    fn from_json(value: &Value) -> Option<Self> {
        value.as_bool()
    }
}

impl FromJson for String {
    fn from_json(value: &Value) -> Option<Self> {
        Some(value.as_str()?.to_owned())
    }
}

impl<T: FromJson> FromJson for Option<T> {
    fn from_json(value: &Value) -> Option<Self> {
        Some(if value.is_null() { None } else { Some(T::from_json(value)?) })
    }
}

impl<T: FromJson> FromJson for Vec<T> {
    fn from_json(value: &Value) -> Option<Self> {
        value.as_array()?.iter().map(T::from_json).collect()
    }
}

impl<T: FromJson, const N: usize> FromJson for [T; N] {
    fn from_json(value: &Value) -> Option<Self> {
        let items = value.as_array()?;
        if items.len() != N {
            return None;
        }
        items.iter().map(T::from_json).collect::<Option<Vec<T>>>()?.try_into().ok()
    }
}

impl<A: FromJson, B: FromJson> FromJson for (A, B) {
    fn from_json(value: &Value) -> Option<Self> {
        let items = value.as_array()?;
        Some((A::from_json(items.first()?)?, B::from_json(items.get(1)?)?))
    }
}

impl<A: FromJson, B: FromJson, C: FromJson> FromJson for (A, B, C) {
    fn from_json(value: &Value) -> Option<Self> {
        let items = value.as_array()?;
        Some((A::from_json(items.first()?)?, B::from_json(items.get(1)?)?, C::from_json(items.get(2)?)?))
    }
}

impl FromJson for BTreeMap<String, String> {
    fn from_json(value: &Value) -> Option<Self> {
        value
            .as_object()?
            .iter()
            .map(|(key, value)| Some((key.clone(), value.as_str()?.to_owned())))
            .collect()
    }
}

pub struct Catalog {
    pub tags: Vec<Topic>,
    pub scenes: Vec<Entry>,
    ui: BTreeMap<String, String>,
}

pub struct Topic {
    pub id: String,
    pub icon: String,
    pub title: String,
    pub description: String,
    pub index: bool,
    pub items: Vec<String>,
    pub main_item: Option<String>,
}

pub struct Entry {
    pub title: String,
    pub file: String,
    pub items: Vec<String>,
    pub tags: Vec<String>,
}

impl FromJson for Catalog {
    fn from_json(value: &Value) -> Option<Self> {
        Some(Self {
            tags: field(value, "tags")?,
            scenes: field(value, "scenes")?,
            ui: field_or(value, "ui"),
        })
    }
}

impl FromJson for Topic {
    fn from_json(value: &Value) -> Option<Self> {
        Some(Self {
            id: field(value, "id")?,
            icon: field(value, "icon")?,
            title: field(value, "title")?,
            description: field(value, "description")?,
            index: field(value, "index")?,
            items: field(value, "items")?,
            main_item: field_or(value, "main_item"),
        })
    }
}

impl FromJson for Entry {
    fn from_json(value: &Value) -> Option<Self> {
        Some(Self {
            title: field(value, "title")?,
            file: field(value, "file")?,
            items: field(value, "items")?,
            tags: field(value, "tags")?,
        })
    }
}

impl Catalog {
    pub fn ui<'a>(&'a self, key: &str, fallback: &'a str) -> &'a str {
        self.ui
            .get(&format!("ponder.ui.{key}"))
            .map_or(fallback, String::as_str)
    }
    pub fn load(host: &dyn Host) -> Option<Self> {
        let bytes = host
            .client_resources()
            .read(&format!("{ASSETS}catalog.json"))?;
        Self::from_json(&serde_json::from_slice::<Value>(&bytes).ok()?)
    }

    pub fn for_item(&self, item: &str) -> Option<usize> {
        self.scenes
            .iter()
            .position(|scene| scene.items.iter().any(|id| id == item))
    }
}

type Properties = BTreeMap<String, String>;
type State = (String, Properties);

// Scene files use variable-length rows, which are decoded once on selection.
struct SceneFile {
    size: Pos,
    bounds: Pos,
    palette: Vec<State>,
    blocks: Vec<(Pos, usize)>,
    speeds: Vec<(Pos, f32)>,
    events: Vec<Vec<Value>>,
    models: Vec<(State, Vec<Model>)>,
    /// Each Mechanical Arm's interaction points: the arm, then (type, mode, position) of each.
    arms: Vec<(Pos, Vec<(String, String, Pos)>)>,
    duration: u32,
    scale: f32,
    offset: f32,
}

impl FromJson for SceneFile {
    fn from_json(value: &Value) -> Option<Self> {
        Some(Self {
            size: field(value, "size")?,
            bounds: field(value, "bounds")?,
            palette: field(value, "palette")?,
            blocks: field(value, "blocks")?,
            speeds: field_or(value, "speeds"),
            events: field(value, "events")?,
            models: field_or(value, "models"),
            arms: field_or(value, "arms"),
            duration: field(value, "duration")?,
            scale: field(value, "scale")?,
            offset: field(value, "offset")?,
        })
    }
}

impl FromJson for Value {
    fn from_json(value: &Value) -> Option<Self> {
        Some(value.clone())
    }
}

enum Event {
    Show(u32, Vec<Pos>, Option<[f32; 3]>),
    Hide(Vec<Pos>, Option<[f32; 3]>),
    HideSection(u32, Option<[f32; 3]>),
    Block(Vec<Pos>, usize),
    Restore(Vec<Pos>),
    Speed(Vec<Pos>, f32, f32),
    Move(u32, [f32; 3], f32),
    Rotate(u32, [f32; 3], f32),
    Pivot(u32, [f32; 3]),
    Caption(String, f32, CaptionStyle),
    Control([f32; 3], f32),
    Line([f32; 3], [f32; 3], u32, f32),
    Outline([f32; 3], [f32; 3], u32, f32),
    Item(String, [f32; 3], [f32; 3], f32),
    Toggle(Vec<Pos>),
    /// `PressingBehaviour.start(mode)`: a stroke of the press head, `Mode` as its id.
    Press(Vec<Pos>, i32),
    Yaw(f32),
    /// `rotateBearing`, `movePulley`, `moveDeployer`: `AnimateBlockEntityInstruction` (by `totalDelta` over `ticks`).
    Animate(Pos, f32, u32),
    /// `createItemOnBelt(pos, side, stack)`: the link, the item and the block.
    BeltPut(u32, String, Pos),
    /// `removeItemsFromBelt(pos)`.
    BeltRemove(Pos),
    /// `stallBeltItem(link, stalled)`.
    BeltStall(u32, bool),
    /// `changeBeltItemTo(link, stack)`.
    BeltChange(u32, String),
    /// `world.instructArm(pos, phase, heldItem, targetedPoint)`.
    Arm(Pos, ArmPhase, Option<String>, i32),
    /// `effects.superGlue(pos, side, fullBlock)`: a burst of glue on a block's face (`SuperGlueItem.spawnParticles`).
    Glue(Pos, [i32; 3], bool),
    /// `effects.emitParticles`: `kind` at a point, its motion, within the block's space or not, per tick, for ticks.
    Particles(ParticleKind, [f32; 3], [f32; 3], bool, f32, u32),
    Marker,
}

pub struct Scene {
    blocks: BTreeMap<Pos, usize>,
    speeds: BTreeMap<Pos, Rotation>,
    palette: Vec<ResolvedBlock>,
    events: Vec<(f32, Event)>,
    markers: Vec<f32>,
    proxy: Option<BlockStateId>,
    /// The points each Mechanical Arm reaches (`InteractionPoints`).
    arms: BTreeMap<Pos, Vec<ArmPoint>>,
    /// The scene's belt blocks, and the items put on belts and depots (`rules::ponder_belt`).
    belts: BTreeMap<Pos, BeltSegment>,
    belt_actions: Vec<(i64, BeltAction)>,
    belt_stacks: Vec<String>,
    /// Each block's speed over time: from the tick, in rpm.
    speed_log: BTreeMap<Pos, Vec<(f32, f32)>>,
    duration: f32,
    center: [f32; 3],
    dimension: f32,
    scale: f32,
    reading: Vec<(f32, f32)>,
}

#[derive(Default)]
pub struct CaptionStyle {
    pub target: Option<[f32; 3]>,
    pub near: bool,
    pub y: Option<f32>,
    pub color: Option<String>,
}

impl FromJson for CaptionStyle {
    fn from_json(value: &Value) -> Option<Self> {
        Some(Self {
            target: field_or(value, "target"),
            near: field_or(value, "near"),
            y: field_or(value, "y"),
            color: field_or(value, "color"),
        })
    }
}

struct ResolvedBlock {
    state: Option<BlockStateId>,
    toggled: Option<BlockStateId>,
    id: String,
    redraw: bool,
    models: Vec<Model>,
    geometry: String,
    properties: Vec<(String, String)>,
}

#[derive(Clone)]
struct Model(String, f32, f32);

impl FromJson for Model {
    fn from_json(value: &Value) -> Option<Self> {
        let (name, a, b) = <(String, f32, f32)>::from_json(value)?;
        Some(Self(name, a, b))
    }
}

#[derive(Clone, Copy, Default)]
struct Motion {
    from: [f32; 3],
    to: [f32; 3],
    start: f32,
    duration: f32,
}

impl Motion {
    fn at(self, tick: f32) -> [f32; 3] {
        let progress = if self.duration > 0.0 {
            ((tick - self.start) / self.duration).clamp(0.0, 1.0)
        } else {
            1.0
        };
        std::array::from_fn(|i| (self.to[i] - self.from[i]).mul_add(progress, self.from[i]))
    }

    fn add(&mut self, amount: [f32; 3], tick: f32, duration: f32) {
        let from = self.at(tick);
        let to = std::array::from_fn(|i| from[i] + amount[i]);
        *self = Self {
            from,
            to,
            start: tick,
            duration,
        };
    }
}

#[derive(Default)]
struct Section {
    blocks: BTreeSet<Pos>,
    offset: Motion,
    rotation: Motion,
    pivot: Option<[f32; 3]>,
    fades: BTreeMap<Pos, SectionFade>,
}

#[derive(Clone, Copy)]
struct SectionFade {
    start: f32,
    direction: [f32; 3],
    incoming: bool,
}

impl SectionFade {
    fn translation(self, tick: f32) -> Option<[f32; 3]> {
        let progress = ((tick - self.start) / 15.0).clamp(0.0, 1.0);
        if !self.incoming && progress >= 1.0 {
            return None;
        }
        let distance = if self.incoming {
            -0.5 * (1.0 - progress).powi(2)
        } else {
            0.5 * progress.powi(2)
        };
        Some(self.direction.map(|component| component * distance))
    }
}

impl Section {
    fn stage(&mut self, pos: Pos, tick: f32, direction: Option<[f32; 3]>, incoming: bool) {
        if incoming {
            self.blocks.insert(pos);
        }
        if let Some(direction) = direction {
            self.fades.insert(
                pos,
                SectionFade {
                    start: tick,
                    direction,
                    incoming,
                },
            );
        } else {
            self.fades.remove(&pos);
            if !incoming {
                self.blocks.remove(&pos);
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Rotation {
    rpm: f32,
    phase: f32,
    since: f32,
}

impl Rotation {
    fn angle(self, tick: f32) -> f32 {
        self.phase + self.rpm * 0.3 * (tick - self.since)
    }
}

#[derive(Default)]
pub struct Frame<'a> {
    pub caption: &'a str,
    pub caption_style: Option<&'a CaptionStyle>,
    yaw: f32,
    point: Option<[f32; 3]>,
    items: Vec<(&'a str, [f32; 3])>,
    /// The particles alive: where, an ARGB colour and the pixels across at 30 pixels a block.
    particles: Vec<([f32; 3], u32, f32)>,
    lines: Vec<([f32; 3], [f32; 3], u32)>,
    blocks: Vec<VisibleBlock>,
}

struct VisibleBlock {
    pos: Pos,
    state: usize,
    rotation: Rotation,
    toggled: bool,
    pose: Option<([f32; 3], [f32; 3])>,
    /// A bearing's angle, a pulley's offset or a deployer's hand offset a scene instruction moved
    /// (`rules::ponder_anim`).
    animated: Option<f32>,
    /// A Mechanical Arm a scene instructed: its four angles and the item in its claw.
    arm: Option<([f32; 4], Option<String>)>,
    /// A press mid-stroke: the progress of its cycle in ticks and `Mode`'s id.
    press: Option<(f32, i32)>,
}

impl Frame<'_> {
    /// Adds a particle `age` ticks old, fading over the last of its life.
    fn add_particle(&mut self, particle: &Particle, age: f32) {
        let Some(at) = particle.at(age) else {
            return;
        };
        let fade = (1.5 * (1.0 - age / particle.lifetime)).clamp(0.0, 1.0);
        let colour = particle.kind.colour();
        let alpha = ((colour >> 24) as f32 * fade) as u32;
        self.particles.push((at.map(|v| v as f32), (alpha << 24) | (colour & 0xFF_FFFF), particle.kind.size()));
    }
}

impl Scene {
    pub fn load(host: &dyn Host, entry: &Entry) -> Option<Self> {
        let bytes = host
            .client_resources()
            .read(&format!("{ASSETS}{}", entry.file))?;
        let file = SceneFile::from_json(&serde_json::from_slice::<Value>(&bytes).ok()?)?;
        Some(Self::prepare(host, file))
    }

    fn prepare(host: &dyn Host, mut file: SceneFile) -> Self {
        let mut events = Vec::new();
        let mut markers = vec![0.0];
        for row in file.events {
            if let Some((tick, event)) = decode(&row, &mut file.palette) {
                if matches!(event, Event::Marker) {
                    markers.push(tick);
                }
                events.push((tick, event));
            }
        }
        markers.sort_by(f32::total_cmp);
        markers.dedup();
        events.sort_by(|(a, _), (b, _)| a.total_cmp(b));
        let mut reading: Vec<(f32, f32)> = Vec::new();
        for (at, event) in &events {
            if let Event::Caption(_, duration, _) = event {
                let end = at + duration;
                if let Some((_, last)) = reading.last_mut()
                    && *at <= *last
                {
                    *last = last.max(end);
                } else {
                    reading.push((*at, end));
                }
            }
        }
        let [x, z, span] = file.bounds;
        let [width, height, depth] = file.size.map(|v| v as f32);
        let center = [
            x as f32 + span as f32 * 0.5,
            1.0 - file.offset,
            z as f32 + span as f32 * 0.5,
        ];
        let belts: BTreeMap<Pos, BeltSegment> = file
            .blocks
            .iter()
            .filter_map(|&(pos, index)| {
                let (id, props) = file.palette.get(index)?;
                (id == "create:belt").then_some(())?;
                Some((
                    pos,
                    BeltSegment {
                        facing: Dir::from_name(props.get("facing")?)?,
                        slope: BeltSlope::from_name(props.get("slope")?)?,
                        part: BeltPart::from_name(props.get("part")?)?,
                    },
                ))
            })
            .collect();
        let mut belt_stacks: Vec<String> = Vec::new();
        let mut stack_of = |name: &str| {
            belt_stacks.iter().position(|known| known == name).unwrap_or_else(|| {
                belt_stacks.push(name.to_owned());
                belt_stacks.len() - 1
            }) as u32
        };
        let belt_actions: Vec<(i64, BeltAction)> = events
            .iter()
            .filter_map(|(at, event)| {
                let action = match event {
                    Event::BeltPut(link, item, pos) => {
                        BeltAction::Put { link: *link, stack: stack_of(item), pos: *pos }
                    }
                    Event::BeltRemove(pos) => BeltAction::Remove { pos: *pos },
                    Event::BeltStall(link, stalled) => BeltAction::Stall { link: *link, stalled: *stalled },
                    Event::BeltChange(link, item) => BeltAction::Change { link: *link, stack: stack_of(item) },
                    _ => return None,
                };
                Some((*at as i64, action))
            })
            .collect();
        // `setKineticSpeed` and its relatives, as a log: what each block turned at, from when.
        let mut speed_log: BTreeMap<Pos, Vec<(f32, f32)>> =
            file.speeds.iter().map(|&(pos, rpm)| (pos, vec![(-1.0, rpm)])).collect();
        for (at, event) in &events {
            if let Event::Speed(positions, factor, base) = event {
                for &pos in positions {
                    let log = speed_log.entry(pos).or_default();
                    let rpm = log.last().map_or(0.0, |&(_, rpm)| rpm).mul_add(*factor, *base);
                    log.push((*at, rpm));
                }
            }
        }
        Self {
            events,
            markers,
            belts,
            belt_actions,
            belt_stacks,
            speed_log,
            palette: resolve(host, &file.palette, file.models.into_iter().collect()),
            blocks: file.blocks.into_iter().collect(),
            speeds: file
                .speeds
                .into_iter()
                .map(|(pos, rpm)| {
                    (
                        pos,
                        Rotation {
                            rpm,
                            ..Rotation::default()
                        },
                    )
                })
                .collect(),
            proxy: host.default_state(super::render::PROXY),
            arms: file
                .arms
                .into_iter()
                .map(|(arm, points)| {
                    let points = points
                        .into_iter()
                        .filter_map(|(ty, mode, pos)| {
                            Some(ArmPoint {
                                ty: ArmPointType::from_id(&ty)?,
                                pos,
                                mode: ArmMode::from_name(&mode),
                            })
                        })
                        .collect();
                    (arm, points)
                })
                .collect(),
            duration: file.duration.max(1) as f32,
            center,
            dimension: width.max(depth).max(height).max(span as f32).max(1.0),
            scale: file.scale.clamp(0.2, 2.0),
            reading,
        }
    }

    pub fn duration(&self) -> f32 {
        self.duration
    }

    pub fn markers(&self) -> &[f32] {
        &self.markers
    }

    pub fn frame(&self, tick: f32) -> Frame<'_> {
        let mut edited = BTreeMap::new();
        let mut rotations = self.speeds.clone();
        let mut toggled = BTreeSet::new();
        let mut presses = BTreeMap::<Pos, (f32, i32)>::new();
        let mut sections = BTreeMap::<u32, Section>::new();
        let mut animations = BTreeMap::<Pos, Vec<Instruction>>::new();
        let mut arms = BTreeMap::<Pos, (Vec<ArmInstruction>, Option<&str>)>::new();
        let mut has_reveals = false;
        let mut frame = Frame::default();
        for (at, event) in &self.events {
            has_reveals |= matches!(event, Event::Show(..));
            if *at > tick {
                continue;
            }
            match event {
                Event::Show(id, positions, direction) => {
                    let section = sections.entry(*id).or_default();
                    for &pos in positions {
                        section.stage(pos, *at, *direction, true);
                    }
                }
                Event::Hide(positions, direction) => {
                    if let Some(section) = sections.get_mut(&0) {
                        for &pos in positions {
                            section.stage(pos, *at, *direction, false);
                        }
                    }
                }
                Event::HideSection(id, direction) => {
                    if let Some(direction) = direction {
                        if let Some(section) = sections.get_mut(id) {
                            section.fades.extend(section.blocks.iter().map(|&pos| {
                                (
                                    pos,
                                    SectionFade {
                                        start: *at,
                                        direction: *direction,
                                        incoming: false,
                                    },
                                )
                            }));
                        }
                    } else {
                        sections.remove(id);
                    }
                }
                Event::Block(positions, id) => {
                    edited.extend(positions.iter().map(|pos| (*pos, *id)))
                }
                Event::Restore(positions) => edited.retain(|pos, _| !positions.contains(pos)),
                Event::Move(id, movement, duration) => sections
                    .entry(*id)
                    .or_default()
                    .offset
                    .add(*movement, *at, *duration),
                Event::Rotate(id, movement, duration) => sections
                    .entry(*id)
                    .or_default()
                    .rotation
                    .add(*movement, *at, *duration),
                Event::Pivot(id, pivot) => sections.entry(*id).or_default().pivot = Some(*pivot),
                Event::Speed(positions, factor, base) => {
                    for &pos in positions {
                        let rotation = rotations.entry(pos).or_default();
                        *rotation = Rotation {
                            phase: rotation.angle(*at),
                            since: *at,
                            rpm: rotation.rpm.mul_add(*factor, *base),
                        };
                    }
                }
                Event::Caption(caption, duration, style) => {
                    if tick <= at + duration {
                        frame.caption = caption;
                        frame.caption_style = Some(style);
                    }
                }
                Event::Control(point, duration) => {
                    if tick <= at + duration {
                        frame.point = Some(*point);
                    }
                }
                Event::Item(item, point, velocity, duration) => {
                    let live = tick - at;
                    if live <= *duration {
                        frame.items.push((
                            item,
                            std::array::from_fn(|i| velocity[i].mul_add(live, point[i])),
                        ));
                    }
                }
                Event::Line(from, to, color, duration) => {
                    if tick <= at + duration {
                        frame.lines.push((*from, *to, *color));
                    }
                }
                Event::Outline(min, max, color, duration) => {
                    if tick <= at + duration {
                        for i in 0..8 {
                            for axis in 0..3 {
                                let mask = 1 << axis;
                                if i & mask == 0 {
                                    let from = std::array::from_fn(|j| {
                                        if i & (1 << j) == 0 { min[j] } else { max[j] }
                                    });
                                    let mut to = from;
                                    to[axis] = max[axis];
                                    frame.lines.push((from, to, *color));
                                }
                            }
                        }
                    }
                }
                Event::Toggle(positions) => {
                    for &pos in positions {
                        if !toggled.insert(pos) {
                            toggled.remove(&pos);
                        }
                    }
                }
                Event::Press(positions, mode) => {
                    presses.extend(positions.iter().map(|&pos| (pos, (*at, *mode))));
                }
                Event::Yaw(yaw) => frame.yaw += yaw * ((tick - at) / 20.0).clamp(0.0, 1.0),
                Event::Animate(pos, delta, duration) => {
                    animations.entry(*pos).or_default().push(Instruction {
                        start: *at as u32,
                        delta: *delta,
                        duration: *duration,
                    });
                }
                Event::BeltPut(..) | Event::BeltRemove(_) | Event::BeltStall(..) | Event::BeltChange(..) => {}
                Event::Arm(pos, phase, held, index) => {
                    let speed = rotations.get(pos).map_or(0.0, |rotation| rotation.rpm);
                    let arm = arms.entry(*pos).or_default();
                    arm.0.push(ArmInstruction { at: *at as i64, phase: *phase, index: *index, speed });
                    arm.1 = held.as_deref();
                }
                Event::Glue(pos, normal, full) => {
                    // `SuperGlueItem.spawnParticles`, then each particle as the game's item particle lives.
                    let age = tick - at;
                    if age < GLUE_LIFE {
                        let mut rng = Rng::seeded(&[
                            *at as u64,
                            pos[0] as u64,
                            pos[1] as u64,
                            pos[2] as u64,
                            normal.iter().fold(0, |sum, &v| sum * 3 + (v + 1) as u64),
                        ]);
                        for spawn in glue_burst(*pos, *normal, *full, &mut rng) {
                            let particle = particle(ParticleKind::Slime, spawn, &mut rng);
                            frame.add_particle(&particle, age);
                        }
                    }
                }
                Event::Particles(kind, origin, motion, within, per_tick, ticks) => {
                    for emitted in 0..*ticks {
                        let age = tick - (at + emitted as f32);
                        if age < 0.0 {
                            break;
                        }
                        if age >= EMITTED_LIFE {
                            continue;
                        }
                        let mut rng = Rng::seeded(&[
                            *at as u64,
                            u64::from(emitted),
                            origin[0].to_bits().into(),
                            origin[2].to_bits().into(),
                        ]);
                        for _ in 0..emission_runs(*per_tick, &mut rng) {
                            let mut start = origin.map(f64::from);
                            if *within {
                                start = within_block(start, &mut rng);
                            }
                            let spawn = Spawn { pos: start, motion: motion.map(f64::from) };
                            let particle = particle(*kind, spawn, &mut rng);
                            frame.add_particle(&particle, age);
                        }
                    }
                }
                Event::Marker => {}
            }
        }
        if !self.belt_actions.is_empty() {
            let world = belt_items::World {
                belt: &|pos| self.belts.get(&pos).copied(),
                rpm: &|pos, tick| {
                    self.speed_log.get(&pos).map_or(0.0, |log| {
                        log.iter().rev().find(|(from, _)| *from <= tick as f32).map_or(0.0, |&(_, rpm)| rpm)
                    })
                },
                ending: &|pos| match self.blocks.get(&pos).and_then(|&state| self.palette.get(state)) {
                    None => Ending::Eject,
                    Some(block) if block.id.starts_with("create:") => Ending::Insert,
                    Some(_) => Ending::Blocked,
                },
            };
            for placed in belt_items::run(&world, &self.belt_actions, tick.floor() as i64) {
                frame.items.push((&self.belt_stacks[placed.stack as usize], placed.at.map(|v| v as f32)));
            }
        }
        if !has_reveals {
            let static_blocks = &mut sections.entry(0).or_default().blocks;
            static_blocks.extend(self.blocks.keys().chain(edited.keys()));
        }
        for section in sections.values() {
            let offset = section.offset.at(tick);
            let angles = section.rotation.at(tick);
            let pivot = section.pivot.unwrap_or(self.center);
            for &pos in &section.blocks {
                let offset = if let Some(fade) = section.fades.get(&pos) {
                    let Some(fade) = fade.translation(tick) else {
                        continue;
                    };
                    std::array::from_fn(|axis| offset[axis] + fade[axis])
                } else {
                    offset
                };
                let moving = angles != [0.0; 3] || offset != [0.0; 3];
                let Some(&state) = edited.get(&pos).or_else(|| self.blocks.get(&pos)) else {
                    continue;
                };
                let pose = moving.then(|| {
                    (
                        translation(pos, offset, angles.map(f32::to_radians), pivot),
                        angles,
                    )
                });
                let rotation = rotations.get(&pos).copied().unwrap_or_default();
                frame.blocks.push(VisibleBlock {
                    pos,
                    state,
                    rotation,
                    toggled: toggled.contains(&pos),
                    pose,
                    animated: animations.get(&pos).map(|steps| anim::value_at(steps, tick)),
                    arm: arms.get(&pos).map(|(steps, held)| {
                        let facing = |at: Pos| {
                            let state = *edited.get(&at).or_else(|| self.blocks.get(&at))?;
                            let (_, value) = self.palette.get(state)?.properties.iter().find(|(key, _)| key == "facing")?;
                            Dir::from_name(value)
                        };
                        let ceiling = self.palette.get(state).is_some_and(|block| {
                            block.properties.iter().any(|(key, value)| key == "ceiling" && value == "true")
                        });
                        let reach = Arm {
                            pos,
                            ceiling,
                            points: self.arms.get(&pos).map_or(&[], Vec::as_slice),
                            facing: &facing,
                        };
                        (reach.angles(steps, tick), held.map(str::to_owned))
                    }),
                    press: presses.get(&pos).map(|&(at, mode)| {
                        // The press turns at the scene's speed when the stroke runs.
                        let rpm = rotation.rpm;
                        (press_ticks(rpm, tick - at), mode)
                    }),
                });
            }
        }
        frame
    }

    pub fn draw(
        &self,
        host: &dyn Host,
        batch: &mut HudBatch,
        rect: [f32; 4],
        showing: Showing,
        frame: &Frame<'_>,
        tick: f32,
    ) {
        let mut blocks: Vec<_> = frame
            .blocks
            .iter()
            .filter_map(|visible| {
                let block = self.palette.get(visible.state)?;
                let actual = if visible.toggled {
                    block.toggled.or(block.state)
                } else {
                    block.state
                }?;
                let custom_model = !block.models.is_empty() && self.proxy.is_some();
                let pose = visible.pose.filter(|_| self.proxy.is_some());
                let entity = (block.id.starts_with("create:") || pose.is_some()).then(|| {
                    let mut entity = ModBlockEntity::default();
                    entity.id.clone_from(&block.id);
                    crate::set_float(&mut entity.nbt, super::TICK, tick);
                    crate::set_float(&mut entity.nbt, super::ANGLE, visible.rotation.angle(tick));
                    crate::set_float(&mut entity.nbt, crate::SPEED, visible.rotation.rpm);
                    if let Some((ticks, mode)) = visible.press {
                        crate::set_float(
                            &mut entity.nbt,
                            super::PRESS_PROGRESS,
                            ticks / create_kinetics::press::CYCLE as f32,
                        );
                        entity
                            .nbt
                            .insert(super::PRESS_MODE.into(), crate::compat::Tag::Int(mode));
                    }
                    if let Some((angles, held)) = &visible.arm {
                        super::render::store_arm(&mut entity, *angles, held.as_deref());
                    }
                    if let Some(value) = visible.animated {
                        crate::set_float(&mut entity.nbt, super::render::ANIMATED, value);
                    }
                    if let Some((translation, rotation)) = pose {
                        super::render::store_pose(&mut entity, translation, rotation);
                    }
                    if custom_model {
                        super::render::store_models(
                            &mut entity,
                            block
                                .models
                                .iter()
                                .map(|model| (model.0.as_str(), model.1, model.2)),
                        );
                    } else if block.redraw && self.proxy.is_some() {
                        entity.nbt.insert(
                            super::render::DRAW_STATE.into(),
                            crate::compat::Tag::Int(actual.0 as i32),
                        );
                    } else if pose.is_some() {
                        super::render::store_geometry(
                            &mut entity,
                            &block.geometry,
                            &block.properties,
                        );
                    }
                    entity
                });
                let state = if block.redraw || custom_model || pose.is_some() {
                    self.proxy.unwrap_or(actual)
                } else {
                    actual
                };
                Some((visible.pos, state, entity))
            })
            .collect();
        if let Some(proxy) = self.proxy {
            for &(id, point) in &frame.items {
                let mut entity = ModBlockEntity::default();
                let pos = point.map(|axis| axis.floor() as i32);
                let offset = std::array::from_fn(|axis| point[axis] - pos[axis] as f32);
                super::render::store_item(&mut entity, id, offset);
                blocks.push((pos, proxy, Some(entity)));
            }
        }
        let blocks: Vec<_> = blocks
            .iter()
            .map(|(pos, state, entity)| NativeSceneBlock {
                pos: *pos,
                state: *state,
                entity: entity.as_ref(),
            })
            .collect();
        let view = self.view(rect, showing, frame);
        host.draw_scene(batch, rect, &blocks, view);
        for &(from, to, color) in &frame.lines {
            overlay_line(
                host,
                batch,
                rect,
                project(from, rect, view),
                project(to, rect, view),
                color,
            );
        }
        for &(at, colour, size) in &frame.particles {
            let [x, y] = project(at, rect, view);
            let side = size * view.scale / 30.0;
            if x > rect[0] + side && x < rect[2] - side && y > rect[1] + side && y < rect[3] - side {
                host.fill(batch, [x - side * 0.5, y - side * 0.5, side, side], colour);
            }
        }
        if let Some(point) = frame.point {
            let [x, y] = project(point, rect, view);
            if x > rect[0] + 3.0 && x < rect[2] - 3.0 && y > rect[1] + 3.0 && y < rect[3] - 3.0 {
                host.fill(batch, [x - 3.0, y - 1.0, 6.0, 2.0], 0xFFE6_B873);
                host.fill(batch, [x - 1.0, y - 3.0, 2.0, 6.0], 0xFFE6_B873);
            }
        }
    }

    pub fn reading_tick(&self, mut tick: f32, mut elapsed: f32) -> f32 {
        for &(start, end) in &self.reading {
            if tick >= end {
                continue;
            }
            let gap = (start - tick).max(0.0);
            if elapsed <= gap {
                break;
            }
            tick += gap;
            elapsed -= gap;
            let reading_time = (end - tick) * 3.0;
            if elapsed <= reading_time {
                return tick + elapsed / 3.0;
            }
            tick = end;
            elapsed -= reading_time;
        }
        tick + elapsed
    }

    fn view(&self, rect: [f32; 4], showing: Showing, frame: &Frame<'_>) -> NativeSceneView {
        let width = rect[2] - rect[0];
        let height = rect[3] - rect[1];
        let fit = (width * 0.76).min(height * 1.2) / self.dimension;
        let mut view = NativeSceneView {
            center: self.center,
            yaw: showing.yaw + frame.yaw,
            pitch: showing.pitch,
            scale: (30.0 * self.scale).min(fit) * showing.zoom,
        };
        if showing.shift != [0.0; 2] {
            let [x, y] = showing.shift.map(|value| value / view.scale);
            let (y, z) = rotate(y, 0.0, view.pitch.to_radians());
            let (z, x) = rotate(z, x, -view.yaw.to_radians());
            for (value, shift) in view.center.iter_mut().zip([x, y, z]) {
                *value += shift;
            }
        }
        view
    }

    pub fn caption_target(
        &self,
        rect: [f32; 4],
        showing: Showing,
        frame: &Frame<'_>,
    ) -> Option<[f32; 2]> {
        Some(project(
            frame.caption_style?.target?,
            rect,
            self.view(rect, showing, frame),
        ))
    }

    pub fn inspected_block(
        &self,
        rect: [f32; 4],
        showing: Showing,
        frame: &Frame<'_>,
        mouse: [f32; 2],
    ) -> Option<&str> {
        let view = self.view(rect, showing, frame);
        let x = ((rect[0] + rect[2]) * 0.5 - mouse[0]) / view.scale;
        let y = ((rect[1] + rect[3]) * 0.5 - mouse[1]) / view.scale;
        let pitch = view.pitch.to_radians();
        let yaw = view.yaw.to_radians();
        let (origin_y, origin_z) = rotate(y, 0.0, pitch);
        let (origin_z, origin_x) = rotate(origin_z, x, -yaw);
        let (dir_y, dir_z) = rotate(0.0, 1.0, pitch);
        let (dir_z, dir_x) = rotate(dir_z, 0.0, -yaw);
        let origin: [f32; 3] =
            std::array::from_fn(|i| view.center[i] + [origin_x, origin_y, origin_z][i]);
        let direction = [dir_x, dir_y, dir_z];
        frame
            .blocks
            .iter()
            .filter_map(|block| {
                let state = self.palette.get(block.state)?;
                state.state?;
                let (offset, angles) = block.pose.unwrap_or(([0.0; 3], [0.0; 3]));
                let mut from =
                    std::array::from_fn(|i| origin[i] - block.pos[i] as f32 - offset[i] - 0.5);
                let mut dir = direction;
                for point in [&mut from, &mut dir] {
                    (point[0], point[1]) = rotate(point[0], point[1], -angles[2].to_radians());
                    (point[2], point[0]) = rotate(point[2], point[0], -angles[1].to_radians());
                    (point[1], point[2]) = rotate(point[1], point[2], -angles[0].to_radians());
                }
                let mut near = f32::NEG_INFINITY;
                let mut far = f32::INFINITY;
                for i in 0..3 {
                    if dir[i].abs() < 0.0001 {
                        if from[i].abs() > 0.5 {
                            return None;
                        }
                        continue;
                    }
                    let a = (-0.5 - from[i]) / dir[i];
                    let b = (0.5 - from[i]) / dir[i];
                    near = near.max(a.min(b));
                    far = far.min(a.max(b));
                }
                (near <= far).then_some((near, state.id.as_str()))
            })
            .min_by(|(a, _), (b, _)| a.total_cmp(b))
            .map(|(_, id)| id)
    }
}

fn press_ticks(rpm: f32, elapsed: f32) -> f32 {
    create_kinetics::press::virtual_ticks(rpm, elapsed)
}

fn rotate(a: f32, b: f32, angle: f32) -> (f32, f32) {
    let (sin, cos) = angle.sin_cos();
    (a * cos - b * sin, a * sin + b * cos)
}

fn translation(pos: Pos, offset: [f32; 3], angles: [f32; 3], pivot: [f32; 3]) -> [f32; 3] {
    let [mut x, mut y, mut z] = std::array::from_fn(|i| pos[i] as f32 + 0.5 - pivot[i]);
    (y, z) = rotate(y, z, angles[0]);
    (z, x) = rotate(z, x, angles[1]);
    (x, y) = rotate(x, y, angles[2]);
    let relative = [x, y, z];
    std::array::from_fn(|i| relative[i] + pivot[i] + offset[i] - 0.5 - pos[i] as f32)
}

fn project(point: [f32; 3], rect: [f32; 4], view: NativeSceneView) -> [f32; 2] {
    let [x, y, z] = std::array::from_fn(|i| point[i] - view.center[i]);
    let (z, x) = rotate(z, x, view.yaw.to_radians());
    let (y, _) = rotate(y, z, -view.pitch.to_radians());
    [
        (rect[0] + rect[2]) * 0.5 - view.scale * x,
        (rect[1] + rect[3]) * 0.5 - view.scale * y,
    ]
}

fn overlay_line(
    host: &dyn Host,
    batch: &mut HudBatch,
    rect: [f32; 4],
    a: [f32; 2],
    b: [f32; 2],
    color: u32,
) {
    let delta = [b[0] - a[0], b[1] - a[1]];
    let mut begin: f32 = 0.0;
    let mut end: f32 = 1.0;
    for axis in 0..2 {
        let (low, high) = (rect[axis] + 1.0, rect[axis + 2] - 1.0);
        if delta[axis].abs() < f32::EPSILON {
            if a[axis] < low || a[axis] > high {
                return;
            }
        } else {
            let t0 = (low - a[axis]) / delta[axis];
            let t1 = (high - a[axis]) / delta[axis];
            begin = begin.max(t0.min(t1));
            end = end.min(t0.max(t1));
        }
    }
    let length = delta[0].hypot(delta[1]);
    if begin >= end || length < 0.01 {
        return;
    }
    let [x0, y0] = std::array::from_fn(|i| a[i] + begin * delta[i]);
    let [x1, y1] = std::array::from_fn(|i| a[i] + end * delta[i]);
    let [dx, dy] = [-delta[1] / length, delta[0] / length];
    host.quad(
        batch,
        [
            [x0 - dx, y0 - dy],
            [x0 + dx, y0 + dy],
            [x1 + dx, y1 + dy],
            [x1 - dx, y1 - dy],
        ],
        color,
    );
}

fn decode(row: &[Value], palette: &mut Vec<State>) -> Option<(f32, Event)> {
    let tick = read::<u32>(row, 0)? as f32;
    let op = row.get(1)?.as_str()?;
    let color = |name: &str| match name {
        "blue" => 0xFF6C_B9F8,
        "red" => 0xFFED_715E,
        "green" => 0xFF95_C971,
        "medium" => 0xFFE6_B873,
        _ => 0xFFF4_E9D4,
    };
    let event = match op {
        "show" => Event::Show(read(row, 2)?, read(row, 3)?, read(row, 4)),
        "hide" => Event::Hide(read(row, 2)?, read(row, 3)),
        "hide_section" => Event::HideSection(read(row, 2)?, read(row, 3)),
        "block" => {
            let state = (read(row, 3)?, read(row, 4)?);
            let index = palette
                .iter()
                .position(|entry| entry == &state)
                .unwrap_or_else(|| {
                    palette.push(state);
                    palette.len() - 1
                });
            Event::Block(read(row, 2)?, index)
        }
        "restore" => Event::Restore(read(row, 2)?),
        "speed" => Event::Speed(read(row, 2)?, 0.0, read(row, 3)?),
        "multiply_speed" => Event::Speed(read(row, 2)?, read(row, 3)?, 0.0),
        "affine_speed" => Event::Speed(read(row, 2)?, read(row, 3)?, read(row, 4)?),
        "move" => Event::Move(read(row, 2)?, read(row, 3)?, read(row, 4)?),
        "rotate" => Event::Rotate(read(row, 2)?, read(row, 3)?, read(row, 4)?),
        "pivot" => Event::Pivot(read(row, 2)?, read(row, 3)?),
        "caption" => Event::Caption(
            read(row, 2)?,
            read(row, 3)?,
            read(row, 4).unwrap_or_default(),
        ),
        "control" => Event::Control(read(row, 2)?, read(row, 3)?),
        "item" => Event::Item(read(row, 2)?, read(row, 3)?, read(row, 4)?, read(row, 5)?),
        "line" => Event::Line(
            read(row, 2)?,
            read(row, 3)?,
            color(row.get(4)?.as_str()?),
            read(row, 5)?,
        ),
        "outline" => Event::Outline(
            read(row, 2)?,
            read(row, 3)?,
            color(row.get(4)?.as_str()?),
            read(row, 5)?,
        ),
        "toggle" => Event::Toggle(read(row, 2)?),
        "press" => Event::Press(
            read(row, 2)?,
            match row.get(3)?.as_str()? {
                "belt" => create_kinetics::press::Mode::Belt,
                "basin" => create_kinetics::press::Mode::Basin,
                _ => create_kinetics::press::Mode::World,
            }
            .id(),
        ),
        "yaw" => Event::Yaw(read(row, 2)?),
        "animate" => Event::Animate(read(row, 3)?, read(row, 4)?, read(row, 5)?),
        "belt_put" => Event::BeltPut(read(row, 2)?, read(row, 4)?, read(row, 3)?),
        "belt_remove" => Event::BeltRemove(read(row, 2)?),
        "belt_stall" => Event::BeltStall(read(row, 2)?, read(row, 3)?),
        "belt_change" => Event::BeltChange(read(row, 2)?, read(row, 3)?),
        "arm" => Event::Arm(
            read(row, 2)?,
            ArmPhase::from_name(row.get(3)?.as_str()?),
            read(row, 4),
            read(row, 5)?,
        ),
        "glue" => Event::Glue(read(row, 2)?, read(row, 3)?, read(row, 4)?),
        "particles" => Event::Particles(
            ParticleKind::from_name(row.get(2)?.as_str()?)?,
            read(row, 3)?,
            read(row, 4)?,
            read(row, 5)?,
            read(row, 6)?,
            read(row, 7)?,
        ),
        "marker" => Event::Marker,
        _ => return None,
    };
    Some((tick, event))
}

fn resolve(
    host: &dyn Host,
    palette: &[State],
    models: BTreeMap<State, Vec<Model>>,
) -> Vec<ResolvedBlock> {
    let mut states: BTreeMap<&str, Vec<BlockStateId>> = palette
        .iter()
        .filter(|(_, props)| !props.is_empty())
        .map(|(id, _)| (id.as_str(), Vec::new()))
        .collect();
    if !states.is_empty() {
        for (id, block) in host.block_states().iter() {
            if let Some(all) = states.get_mut(block.name.as_ref()) {
                all.push(id);
            }
        }
    }
    let unavailable = host.default_state("minecraft:light_gray_stained_glass");
    palette
        .iter()
        .map(|(id, props)| {
            let default = host.default_state(id);
            let find = |props: &Properties| {
                let default = default?;
                let translated = match (id.as_str(), props.get("facing")) {
                    ("create:water_wheel", Some(face)) => {
                        let axis = match face.as_str() {
                            "east" | "west" => "x",
                            "north" | "south" => "z",
                            _ => "y",
                        };
                        BTreeMap::from([(String::from("axis"), String::from(axis))])
                    }
                    _ => props.clone(),
                };
                let matches = |state| {
                    translated.iter().all(|(key, value)| {
                        host.block_state_property(state, key)
                            .is_none_or(|saved| saved == value)
                    })
                };
                if matches(default) {
                    Some(default)
                } else {
                    states
                        .get(id.as_str())
                        .and_then(|all| all.iter().copied().find(|&state| matches(state)))
                        .or(Some(default))
                }
            };
            let state = match id.as_str() {
                "minecraft:air" | "minecraft:structure_void" | "minecraft:barrier" => None,
                "minecraft:water" => host.default_state("minecraft:blue_stained_glass"),
                "minecraft:lava" => host.default_state("minecraft:orange_stained_glass"),
                _ => find(props).or(unavailable),
            };
            let toggled = props
                .get("powered")
                .map(|value| {
                    let mut other = props.clone();
                    other.insert("powered".into(), (value != "true").to_string());
                    other
                })
                .and_then(|other| find(&other));
            let redraw = default.is_some()
                && (id == crate::belt::ID
                    || crate::Block::of(id).is_some_and(|block| block != crate::Block::Press));
            let geometry = match id.as_str() {
                "minecraft:water" => "minecraft:blue_stained_glass",
                "minecraft:lava" => "minecraft:orange_stained_glass",
                _ if default.is_none() => "minecraft:light_gray_stained_glass",
                _ => id,
            };
            let properties = state.map_or_else(Vec::new, |state| {
                props
                    .keys()
                    .filter_map(|key| {
                        host.block_state_property(state, key)
                            .map(|value| (key.clone(), value.to_owned()))
                    })
                    .collect()
            });
            ResolvedBlock {
                state,
                toggled,
                redraw,
                id: id.clone(),
                models: if default.is_none() {
                    models
                        .get(&(id.clone(), props.clone()))
                        .cloned()
                        .unwrap_or_default()
                } else {
                    Vec::new()
                },
                geometry: geometry.to_owned(),
                properties,
            }
        })
        .collect()
}
