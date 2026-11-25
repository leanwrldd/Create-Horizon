//! The original Create catalog and compiled scene instructions.

use std::collections::{BTreeMap, BTreeSet};

use crate::compat::{
    BlockStateId, Host, HudBatch, ItemStack, ModBlockEntity, NativeSceneBlock, NativeSceneView, Tag,
    serde_json::{self, Value},
};

use create_kinetics::Dir;
use create_kinetics::belt::{Ending, Part as BeltPart, Slope as BeltSlope};
use create_kinetics::rules::fluid_tank::Part as TankPart;
use create_kinetics::rules::ponder_belt::{self as belt_items, Action as BeltAction, Segment as BeltSegment};
use create_kinetics::rules::ponder_fluid::{self as tank_fluid, Kind as VesselKind, Op as FluidOp, Saved as SavedTank, Vessel};
use create_kinetics::rules::arm::Phase as ArmPhase;
use create_kinetics::rules::arm_points::{Mode as ArmMode, Point as ArmPoint, PointType as ArmPointType};
use create_kinetics::rules::ponder_anim::{self as anim, Instruction};
use create_kinetics::rules::ponder_arm::{Arm, Instruction as ArmInstruction};
use create_kinetics::rules::ponder_particles::{
    Kind as ParticleKind, Particle, Rng, Spawn, destroy_burst, emission_runs, glue_burst, jittered, jittered_across, particle,
    within_block,
};

use crate::machines::funnel as create_funnel;
use crate::machines::tunnel_world;

use create_kinetics::rules::ponder_crafter::{Action as CrafterAction, Target as CrafterTarget, Timeline as CrafterTimeline};

use super::crafters;
use super::scene::Pos;
use super::screen::Showing;

/// The longest an item or block break particle lives: `(int)(4 / 0.1)` ticks.
const GLUE_LIFE: f32 = 40.0;
/// The first link a stack a crafter passed on takes: the scene's own count from 1 stays below.
const CRAFTED_LINKS: u32 = 1_000_000;
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
    /// The scene's Fluid Tank blocks.
    tanks: Vec<SavedTank>,
    /// The scene's Display Boards: the controller block and the board's size.
    boards: Vec<(Pos, i32, i32)>,
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
            tanks: field_or(value, "tanks"),
            boards: field_or(value, "boards"),
            duration: field(value, "duration")?,
            scale: field(value, "scale")?,
            offset: field(value, "offset")?,
        })
    }
}

