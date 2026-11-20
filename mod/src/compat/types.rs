//! Old API types the new API has no twin for, or has with another shape. Each keeps the OLD public fields so the
//! machine modules compile unchanged; `adapter.rs` and `events.rs` convert at the boundary. Client-only engine types
//! (the HUD batch, screens, world primitives) are inert stand-ins: their hooks have no match in the new API yet
//! (MIGRATION.md, step 2).
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use super::*;

// --- item stacks ---------------------------------------------------------------------------------------

/// The old stack: an item id, a count, damage and the component patch. The new `mod_api::ItemStack` is opaque
/// (an `ItemKey` and a `NonZeroU32` count) and needs the host to name its item, so the modules keep this one and
/// `Adapter` converts through the stack's NBT form (`{id, count, components}`).
#[derive(Clone, Debug, PartialEq)]
pub struct ItemStack {
    pub item: Arc<str>,
    pub count: u32,
    pub damage: u32,
    /// Components beyond count and damage; None when default.
    pub extra: Option<Box<StackData>>,
}

/// The old stack's components, kept as the NEW api's component patch (id → value; `!id` removes).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StackData {
    pub patch: Compound,
    pub enchantments: Vec<(Arc<str>, u32)>,
    pub stored_enchantments: Vec<(Arc<str>, u32)>,
    pub written: Option<WrittenBook>,
    pub custom_name: Option<Tag>,
    pub container: Vec<(u8, ItemStack)>,
}

impl StackData {
    /// One component by id, as saved.
    pub fn other(&self, id: &str) -> Option<&Tag> {
        self.patch.get(&id.into())
    }
    pub fn set_other(&mut self, id: &str, value: Option<Tag>) {
        match value {
            Some(tag) => {
                self.patch.insert(id.into(), tag);
            }
            None => {
                self.patch.remove(&id.into());
            }
        }
    }
    pub fn is_empty(&self) -> bool {
        self.patch.is_empty()
    }
    /// `minecraft:custom_name` as plain text, when it is a plain string.
    pub fn custom_name_plain(&self) -> Option<String> {
        match self.other("minecraft:custom_name")? {
            Tag::String(text) => text.to_string().ok(),
            _ => None,
        }
    }
}

impl ItemStack {
    pub fn new(item: &str, count: u32) -> Self {
        Self { item: Arc::from(item), count, damage: 0, extra: None }
    }
    pub fn data(&self) -> Option<&StackData> {
        self.extra.as_deref()
    }
    pub fn data_mut(&mut self) -> &mut StackData {
        self.extra.get_or_insert_with(Box::default)
    }
    pub fn tidy(&mut self) {
        if self.extra.as_ref().is_some_and(|d| d.is_empty()) {
            self.extra = None;
        }
    }
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
    pub fn with_count(&self, count: u32) -> Self {
        Self { count, ..self.clone() }
    }
    /// The NBT form `{id, count, components}` the new API converts stacks through.
    pub fn to_tag(&self) -> Compound {
        let mut tag = Compound::default();
        tag.insert("id".into(), Tag::String(self.item.as_ref().into()));
        tag.insert("count".into(), Tag::Int(self.count as i32));
        let mut components = self.extra.as_ref().map(|d| d.patch.clone()).unwrap_or_default();
        if self.damage != 0 {
            components.insert("minecraft:damage".into(), Tag::Int(self.damage as i32));
        }
        if !components.is_empty() {
            tag.insert("components".into(), Tag::Compound(components));
        }
        tag
    }
    pub fn from_tag(tag: &Compound) -> Option<Self> {
        let id = match tag.get(&"id".into())? {
            Tag::String(id) => id.to_string().ok()?,
            _ => return None,
        };
        let count = match tag.get(&"count".into()) {
            Some(Tag::Int(n)) => *n as u32,
            Some(Tag::Byte(n)) => *n as u8 as u32,
            _ => 1,
        };
        let mut stack = Self::new(&id, count);
        if let Some(Tag::Compound(components)) = tag.get(&"components".into()) {
            let mut components = components.clone();
            if let Some(Tag::Int(damage)) = components.remove(&"minecraft:damage".into()) {
                stack.damage = damage.max(0) as u32;
            }
            if !components.is_empty() {
                stack.extra = Some(Box::new(StackData { patch: components, ..Default::default() }));
            }
        }
        Some(stack)
    }
}