/// `[pos, controller, width, height, window, fluid, amount]`: a Fluid Tank block as the structure file saves it.
impl FromJson for SavedTank {
    fn from_json(value: &Value) -> Option<Self> {
        let items = value.as_array()?;
        Some(Self {
            pos: read(items, 0)?,
            controller: read(items, 1)?,
            width: read(items, 2)?,
            height: read(items, 3)?,
            window: read::<i32>(items, 4)? != 0,
            fluid: read(items, 5)?,
            amount: read(items, 6)?,
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
    /// `createItemEntity`: the item, where, its motion, how long it lives and its link (0 for none).
    Item(String, [f32; 3], [f32; 3], f32, u32),
    /// `modifyEntity(link, Entity::discard)` (the link), or `modifyEntities(ItemEntity.class, Entity::discard)` (0).
    ItemEnd(u32),
    /// `modifyBlockEntityNBT` and its relatives: what to set in the block entity's data.
    Nbt(Vec<Pos>, Vec<(String, Tag)>),
    /// `flapFunnel(pos, outward)`.
    Flap(Pos, bool),
    /// The breaking texture's stage over a block (`incrementBlockBreakingProgress`), or none (-1).
    Crack(Pos, i32),
    /// A block broke: its crumbs fly (`PonderLevel.addBlockDestroyEffects`).
    Destroy(Pos),
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
    /// Fluid into or out of the Fluid Tanks at these blocks (`rules::ponder_fluid`).
    Fluid(Vec<Pos>, FluidOp),
    /// `effects.superGlue(pos, side, fullBlock)`: a burst of glue on a block's face (`SuperGlueItem.spawnParticles`).
    Glue(Pos, [i32; 3], bool),
    /// `effects.emitParticles`: `kind` at a point, its motion, within the block's space or not, per tick, for ticks.
    /// The last is the radius of the random offset each particle's motion takes (`VecHelper.offsetRandomly`).
    /// The last says the offset is across only (`jittered_across`: a press's item particles).
    Particles(ParticleKind, [f32; 3], [f32; 3], bool, f32, u32, f32, bool),
    /// `setDisplayBoardText` and `dyeDisplayBoard` on the board whose controller is here.
    Board(Pos, usize, BoardEdit),
    /// What a scene did to its Mechanical Crafters (`rules::ponder_crafter`): one action for each block it names.
    Crafter(Vec<CrafterAction<ItemStack>>),
    /// `flashDisplayLink`: the bulb of the links here glows from now (`LinkWithBulbBlockEntity.pulse`).
    Pulse(Vec<Pos>),
    Marker,
}

/// What a scene set on one line of a Display Board.
#[derive(Clone)]
pub enum BoardEdit {
    /// The line's text: literal pieces, language keys and item names (`kind`, text), as the language words them.
    Text(Vec<(String, String)>),
    /// The line's dye (`None`: white).
    Dye(Option<String>),
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
    /// The scene's Fluid Tank blocks as the structure file left them.
    tanks: BTreeMap<Pos, TankPart>,
    /// The scene's belt blocks, and the items put on belts and depots (`rules::ponder_belt`).
    belts: BTreeMap<Pos, BeltSegment>,
    belt_actions: Vec<(i64, BeltAction)>,
    belt_stacks: Vec<String>,
    /// Each block's speed over time: from the tick, in rpm.
    speed_log: BTreeMap<Pos, Vec<(f32, f32)>>,
    /// The Mechanical Crafters' replay, when the scene instructs any.
    crafters: Option<CrafterTimeline<ItemStack>>,
    /// The scene's Display Boards: where each is controlled and its size.
    boards: BTreeMap<Pos, (i32, i32)>,
    /// The Item Drains, Spouts and Basins a scene can fill (`rules::ponder_fluid::Vessel`).
    vessels: BTreeMap<Pos, VesselKind>,
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
    /// What scene instructions set in the block entity's data, in order.
    nbt: Vec<(String, Tag)>,
    /// The lines a scene wrote on a Display Board, in order (a controller's own).
    board: Vec<(usize, BoardEdit)>,
    /// A funnel's last flap: the tick and whether it went outward.
    flap: Option<(f32, bool)>,
    /// The stage of the breaking texture over the block.
    crack: Option<i32>,
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
        // The Mechanical Crafters run Create's own logic in a scene, so they are replayed from the first tick; what
        // they put on a Depot or a Belt and what they eject join the scene's items.
        let crafters = {
            let blocks: BTreeMap<Pos, usize> = file.blocks.iter().copied().collect();
            let mut scenery = crafters::Scenery {
                palette: &file.palette,
                blocks: &blocks,
                changes: BTreeMap::new(),
                toggles: BTreeMap::new(),
                speeds: &speed_log,
            };
            let mut actions: crafters::Actions = Vec::new();
            let depot = |pos: &Pos| {
                blocks.get(pos).and_then(|&state| file.palette.get(state)).is_some_and(|(id, _)| id == "create:depot")
            };
            for (at, event) in &events {
                let at = *at as i64;
                match event {
                    Event::Block(positions, state) => {
                        for &pos in positions {
                            scenery.changes.entry(pos).or_default().push((at, Some(*state)));
                        }
                    }
                    Event::Restore(positions) => {
                        for &pos in positions {
                            scenery.changes.entry(pos).or_default().push((at, None));
                        }
                    }
                    Event::Toggle(positions) => {
                        for &pos in positions {
                            scenery.toggles.entry(pos).or_default().push(at);
                        }
                    }
                    Event::Crafter(rows) => actions.extend(rows.iter().cloned().map(|row| (at, row))),
                    Event::Nbt(positions, entries) => {
                        for (key, tag) in entries {
                            if key == "Cover"
                                && let Tag::Byte(covered) = tag
                            {
                                actions.extend(
                                    positions.iter().map(|&pos| (at, CrafterAction::Cover { pos, covered: *covered != 0 })),
                                );
                            }
                        }
                    }
                    Event::BeltPut(_, _, pos) if depot(pos) => actions.push((at, CrafterAction::Occupy { pos: *pos })),
                    Event::BeltRemove(pos) => actions.push((at, CrafterAction::Clear { pos: *pos })),
                    _ => {}
                }
            }
            crafters::replay(&scenery, &actions, i64::from(file.duration.max(1)))
        };
        if let Some(timeline) = &crafters {
            // An ejected stack drops from the crafter's front; a scene's `modifyEntities(ItemEntity.class, ..)` clears it.
            for dropped in &timeline.drops {
                let point = crafters::eject_point(dropped.pos, dropped.facing);
                events.push((dropped.tick as f32, Event::Item(dropped.stack.item.to_string(), point, [0.0, -0.04, 0.0], 25.0, 0)));
            }
            events.sort_by(|(a, _), (b, _)| a.total_cmp(b));
        }
        // An item entity lives until a scene discards it (its link, or every item with 0).
        let discards: Vec<(f32, u32)> = events
            .iter()
            .filter_map(|(at, event)| match event {
                Event::ItemEnd(link) => Some((*at, *link)),
                _ => None,
            })
            .collect();
        for (at, event) in &mut events {
            if let Event::Item(_, _, _, duration, link) = event
                && let Some(&(end, _)) = discards
                    .iter()
                    .find(|&&(end, discarded)| end > *at && (discarded == 0 || discarded == *link))
            {
                *duration = duration.min(end - *at);
            }
        }
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
        let mut belt_actions: Vec<(i64, BeltAction)> = events
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
        if let Some(timeline) = &crafters {
            // What a crafter passed to a Depot or a Belt rests or rides there like an item the scene put.
            for (n, output) in timeline.outputs.iter().enumerate() {
                if matches!(output.kind, CrafterTarget::Depot | CrafterTarget::Belt) {
                    let link = CRAFTED_LINKS + n as u32;
                    belt_actions.push((output.tick, BeltAction::Put { link, stack: stack_of(&output.stack.item), pos: output.at }));
                }
            }
            belt_actions.sort_by_key(|(at, _)| *at);
        }
        let tanks = {
            let creative = |pos: Pos| {
                file.blocks
                    .iter()
                    .find(|(at, _)| *at == pos)
                    .and_then(|(_, state)| file.palette.get(*state))
                    .is_some_and(|(id, _)| id == "create:creative_fluid_tank")
            };
            tank_fluid::parts(&file.tanks, creative)
        };
        Self {
            events,
            markers,
            crafters,
            boards: file.boards.iter().map(|&(pos, x, y)| (pos, (x, y))).collect(),
            vessels: file
                .blocks
                .iter()
                .filter_map(|&(pos, state)| Some((pos, VesselKind::of(&file.palette.get(state)?.0)?)))
                .collect(),
            tanks,
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
        let mut data = BTreeMap::<Pos, Vec<(String, Tag)>>::new();
        let mut flaps = BTreeMap::<Pos, (f32, bool)>::new();
        let mut boards = BTreeMap::<Pos, Vec<(usize, BoardEdit)>>::new();
        let mut cracks = BTreeMap::<Pos, i32>::new();
        let mut tanks = self.tanks.clone();
        let mut vessels = BTreeMap::<Pos, Vessel>::new();
        let mut has_reveals = false;
        let mut frame = Frame::default();
        for (index, (at, event)) in self.events.iter().enumerate() {
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
                Event::Item(item, point, velocity, duration, _) => {
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
                Event::ItemEnd(_) => {}
                Event::Nbt(positions, entries) => {
                    for &pos in positions {
                        let stored = data.entry(pos).or_default();
                        for (key, tag) in entries {
                            set_nbt(stored, key, tag, *at as i64);
                        }
                    }
                }
                Event::Fluid(positions, op) => {
                    for &pos in positions {
                        if let Some(&kind) = self.vessels.get(&pos) {
                            vessels.entry(pos).or_insert_with(|| Vessel::new(kind)).apply(op);
                        } else {
                            tank_fluid::apply(&mut tanks, pos, op);
                        }
                    }
                }
                Event::Flap(pos, outward) => {
                    flaps.insert(*pos, (*at, *outward));
                }
                Event::Crack(pos, stage) => {
                    if *stage < 0 {
                        cracks.remove(pos);
                    } else {
                        cracks.insert(*pos, *stage);
                    }
                }
                Event::Destroy(pos) => {
                    let age = tick - at;
                    if age < GLUE_LIFE {
                        for (n, spawn) in destroy_burst(*pos).into_iter().enumerate() {
                            let mut rng = Rng::seeded(&[
                                *at as u64,
                                n as u64,
                                pos[0] as u64,
                                pos[1] as u64,
                                pos[2] as u64,
                            ]);
                            let particle = particle(ParticleKind::Terrain, spawn, &mut rng);
                            frame.add_particle(&particle, age);
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
                Event::Pulse(positions) => {
                    for &pos in positions {
                        let stored = data.entry(pos).or_default();
                        stored.retain(|(known, _)| known != "Pulse");
                        stored.push(("Pulse".to_owned(), Tag::Long(*at as i64)));
                    }
                }
                Event::Particles(kind, origin, motion, within, per_tick, ticks, jitter, across) => {
                    for emitted in 0..*ticks {
                        let age = tick - (at + emitted as f32);
                        if age < 0.0 {
                            break;
                        }
                        if age >= EMITTED_LIFE {
                            continue;
                        }
                        // The event's place among the others tells apart emitters that start together.
                        let mut rng = Rng::seeded(&[
                            *at as u64,
                            u64::from(emitted),
                            origin[0].to_bits().into(),
                            origin[2].to_bits().into(),
                            index as u64,
                        ]);
                        for _ in 0..emission_runs(*per_tick, &mut rng) {
                            let mut start = origin.map(f64::from);
                            if *within {
                                start = within_block(start, &mut rng);
                            }
                            let motion = match (*jitter > 0.0, *across) {
                                (true, true) => jittered_across(*motion, *jitter, &mut rng),
                                (true, false) => jittered(*motion, *jitter, &mut rng),
                                _ => *motion,
                            };
                            let spawn = Spawn { pos: start, motion: motion.map(f64::from) };
                            let particle = particle(*kind, spawn, &mut rng);
                            frame.add_particle(&particle, age);
                        }
                    }
                }
                Event::Board(pos, line, edit) => boards.entry(*pos).or_default().push((*line, edit.clone())),
                Event::Crafter(_) => {}
                Event::Marker => {}
            }
        }
        // Each crafter shows what the replay of its chain left in it at this tick.
        if let Some(timeline) = &self.crafters {
            for &pos in timeline.states.keys() {
                let stored = data.entry(pos).or_default();
                for (key, tag) in crafters::tags(timeline, pos, tick.floor() as i64) {
                    stored.retain(|(known, _)| *known != key);
                    stored.push((key, tag));
                }
            }
        }
        // An Item Drain, a Spout or a Basin a scene filled shows its tanks as the block saves them.
        for (pos, vessel) in &vessels {
            let stored = data.entry(*pos).or_default();
            stored.retain(|(known, _)| known != "fluids");
            stored.push(("fluids".to_owned(), vessel_tag(vessel)));
        }
        // Each tank block shows what the structure file and the fluid instructions left in it.
        for (pos, part) in &tanks {
            data.entry(*pos).or_default().extend(crate::machines::fluid_tank::scene_tags(*pos, part));
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
                    nbt: data.get(&pos).cloned().unwrap_or_default(),
                    board: boards.get(&pos).cloned().unwrap_or_default(),
                    flap: flaps.get(&pos).copied(),
                    crack: cracks.get(&pos).copied(),
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
                let cracked = visible.crack.is_some();
                let entity = (block.id.starts_with("create:") || pose.is_some() || cracked).then(|| {
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
                    for (key, tag) in &visible.nbt {
                        entity.nbt.insert(key.as_str().into(), tag.clone());
                    }
                    if let Some(&(x, y)) = self.boards.get(&visible.pos) {
                        crate::machines::flap_display::write_board(&mut entity.nbt, &display_board(host, x, y, &visible.board));
                    }
                    if let Some((at, outward)) = visible.flap {
                        // `FunnelBlockEntity.flap(inward)`: `FlapTick` is the scene tick the flap began.
                        entity.nbt.insert(
                            create_funnel::FLAP_KEY.into(),
                            Tag::Byte(if outward { 1 } else { -1 }),
                        );
                        entity.nbt.insert(create_funnel::FLAP_TICK.into(), Tag::Long(at as i64));
                    }
                    if let Some(stage) = visible.crack {
                        entity.nbt.insert(super::render::CRACK.into(), Tag::Int(stage));
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
                    } else if pose.is_some() || (cracked && !block.id.starts_with("create:")) {
                        super::render::store_geometry(
                            &mut entity,
                            &block.geometry,
                            &block.properties,
                        );
                    }
                    entity
                });
                let state = if block.redraw || custom_model || pose.is_some() || (cracked && !block.id.starts_with("create:")) {
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
        // `PonderPalette.INPUT`, `OUTPUT` and `BLACK`, from the Ponder library.
        "input" => 0xFF7F_CDE0,
        "output" => 0xFFDD_C166,
        "black" => 0xFF22_1111,
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
        "item" => Event::Item(
            read(row, 2)?,
            read(row, 3)?,
            read(row, 4)?,
            read(row, 5)?,
            read(row, 6).unwrap_or(0),
        ),
        "item_end" => Event::ItemEnd(read(row, 2)?),
        "nbt" => Event::Nbt(read(row, 2)?, nbt_entries(row.get(3)?)?),
        "flap" => Event::Flap(read(row, 2)?, read(row, 3)?),
        "crack" => Event::Crack(read(row, 2)?, read(row, 3)?),
        "destroy" => Event::Destroy(read(row, 2)?),
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
            read(row, 8).unwrap_or(0.0),
            read(row, 9).unwrap_or(false),
        ),
        "pulse" => Event::Pulse(read(row, 2)?),
        "fluid" => Event::Fluid(
            read(row, 2)?,
            FluidOp::from_row(row.get(3)?.as_str()?, row.get(4)?.as_str()?, read(row, 5)?)?,
        ),
        "crafter" => Event::Crafter(crafter_actions(row)?),
        "board" => {
            let positions: Vec<Pos> = read(row, 3)?;
            let edit = match row.get(2)?.as_str()? {
                "text" => BoardEdit::Text(
                    row.get(5)?
                        .as_array()?
                        .iter()
                        .map(|part| Some((part.get(0)?.as_str()?.to_owned(), part.get(1)?.as_str()?.to_owned())))
                        .collect::<Option<_>>()?,
                ),
                "dye" => BoardEdit::Dye(Some(row.get(5)?.as_str()?.to_owned()).filter(|dye| dye != "white")),
                _ => return None,
            };
            Event::Board(*positions.first()?, read(row, 4)?, edit)
        }
        "marker" => Event::Marker,
        _ => return None,
    };
    Some((tick, event))
}

/// `[tick, "crafter", kind, positions, item, count]`: an action for each of the crafters it names (`connect` names
/// the two that are joined).
fn crafter_actions(row: &[Value]) -> Option<Vec<CrafterAction<ItemStack>>> {
    let kind = row.get(2)?.as_str()?;
    let positions: Vec<Pos> = read(row, 3)?;
    let id: String = read(row, 4)?;
    let count: u32 = read(row, 5)?;
    let stack = || ItemStack::new(&id, count.max(1));
    let one = || ItemStack::new(&id, 1);
    Some(match kind {
        "connect" => vec![CrafterAction::Connect { first: *positions.first()?, second: *positions.get(1)? }],
        "result" => positions.into_iter().map(|pos| CrafterAction::Result { pos, stack: stack() }).collect(),
        "insert" => positions.into_iter().map(|pos| CrafterAction::Insert { pos, stack: one() }).collect(),
        "set" => positions.into_iter().map(|pos| CrafterAction::Set { pos, stack: one() }).collect(),
        "stack" => positions.into_iter().map(|pos| CrafterAction::Stacked { pos, stack: one(), count }).collect(),
        "check" => positions.into_iter().map(|pos| CrafterAction::Check { pos }).collect(),
        "eject" => positions.into_iter().map(|pos| CrafterAction::Eject { pos }).collect(),
        _ => return None,
    })
}

/// `{key: [type, value]}` as block entity data: `i` an int, `f` a float, `b` a boolean, `s` a string (an enum is its
/// name, as `NBTHelper.writeEnum` saves it) and `item` a stack (empty for none).
fn nbt_entries(value: &Value) -> Option<Vec<(String, Tag)>> {
    value
        .as_object()?
        .iter()
        .map(|(key, entry)| {
            let kind = entry.get(0)?.as_str()?;
            let value = entry.get(1)?;
            let tag = match kind {
                "i" => Tag::Int(i32::try_from(value.as_i64()?).ok()?),
                "f" => Tag::Float((value.as_f64()? as f32).to_bits()),
                "b" => Tag::Byte(i8::from(value.as_bool()?)),
                "s" => Tag::String(value.as_str()?.into()),
                "item" => match value.as_str()? {
                    "" => Tag::Compound(crate::Compound::new()),
                    id => crate::util::stack_tag(&ItemStack::new(id, 1)),
                },
                // `TransportedItemStack.serializeNBT` of a stack on a Depot: at its start, put there from above.
                "held" => held_tag(value.as_str()?),
                // A side's filter: `{side: item}`, an empty id for none.
                "filters" => Tag::Compound(
                    value
                        .as_object()?
                        .iter()
                        .map(|(side, id)| {
                            let stack = match id.as_str()? {
                                "" => Tag::Compound(crate::Compound::new()),
                                id => crate::util::stack_tag(&ItemStack::new(id, 1)),
                            };
                            Some((side.as_str().into(), stack))
                        })
                        .collect::<Option<_>>()?,
                ),
                "items" => Tag::List(
                    value.as_array()?.iter().map(|id| Some(crate::util::stack_tag(&ItemStack::new(id.as_str()?, 1)))).collect::<Option<_>>()?,
                ),
                _ => return None,
            };
            Some((key.clone(), tag))
        })
        .collect()
}

/// A Display Board as a scene left it: each line's text worded from the language (`FlapDisplayBlockEntity.applyTextManually`
/// in a virtual world refreshes the flaps at once, so none is left spinning) and its dye.
fn display_board(host: &dyn Host, x_size: i32, y_size: i32, edits: &[(usize, BoardEdit)]) -> create_kinetics::rules::flap_display::Board {
    use create_kinetics::rules::flap_display::Board;
    let lang = crate::machines::goggles::lang(host);
    let mut board = Board::new(x_size, y_size);
    for (line, edit) in edits {
        match edit {
            BoardEdit::Text(parts) => {
                let text: String = parts
                    .iter()
                    .map(|(kind, text)| match kind.as_str() {
                        "lang" => lang.translate(text, &[]),
                        "name" => lang.item_name(text),
                        _ => text.clone(),
                    })
                    .collect();
                board.apply_text_manually(*line, Some(&text), 0);
                if let Some(layout) = board.lines.get_mut(*line) {
                    for section in &mut layout.sections {
                        section.prev.clone_from(&section.text);
                        section.since = None;
                    }
                }
            }
            BoardEdit::Dye(dye) => {
                if let Some(colour) = board.colour.get_mut(*line) {
                    colour.clone_from(dye);
                }
            }
        }
    }
    board
}

/// One key a scene set in a block entity's data, with what it implies: a Basin's output stacks fly for ten ticks
/// from when they were set, and a Brass Tunnel's filters are kept by side (each instruction sets one side) and open
/// the sides they are on.
fn set_nbt(stored: &mut Vec<(String, Tag)>, key: &str, tag: &Tag, at: i64) {
    let mut put = |stored: &mut Vec<(String, Tag)>, key: &str, tag: Tag| {
        stored.retain(|(known, _)| known != key);
        stored.push((key.to_owned(), tag));
    };
    match (key, tag) {
        ("VisualizedItems", _) => {
            put(stored, "VisualizedSince", Tag::Long(at));
            put(stored, key, tag.clone());
        }
        ("Filters", Tag::Compound(sides)) => {
            let mut filters = match stored.iter().find(|(known, _)| known == key) {
                Some((_, Tag::Compound(filters))) => filters.clone(),
                _ => crate::Compound::new(),
            };
            for (side, stack) in sides {
                match stack {
                    Tag::Compound(stack) if stack.is_empty() => {
                        filters.remove(side);
                    }
                    _ => {
                        filters.insert(side.clone(), stack.clone());
                    }
                }
            }
            // The slots a filter sits on are the tunnel's open sides (`BrassTunnelBlockEntity.sides`).
            let mut known = crate::Compound::new();
            if let Some((_, open)) = stored.iter().find(|(known, _)| known == tunnel_world::SIDES) {
                known.insert(tunnel_world::SIDES.into(), open.clone());
            }
            let mut open = tunnel_world::dirs_of(&known, tunnel_world::SIDES);
            for dir in sides.keys().filter_map(|side| create_kinetics::Dir::from_name(&side.to_string_lossy())) {
                if !open.contains(&dir) {
                    open.push(dir);
                }
            }
            put(stored, tunnel_world::SIDES, tunnel_world::dirs_tag(&open));
            put(stored, key, Tag::Compound(filters));
        }
        _ => put(stored, key, tag.clone()),
    }
}

/// The `fluids` list of an Item Drain, a Spout or a Basin: each tank's `Fluid` and `Amount`, `minecraft:empty` when bare.
fn vessel_tag(vessel: &Vessel) -> Tag {
    Tag::List(
        vessel
            .tanks
            .iter()
            .map(|(fluid, amount)| {
                let mut tank = crate::Compound::new();
                let fluid = fluid.as_deref().filter(|_| *amount > 0).unwrap_or("minecraft:empty");
                tank.insert("Fluid".into(), Tag::String(fluid.into()));
                tank.insert("Amount".into(), Tag::Int((*amount).max(0)));
                Tag::Compound(tank)
            })
            .collect(),
    )
}

/// A Depot's `HeldItem` for a stack a scene put on it (`new TransportedItemStack(stack).serializeNBT(..)`): `Item` and the
/// fields `TransportedItemStack` starts with, which `depot::draw` reads (it came in from above, so it rests in the middle).
fn held_tag(id: &str) -> Tag {
    let mut held = crate::Compound::new();
    held.insert("Item".into(), crate::util::stack_tag(&ItemStack::new(id, 1)));
    for key in ["Pos", "PrevPos", "Offset", "PrevOffset"] {
        crate::set_float(&mut held, key, 0.0);
    }
    held.insert("InSegment".into(), Tag::Int(0));
    held.insert("Angle".into(), Tag::Int(0));
    // `Direction.UP.get3DDataValue()`.
    held.insert("InDirection".into(), Tag::Int(1));
    Tag::Compound(held)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn row(json: &str) -> Vec<Value> {
        serde_json::from_str::<Value>(json).unwrap().as_array().unwrap().clone()
    }

    fn actions(json: &str) -> Vec<CrafterAction<ItemStack>> {
        match decode(&row(json), &mut Vec::new()) {
            Some((_, Event::Crafter(actions))) => actions,
            _ => panic!("not a crafter row"),
        }
    }

    #[test]
    fn a_crafter_row_is_an_action_for_each_crafter_it_names() {
        let planks = ItemStack::new("minecraft:oak_planks", 1);
        assert_eq!(
            actions(r#"[12, "crafter", "insert", [[1,1,2],[2,1,2]], "minecraft:oak_planks", 1]"#),
            vec![
                CrafterAction::Insert { pos: [1, 1, 2], stack: planks.clone() },
                CrafterAction::Insert { pos: [2, 1, 2], stack: planks.clone() },
            ]
        );
        assert_eq!(
            actions(r#"[1, "crafter", "stack", [[3,2,5]], "minecraft:iron_ingot", 4]"#),
            vec![CrafterAction::Stacked { pos: [3, 2, 5], stack: ItemStack::new("minecraft:iron_ingot", 1), count: 4 }]
        );
        assert_eq!(
            actions(r#"[1, "crafter", "result", [[1,1,2]], "minecraft:oak_door", 3]"#),
            vec![CrafterAction::Result { pos: [1, 1, 2], stack: ItemStack::new("minecraft:oak_door", 3) }]
        );
        assert_eq!(
            actions(r#"[1, "crafter", "connect", [[2,2,2],[1,2,2]], "", 0]"#),
            vec![CrafterAction::Connect { first: [2, 2, 2], second: [1, 2, 2] }]
        );
        assert_eq!(actions(r#"[1, "crafter", "check", [[6,3,2]], "", 0]"#), vec![CrafterAction::Check { pos: [6, 3, 2] }]);
        assert_eq!(actions(r#"[1, "crafter", "eject", [[2,2,5]], "", 0]"#), vec![CrafterAction::Eject { pos: [2, 2, 5] }]);
        assert!(decode(&row(r#"[1, "crafter", "shake", [[2,2,5]], "", 0]"#), &mut Vec::new()).is_none());
    }

    #[test]
    fn a_board_row_is_a_text_or_a_dye_for_a_line() {
        let text = decode(
            &row(r#"[5, "board", "text", [[3,2,3]], 1, [["lit","1024 "],["lang","create.generic.unit.stress"]]]"#),
            &mut Vec::new(),
        );
        match text {
            Some((_, Event::Board(pos, 1, BoardEdit::Text(parts)))) => {
                assert_eq!(pos, [3, 2, 3]);
                assert_eq!(
                    parts,
                    vec![
                        ("lit".to_owned(), "1024 ".to_owned()),
                        ("lang".to_owned(), "create.generic.unit.stress".to_owned())
                    ]
                );
            }
            _ => panic!("not a board text"),
        }
        assert!(matches!(
            decode(&row(r#"[5, "board", "dye", [[3,2,3]], 0, "pink"]"#), &mut Vec::new()),
            Some((_, Event::Board(_, 0, BoardEdit::Dye(Some(dye))))) if dye == "pink"
        ));
        assert!(matches!(
            decode(&row(r#"[5, "board", "dye", [[3,2,3]], 0, "white"]"#), &mut Vec::new()),
            Some((_, Event::Board(_, 0, BoardEdit::Dye(None))))
        ));
    }

    fn entries(json: &str) -> Vec<(String, Tag)> {
        nbt_entries(&serde_json::from_str::<Value>(json).unwrap()).unwrap()
    }

    #[test]
    fn a_depots_held_stack_is_what_its_renderer_reads() {
        let held = entries(r#"{"HeldItem": ["held", "minecraft:clay_ball"]}"#);
        let Some((_, Tag::Compound(held))) = held.first() else { panic!("no held item") };
        let stack = crate::util::get(held, "Item").and_then(crate::util::stack_of).unwrap();
        assert_eq!(&*stack.item, "minecraft:clay_ball");
        assert_eq!(crate::util::int(held, "InDirection"), 1);
        assert_eq!(crate::util::int(held, "Angle"), 0);
    }

    #[test]
    fn a_basins_output_stacks_are_a_list_and_remember_when_they_were_set() {
        let items = entries(r#"{"VisualizedItems": ["items", ["minecraft:bricks", "create:brass_ingot"]]}"#);
        let Some((key, tag)) = items.first() else { panic!("no items") };
        let mut stored = Vec::new();
        set_nbt(&mut stored, key, tag, 120);
        let Some((_, Tag::List(list))) = stored.iter().find(|(known, _)| known == "VisualizedItems") else {
            panic!("no list")
        };
        assert_eq!(list.len(), 2);
        assert!(matches!(stored.iter().find(|(known, _)| known == "VisualizedSince"), Some((_, Tag::Long(120)))));
    }

    #[test]
    fn a_tunnels_filters_are_kept_by_side_and_open_the_side_they_sit_on() {
        let mut stored = Vec::new();
        for (json, at) in [
            (r#"{"Filters": ["filters", {"west": "minecraft:copper_ingot"}]}"#, 1),
            (r#"{"Filters": ["filters", {"north": "minecraft:iron_ingot"}]}"#, 2),
            (r#"{"Filters": ["filters", {"west": ""}]}"#, 3),
        ] {
            for (key, tag) in entries(json) {
                set_nbt(&mut stored, &key, &tag, at);
            }
        }
        let Some((_, Tag::Compound(filters))) = stored.iter().find(|(known, _)| known == "Filters") else {
            panic!("no filters")
        };
        assert!(crate::util::get(filters, "west").is_none());
        assert!(crate::util::get(filters, "north").and_then(crate::util::stack_of).is_some());
        let mut nbt = crate::Compound::new();
        for (key, tag) in &stored {
            nbt.insert(key.as_str().into(), tag.clone());
        }
        let open = tunnel_world::dirs_of(&nbt, tunnel_world::SIDES);
        assert!(open.contains(&Dir::West) && open.contains(&Dir::North) && open.len() == 2);
    }

    #[test]
    fn a_vessels_tanks_are_saved_the_way_its_block_saves_them() {
        let mut basin = Vessel::new(VesselKind::Basin);
        basin.apply(&FluidOp::Fill { fluid: "create:chocolate".into(), amount: 750 });
        let Tag::List(tanks) = vessel_tag(&basin) else { panic!("no list") };
        assert_eq!(tanks.len(), 4);
        assert_eq!(tanks[0].get("Fluid"), Some(&Tag::String("create:chocolate".into())));
        assert_eq!(tanks[0].get("Amount"), Some(&Tag::Int(750)));
        assert_eq!(tanks[1].get("Fluid"), Some(&Tag::String("minecraft:empty".into())));
    }
}