/// `StackView` of the old API: a stack as the item-model properties read it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StackView {
    pub item: String,
    pub count: i32,
    pub patch: BTreeMap<String, Option<serde_json::Value>>,
    pub bundle_selected: i32,
}

/// The old `item_data` helpers the glue calls: a stack's components as one NBT tag, and back.
pub mod item_data {
    use super::*;

    pub fn encode_components(stack: &ItemStack) -> Option<Tag> {
        let mut components = stack.extra.as_ref().map(|d| d.patch.clone()).unwrap_or_default();
        if stack.damage != 0 {
            components.insert("minecraft:damage".into(), Tag::Int(stack.damage as i32));
        }
        if components.is_empty() { None } else { Some(Tag::Compound(components)) }
    }

    pub fn decode_components(components: Option<&Tag>) -> (u32, Option<Box<StackData>>) {
        let Some(Tag::Compound(components)) = components else {
            return (0, None);
        };
        let mut components = components.clone();
        let damage = match components.remove(&"minecraft:damage".into()) {
            Some(Tag::Int(n)) => n.max(0) as u32,
            _ => 0,
        };
        let extra = (!components.is_empty()).then(|| Box::new(StackData { patch: components, ..Default::default() }));
        (damage, extra)
    }
}

/// `horizon_client::brewing` of the old API.
pub mod brewing {
    /// A potion's colour. The old table is vanilla's; the new API has no read of it (a gap): water's colour.
    pub fn potion_color(_potion: Option<&str>) -> u32 {
        0x385DC6
    }
}

// --- block entities ------------------------------------------------------------------------------------

/// The old mod block entity: what a hook reads and changes. The new API lends a `ModBlockEntityView` whose
/// `id` is a method; `events.rs` builds this from it, hands it to the modules and writes back what changed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModBlockEntity {
    pub id: String,
    pub nbt: Compound,
    pub items: Vec<Option<ItemStack>>,
    pub data: Vec<i32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModContainer {
    pub slots: usize,
    pub faces: [Vec<usize>; 6],
    pub inputs: Vec<usize>,
    pub outputs: Vec<usize>,
    pub menu: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModFluidTanks {
    pub tanks: Vec<i32>,
    pub faces: [Vec<usize>; 6],
    pub inputs: Vec<usize>,
    pub outputs: Vec<usize>,
}

/// The old block-entity draw handle. It owns nothing: it wraps the new API's `BlockEntityDrawContext` for one
/// `block_entity_submit` call and keeps the old readable fields.
pub struct BlockEntityDraw {
    pub partial: f32,
    pub game_time: i64,
    pub light: u32,
    // The old handle had no lifetime, so this holds the borrowed context as a pointer. A `BlockEntityDraw` is made
    // by `events.rs` for one `block_entity_submit` call and dropped before it returns; nothing keeps it.
    pub(crate) ctx: std::ptr::NonNull<dyn BlockEntityDrawContext>,
    /// A part turns in this draw (`spin` was called), so `still` stops it with a zero-rate spin.
    spun: bool,
}

impl BlockEntityDraw {
    /// # Safety
    /// The result must not outlive the borrow of `ctx`.
    pub(crate) unsafe fn new(ctx: &mut dyn BlockEntityDrawContext) -> Self {
        let (partial, game_time, light) = (ctx.partial(), ctx.game_time(), ctx.light());
        // SAFETY: only the trait object's lifetime bound is erased; the caller keeps the borrow alive.
        let ctx: &mut (dyn BlockEntityDrawContext + 'static) = unsafe { std::mem::transmute(ctx) };
        Self { partial, game_time, light, ctx: std::ptr::NonNull::from(ctx), spun: false }
    }
    pub(crate) fn ctx(&mut self) -> &mut dyn BlockEntityDrawContext {
        // SAFETY: see `new`.
        unsafe { self.ctx.as_mut() }
    }
    pub fn push(&mut self) {
        self.ctx().push();
    }
    pub fn pop(&mut self) {
        self.ctx().pop();
    }
    pub fn translate(&mut self, by: [f32; 3]) {
        self.ctx().translate(by);
    }
    pub fn rotate_about(&mut self, axis: [f32; 3], degrees: f32, pivot: [f32; 3]) {
        self.ctx().rotate_about(axis, degrees, pivot);
    }
    pub fn scale(&mut self, by: [f32; 3]) {
        self.ctx().scale(by);
    }
    pub fn keep(&mut self) {
        self.ctx().keep();
    }
    /// What is drawn next stands still. Once something has turned in this draw, a zero-rate spin
    /// does it instead of the game's `still`: on 0b14f8a a model drawn after `still` is deferred
    /// and baked in later (`bake_recorded`), which moves the `still` marker that shares its vertex
    /// index past the model, so the model turns with the part before it (the press head and the
    /// mixer pole swung around with the shaft and the cog). A model drawn while a spin is active is
    /// not deferred, so it lands in order.
    pub fn still(&mut self) {
        if self.spun {
            self.ctx().spin([0.0, 1.0, 0.0], [0.0; 3], 0.0, 0.0);
        } else {
            self.ctx().still();
        }
    }
    pub fn spin(&mut self, axis: [f32; 3], pivot: [f32; 3], degrees_per_tick: f32, phase: f32) {
        self.spun = true;
        self.ctx().spin(axis, pivot, degrees_per_tick, phase);
    }
    pub fn set_light(&mut self, light: u32) {
        self.light = light;
        self.ctx().set_light(light);
    }
}

// --- plain data ----------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DisplayContext {
    None,
    ThirdPersonLeftHand,
    ThirdPersonRightHand,
    FirstPersonLeftHand,
    FirstPersonRightHand,
    Head,
    Gui,
    Ground,
    Fixed,
    OnShelf,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeFluidStack {
    pub fluid: String,
    pub amount: i32,
    pub components: Compound,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeFluidTankView {
    pub tank: usize,
    pub content: Option<NativeFluidStack>,
    pub capacity: i32,
    pub can_fill: bool,
    pub can_drain: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeFluidTank {
    pub tank: usize,
    pub fluid: Option<String>,
    pub amount: i32,
    pub capacity: i32,
    pub can_fill: bool,
    pub can_drain: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeFluidAction {
    Simulate,
    Execute,
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum NativeFluidDrain {
    Any(i32),
    Exact(NativeFluidStack),
}

#[derive(Clone, Copy, Debug)]
pub struct NativeFluidRenderOptions {
    pub argb: Option<u32>,
    pub render_bottom: bool,
    pub uv_shrink_ratio: f32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum NativeBlockReadError {
    InactiveContext,
    NotServer,
    TagsUnavailable,
    UnavailableDimension,
    OutOfBounds,
    Unloaded,
    InvalidState,
}

#[derive(Debug)]
pub enum NativeComponentError {
    UnknownType,
    InvalidValue(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockId(pub u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct NativeLifecycleContext;

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct NativeCommand {
    pub usage: String,
    pub level: u8,
    pub permission: Option<String>,
    pub replace_vanilla: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeEffect {
    pub id: String,
    pub amplifier: u8,
    pub duration: i32,
    pub ambient: bool,
    pub particles: bool,
}

#[derive(Clone, Debug)]
pub struct ClientResources {
    pub root: PathBuf,
    /// The host of the hook that asked: the new API reads a resource through it (`Host::resource`). The old handle had
    /// no lifetime, so this is a pointer; a `ClientResources` is made and read inside one hook call, like
    /// `BlockEntityDraw`.
    pub(crate) host: Option<std::ptr::NonNull<dyn mod_api::Host>>,
}

impl ClientResources {
    /// # Safety
    /// The handle must not outlive the borrow of `host` (it is used within the hook that made it).
    pub(crate) unsafe fn of(host: &dyn mod_api::Host) -> Self {
        // SAFETY: lifetime erased as in `BlockEntityDraw::new`; see above.
        let host: &(dyn mod_api::Host + 'static) = unsafe { std::mem::transmute(host) };
        Self { root: PathBuf::new(), host: Some(std::ptr::NonNull::from(host)) }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct NativeSceneBlock<'a> {
    pub pos: [i32; 3],
    pub state: BlockStateId,
    pub entity: Option<&'a ModBlockEntity>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeSceneView {
    pub center: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub scale: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NativeModelMaterial {
    Model,
    Solid,
    Cutout,
    CutoutMipped,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NativeModelShade {
    Model,
    Off,
    Entity,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeModelUvShift<'a> {
    pub source_sprite: &'a str,
    pub target_sprite: &'a str,
    pub scale: [f32; 2],
    pub offset: [f32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeModelRenderOptions<'a> {
    pub material: NativeModelMaterial,
    pub shade: NativeModelShade,
    pub uv_shift: Option<NativeModelUvShift<'a>>,
    pub uv_shrink_ratio: f32,
    pub tint: u32,
}

#[derive(Clone, Debug)]
pub struct NativeItemEntity {
    pub id: u32,
    pub position: [f64; 3],
    pub stack: ItemStack,
    pub pickup_delay: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativePlayer {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub yaw: f32,
    pub pitch: f32,
    pub dimension: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CustomRecipe {
    pub id: String,
    pub json: Arc<serde_json::Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeInventoryKind;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeInventoryError {
    InsidePlayerTurn,
    NoServer,
    LoggingIn,
    Dead,
    Online,
    NoWorld,
    NoSavedPlayer,
    HasFile,
    NotOnline,
    BothAway,
    SameInventory,
    StackChanged,
    NoRoom,
    WrongSize { want: usize, got: usize },
    NoSuchHotbarSlot,
    NoSuchSlot,
    SlotNamedTwice,
    File(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NativeUseTarget {
    Block { pos: [i32; 3], face: Direction, point: [f64; 3] },
    Item,
    Entity { id: i32, location: [f64; 3] },
}

#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct NativeFakeUse {
    pub consumed: bool,
    pub held: Option<ItemStack>,
    pub other: Vec<ItemStack>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeHit {
    pub block_x: i32,
    pub block_y: i32,
    pub block_z: i32,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub block: String,
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct NativeServerPlayer {
    pub uuid: u128,
    pub name: String,
    pub permission: u8,
    pub dimension: String,
    pub position: [f64; 3],
    pub game_mode: i32,
    pub health: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub entity_id: i32,
    pub address: String,
    pub latency: i32,
    pub protocol: i32,
    pub xp_level: i32,
    pub xp_progress: f32,
    pub on_ground: bool,
    pub flying: bool,
    pub may_fly: bool,
    pub invulnerable: bool,
    pub walk_speed: f32,
    pub fly_speed: f32,
    pub sneaking: bool,
    pub eye_height: f32,
    pub selected_slot: u8,
    pub nickname: Option<String>,
    pub hidden: Option<u8>,
    pub fake: bool,
    pub using_item: bool,
    pub blocking: bool,
    pub sprinting: bool,
    pub gliding: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NativeScreenKind {
    #[default]
    None,
    Other,
}

#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct NativeClientState {
    pub window_id: u64,
    pub focused: bool,
    pub gui_hidden: bool,
    pub gui_size: [f32; 2],
    pub server_address: Option<String>,
    pub logged_in: bool,
    pub world_partial_tick: f64,
    pub eye_position: Option<[f64; 3]>,
    pub yaw: f32,
    pub pitch: f32,
    pub menu_blur: f32,
    pub screen: NativeScreenKind,
    pub debug_screen: bool,
    pub sleeping_pos: Option<[i32; 3]>,
}

#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct NativeScreenView<'a> {
    pub id: &'static str,
    pub menu: Option<&'a str>,
    pub title: Option<&'a Text>,
    pub rect: [f32; 4],
    pub slots: usize,
    pub mouse: [f32; 2],
    pub partial_tick: f32,
}

#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct NativeLootDrops<'a> {
    pub table: &'a str,
    pub dimension: &'a str,
    pub origin: [f64; 3],
    pub block: Option<([i32; 3], BlockStateId)>,
    pub entity: Option<(u32, &'a str)>,
    pub player: Option<u128>,
    pub tool: Option<&'a ItemStack>,
    pub explosion: Option<f32>,
    pub used: bool,
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct BlockStateFacts {
    pub name: Arc<str>,
    pub air: bool,
    pub invisible: bool,
    pub fluid: FluidState,
    pub waterlogged: bool,
    pub emission: u8,
    pub outline: Arc<[[f64; 6]]>,
    pub map_color: Option<u32>,
    pub collides: bool,
}

/// What the old `Host::block_states()` handed back: the registry of states. Unavailable here (a gap).
#[derive(Clone, Copy, Debug, Default)]
pub struct BlockStates;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UseAnimation {
    None,
    Eat,
    Drink,
    Block,
    Bow,
    Trident,
    Crossbow,
    Spyglass,
    TootHorn,
    Brush,
    Bundle,
    Spear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyCode(pub u32);

// --- client-only stand-ins (their hooks have no match yet) ----------------------------------------------

/// The old HUD batch. It owns nothing: it wraps the new API's `DrawContext` for one hook call (`render_gui_post`, a
/// screen hook, a tooltip hook) and keeps the old readable size. Only the `Host` drawing methods (`fill`, `text`,
/// `item`, ...) draw on it, through `draw()`.
#[derive(Default)]
pub struct HudBatch {
    pub scale: f32,
    pub width: f32,
    pub height: f32,
    /// Unused: the old batch collected vertices, this one draws at once. `len()` of it is still a mark for
    /// `scale_since`.
    pub vertices: Vec<[f32; 8]>,
    // As `BlockEntityDraw`: the old handle had no lifetime, so the borrowed context is held as a pointer. A batch is
    // made by `compat::client` for one hook call and dropped before it returns.
    draw: Option<std::ptr::NonNull<dyn DrawContext>>,
}

/// The GUI's size in GUI pixels, as the last HUD, screen or tooltip draw context gave it. The new API has no host
/// call for it outside those hooks; the old `client_state().gui_size` reads this.
static GUI_SIZE: std::sync::Mutex<[f32; 2]> = std::sync::Mutex::new([0.0, 0.0]);

fn set_gui_size(size: [f32; 2]) {
    *GUI_SIZE.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = size;
}

pub(crate) fn gui_size() -> [f32; 2] {
    *GUI_SIZE.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl HudBatch {
    /// # Safety
    /// The result must not outlive the borrow of `draw`.
    pub(crate) unsafe fn new(draw: &mut dyn DrawContext) -> Self {
        let [width, height] = draw.size();
        set_gui_size([width, height]);
        // SAFETY: only the trait object's lifetime bound is erased; the caller keeps the borrow alive.
        let draw: &mut (dyn DrawContext + 'static) = unsafe { std::mem::transmute(draw) };
        Self { scale: 1.0, width, height, vertices: Vec::new(), draw: Some(std::ptr::NonNull::from(draw)) }
    }
    /// The context behind this batch; None for a default (empty) batch, which draws nothing.
    pub(crate) fn draw(&mut self) -> Option<&mut dyn DrawContext> {
        // SAFETY: see `new`.
        self.draw.map(|mut draw| unsafe { draw.as_mut() })
    }
    pub fn scale_since(&mut self, _mark: usize, _x: f32, _y: f32, _sx: f32, _sy: f32) {}
}

#[derive(Default)]
pub struct Corners {
    width: f32,
    height: f32,
    used: [f32; 4],
}

/// The old per-frame world drawing handle. It collects lines and quads for one `render_level_stage` call
/// (`compat::client::render_level_stage` sends them to the `WorldDrawContext`). World text has no counterpart: dropped.
#[derive(Default)]
pub struct WorldPrimitives {
    pub(crate) lines: Vec<mod_api::WorldLine>,
    pub(crate) quads: Vec<mod_api::WorldQuad>,
}

// --- block states --------------------------------------------------------------------------------------

use std::sync::Mutex;

/// The old numeric block state. The new API's `BlockState` is a process-local `(BlockKey, ordinal)` pair with no
/// number, and the modules keep states in caches and compare them as numbers (`BlockStateId.0`), so this interns
/// each `BlockState` the first time it is seen and hands out its index. An id is stable for the process only, as
/// the old one was for a game run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockStateId(pub u32);

static STATES: std::sync::LazyLock<Mutex<(Vec<mod_api::BlockState>, std::collections::HashMap<mod_api::BlockState, u32>)>> =
    std::sync::LazyLock::new(Mutex::default);

impl From<mod_api::BlockState> for BlockStateId {
    fn from(state: mod_api::BlockState) -> Self {
        let mut table = STATES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(&id) = table.1.get(&state) {
            return Self(id);
        }
        let id = table.0.len() as u32;
        table.0.push(state);
        table.1.insert(state, id);
        Self(id)
    }
}

impl BlockStateId {
    /// The new API's state this id stands for.
    pub fn state(self) -> Option<mod_api::BlockState> {
        STATES.lock().unwrap_or_else(std::sync::PoisonError::into_inner).0.get(self.0 as usize).copied()
    }
}

/// The old read-only level a placement or signal hook is lent.
pub trait LevelHost {
    fn block_state(&self, pos: [i32; 3]) -> BlockStateId;
    fn is_client_side(&self) -> bool;
    fn state_with_property(&self, state: BlockStateId, key: &str, value: &str) -> Option<BlockStateId>;
}
pub trait LevelRandomHost: LevelHost {}
pub trait LevelTicksHost: LevelHost {
    fn schedule_block_tick(&mut self, pos: [i32; 3], block: &str, delay: i32) -> bool;
    fn schedule_fluid_tick(&mut self, pos: [i32; 3], fluid: &str, delay: i32) -> bool;
}
pub trait LevelWriteHost: LevelTicksHost + LevelRandomHost {}

/// A point as the old API passed it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// The old placement context (the new one names the held item by an `ItemKey`, which needs the host).
#[derive(Clone, Debug)]
pub struct PlacementContext {
    pub pos: BlockPos,
    pub hit_pos: BlockPos,
    pub face: Direction,
    pub hit: Vec3,
    pub inside: bool,
    pub yaw: f32,
    pub pitch: f32,
    pub sneaking: bool,
    pub item: String,
    pub replace_clicked: bool,
}

// --- small old-API shapes ------------------------------------------------------------------------------

impl Default for NativeBlockReadError {
    fn default() -> Self {
        Self::InactiveContext
    }
}
impl Default for NativeComponentError {
    fn default() -> Self {
        Self::UnknownType
    }
}
impl Default for NativeInventoryError {
    fn default() -> Self {
        Self::NoServer
    }
}
impl Default for ClientResources {
    fn default() -> Self {
        Self { root: PathBuf::new(), host: None }
    }
}
impl ClientResources {
    /// Reads one file of the client's resources, `assets/<namespace>/<rest>`, through `Host::resource`
    /// (`<namespace>:<rest>`), the winning file of the selected packs.
    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        let host = unsafe { self.host?.as_ref() };
        let rest = path.strip_prefix("assets/")?;
        let (namespace, name) = rest.split_once('/')?;
        let id = Identifier::parse(&format!("{namespace}:{name}")).ok()?;
        host.resource(mod_api::PackType::ClientResources, &id)
    }
}

impl NativeInventoryKind {
    /// The player's own inventory (the old kind enum's only case in use).
    #[allow(non_upper_case_globals)]
    pub const Player: Self = Self;
}

impl NativeCommand {
    pub fn new(usage: &str) -> Self {
        Self { usage: usage.to_owned(), level: 0, permission: None, replace_vanilla: false }
    }
}

impl NativeScreenKind {
    #[allow(non_upper_case_globals)]
    pub const Playing: Self = Self::None;
}

impl BlockStates {
    pub fn iter(&self) -> std::iter::Empty<(BlockStateId, &BlockStateFacts)> {
        std::iter::empty()
    }
}

/// `StackData::written`: a written book's header.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrittenBook {
    pub author: String,
    pub generation: u8,
}

/// `Text` constructors of the old API that the new `Text` doesn't have; they build the new value.
pub trait TextExt {
    fn with_color(self, rgb: u32) -> Text;
    fn colored(text: impl Into<String>, rgb: u32) -> Text;
    fn translate_with_fallback(key: &str, fallback: &str, args: &[Text]) -> Text;
}
impl TextExt for Text {
    fn with_color(self, _rgb: u32) -> Text {
        self
    }
    fn colored(text: impl Into<String>, rgb: u32) -> Text {
        // The new API doesn't export its `TextStyle`, so a colour can't be set from here (gap): plain text.
        let _ = rgb;
        Text::literal(text.into())
    }
    fn translate_with_fallback(key: &str, fallback: &str, args: &[Text]) -> Text {
        let _ = (key, args);
        // The new API has no translation fallback constructor: the English text stands in (gap).
        Text::literal(fallback.to_owned())
    }
}

// --- more shapes ---------------------------------------------------------------------------------------

/// The old ingredient: a candidate set of item ids; `test` matches a stack by item (the new API's predicates that
/// look at components need the host, a gap).
#[derive(Clone, Debug)]
pub struct Ingredient(pub mod_api::Ingredient);

impl std::ops::Deref for Ingredient {
    type Target = mod_api::Ingredient;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Ingredient {
    pub fn test(&self, stack: &ItemStack) -> bool {
        self.0.items.contains(&*stack.item)
    }
}

/// The old particle types the glue names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ParticleType {
    LargeSmoke,
    HappyVillager,
    Dust,
}

/// The old particle options: only what the glue builds (`simple`, `dust`).
#[derive(Clone, Debug)]
pub struct ParticleOptions {
    pub ty: ParticleType,
    pub color: [f32; 4],
    pub scale: f32,
}

impl ParticleOptions {
    pub fn simple(ty: ParticleType) -> Self {
        Self { ty, color: [1.0; 4], scale: 1.0 }
    }
    pub fn dust(rgb: [f32; 3], scale: f32) -> Self {
        Self { ty: ParticleType::Dust, color: [rgb[0], rgb[1], rgb[2], 1.0], scale }
    }
}

impl KeyCode {
    #[allow(non_upper_case_globals)]
    pub const KeyW: Self = Self(0x57);
}

impl WorldPrimitives {
    pub fn line(&mut self, a: [f64; 3], b: [f64; 3], argb: u32, width: f32) {
        self.lines.push(mod_api::WorldLine::new(a, b, argb, width, mod_api::WorldDepth::Tested));
    }
    pub fn box_outline(&mut self, min: [f64; 3], max: [f64; 3], argb: u32, width: f32) {
        let corner = |i: usize| [
            if i & 1 == 0 { min[0] } else { max[0] },
            if i & 2 == 0 { min[1] } else { max[1] },
            if i & 4 == 0 { min[2] } else { max[2] },
        ];
        // The 12 edges: corners that differ in exactly one of the three bits.
        for a in 0..8usize {
            for bit in [1usize, 2, 4] {
                let b = a | bit;
                if b != a {
                    self.line(corner(a), corner(b), argb, width);
                }
            }
        }
    }
    pub fn filled_box(&mut self, min: [f64; 3], max: [f64; 3], argb: u32) {
        let [x0, y0, z0] = min;
        let [x1, y1, z1] = max;
        let faces = [
            [[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]],
            [[x0, y1, z0], [x0, y1, z1], [x1, y1, z1], [x1, y1, z0]],
            [[x0, y0, z0], [x0, y1, z0], [x1, y1, z0], [x1, y0, z0]],
            [[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]],
            [[x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0]],
            [[x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [x1, y0, z1]],
        ];
        for corners in faces {
            self.quads.push(mod_api::WorldQuad::new(corners, argb, mod_api::WorldDepth::Tested));
        }
    }
    /// No text in the world on the new API (`WorldDrawContext` draws lines and quads): dropped.
    pub fn text(&mut self, _at: [f64; 3], _text: impl AsRef<str>, _argb: u32, _scale: f32) {}
}

impl ItemStack {
    pub fn enchantments(&self) -> &[(Arc<str>, u32)] {
        self.data().map_or(&[], |d| &d.enchantments)
    }
    pub fn stored_enchantments(&self) -> &[(Arc<str>, u32)] {
        self.data().map_or(&[], |d| &d.stored_enchantments)
    }
}

impl ModBlockEntity {
    /// The new API's revision counter is private; a compat copy is always "new".
    pub fn revision(&self) -> u64 {
        0
    }
}

impl Default for NativeModelRenderOptions<'_> {
    fn default() -> Self {
        Self {
            material: NativeModelMaterial::Model,
            shade: NativeModelShade::Model,
            uv_shift: None,
            uv_shrink_ratio: 0.0,
            tint: 0xFF_FFFF,
        }
    }
}

/// A block's state as `Name[prop=value,...]` text, as the old `LevelRef::block_state` answered; the new one answers
/// a `BlockState`, which `host.block_state_string` names.
pub fn state_text(host: &dyn Host, level: LevelRef, pos: [i32; 3]) -> Option<String> {
    let base = host.new_host();
    let state = level.block_state(base, pos)?;
    base.block_state_string(state)
}

/// `CubeFace` with the old public fields (the new one is `#[non_exhaustive]`).
#[derive(Clone, Copy, Debug)]
pub struct CubeFace<'a> {
    pub sprite: &'a str,
    pub uv: Option<[f32; 4]>,
}
impl<'a> CubeFace<'a> {
    pub const fn new(sprite: &'a str) -> Self {
        Self { sprite, uv: None }
    }
    pub const fn with_uv(self, uv: [f32; 4]) -> Self {
        Self { uv: Some(uv), ..self }
    }
}

/// A parsed SNBT tree as the game's tag.
fn tag_of(value: create_kinetics::rules::snbt::Value) -> Tag {
    use create_kinetics::rules::snbt::Value;
    match value {
        Value::Byte(v) => Tag::Byte(v),
        Value::Short(v) => Tag::Short(v),
        Value::Int(v) => Tag::Int(v),
        Value::Long(v) => Tag::Long(v),
        Value::Float(v) => Tag::Float(v.to_bits()),
        Value::Double(v) => Tag::Double(v.to_bits()),
        Value::Str(v) => Tag::String(v.as_str().into()),
        Value::ByteArray(v) => Tag::ByteArray(v.into_iter().map(|b| b as u8).collect()),
        Value::IntArray(v) => Tag::IntArray(v),
        Value::LongArray(v) => Tag::LongArray(v),
        Value::List(v) => Tag::List(v.into_iter().map(tag_of).collect()),
        Value::Compound(fields) => {
            let mut out = Compound::new();
            for (key, value) in fields {
                out.insert(key.as_str().into(), tag_of(value));
            }
            Tag::Compound(out)
        }
    }
}

/// `parse_snbt` of the old API: text NBT and the bytes it read. The new API hands a mod no parser, so
/// `create_kinetics::rules::snbt` reads it.
pub fn parse_snbt(text: &str) -> Result<(Tag, usize), String> {
    let (value, used) = create_kinetics::rules::snbt::parse(text)?;
    Ok((tag_of(value), used))
}
