//! New events in, old events out: what the Parts and the global hooks of `parts.rs` / `lib.rs` call.
//!
//! Each function builds the OLD event from the new one, runs it through `crate::create()` (the old dispatcher that
//! hands it to every module) and writes the answer back. Where the new event holds a handle the old one held a
//! string or a number, the conversion resolves it; what cannot be resolved is a neutral value and a `gap`.
use super::*;

fn create() -> &'static dyn Hooks {
    crate::create()
}

/// The old air state, for a position the new level has no state for.
fn air(level: &dyn mod_api::LevelHost) -> BlockStateId {
    level.block_state_id("minecraft:air").map(Into::into).unwrap_or(BlockStateId(0))
}

// --- levels ----------------------------------------------------------------------------------------------

/// The old read-only level on the new one.
pub struct Level<'a> {
    pub inner: &'a dyn mod_api::LevelHost,
}

impl LevelHost for Level<'_> {
    fn block_state(&self, pos: [i32; 3]) -> BlockStateId {
        self.inner.block_state(pos).map_or_else(|| air(self.inner), Into::into)
    }
    fn is_client_side(&self) -> bool {
        false /* the new LevelHost can't say which side it is (gap) */
    }
    fn state_with_property(&self, state: BlockStateId, key: &str, value: &str) -> Option<BlockStateId> {
        self.inner.state_with_property(state.state()?, key, value).map(Into::into)
    }
}

/// The old tick-scheduling level on the new one.
pub struct LevelTicks<'a> {
    pub inner: &'a mut dyn mod_api::LevelTicksHost,
}

impl LevelHost for LevelTicks<'_> {
    fn block_state(&self, pos: [i32; 3]) -> BlockStateId {
        let level: &dyn mod_api::LevelHost = &*self.inner;
        level.block_state(pos).map_or_else(|| air(level), Into::into)
    }
    fn is_client_side(&self) -> bool {
        let level: &dyn mod_api::LevelHost = &*self.inner;
        false /* gap: see Level::is_client_side */
    }
    fn state_with_property(&self, state: BlockStateId, key: &str, value: &str) -> Option<BlockStateId> {
        let level: &dyn mod_api::LevelHost = &*self.inner;
        level.state_with_property(state.state()?, key, value).map(Into::into)
    }
}

impl LevelTicksHost for LevelTicks<'_> {
    fn schedule_block_tick(&mut self, pos: [i32; 3], block: &str, delay: i32) -> bool {
        let level: &dyn mod_api::LevelHost = &*self.inner;
        let Some(key) = level.block_state_id(block).map(|state| state.block()) else { return false };
        self.inner.schedule_block_tick(pos, key, delay)
    }
    fn schedule_fluid_tick(&mut self, pos: [i32; 3], fluid: &str, delay: i32) -> bool {
        self.inner.schedule_fluid_tick(pos, fluid, delay)
    }
}

impl LevelRandomHost for LevelTicks<'_> {}
impl LevelWriteHost for LevelTicks<'_> {}

// --- stacks across the host ---------------------------------------------------------------------------

/// The old stack for a new one; the item's name needs the host.
pub fn stack_old(host: &dyn mod_api::Host, stack: &mod_api::ItemStack) -> Option<ItemStack> {
    ItemStack::from_tag(&stack.to_nbt(host)?)
}

/// The new stack for an old one.
pub fn stack_new(host: &dyn mod_api::Host, stack: &ItemStack) -> Option<mod_api::ItemStack> {
    if stack.count == 0 {
        return None;
    }
    mod_api::ItemStack::from_nbt(host, &stack.to_tag())
}

pub fn items_old(host: &dyn mod_api::Host, slots: &[Option<mod_api::ItemStack>]) -> Vec<Option<ItemStack>> {
    slots.iter().map(|slot| slot.as_ref().and_then(|stack| stack_old(host, stack))).collect()
}

pub fn items_new(host: &dyn mod_api::Host, slots: &[Option<ItemStack>]) -> Vec<Option<mod_api::ItemStack>> {
    slots.iter().map(|slot| slot.as_ref().and_then(|stack| stack_new(host, stack))).collect()
}

/// The item slots of each ticking block entity as old stacks, as the last tick or write left them. The hooks that
/// build a client's update tag have no host to name an item with, so they read this copy (a gap otherwise).
static SLOT_CACHE: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<[i32; 3], Vec<Option<ItemStack>>>>> =
    std::sync::LazyLock::new(Default::default);

pub fn cache_slots(pos: [i32; 3], items: &[Option<ItemStack>]) {
    let mut cache = SLOT_CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if items.iter().all(Option::is_none) {
        cache.remove(&pos);
    } else {
        cache.insert(pos, items.to_vec());
    }
}

fn cached_slots(pos: [i32; 3]) -> Vec<Option<ItemStack>> {
    SLOT_CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&pos).cloned().unwrap_or_default()
}

// --- block parts ---------------------------------------------------------------------------------------

pub fn state_for_placement(
    id: &str,
    level: &dyn mod_api::LevelHost,
    event: &mut mod_api::StateForPlacementEvent<'_>,
) -> Option<Option<mod_api::BlockState>> {
    let c = event.context;
    let context = PlacementContext {
        pos: c.pos,
        hit_pos: c.hit_pos,
        face: c.face,
        hit: Vec3 { x: c.hit[0], y: c.hit[1], z: c.hit[2] },
        inside: c.inside,
        yaw: c.yaw,
        pitch: c.pitch,
        sneaking: c.sneaking,
        // The Part belongs to one block, and the item that places it is the block's own.
        item: id.to_owned(),
        replace_clicked: c.replace_clicked,
    };
    let mut old = StateForPlacementEvent {
        dimension: event.dimension,
        state: event.state.into(),
        context: &context,
        fluid: event.fluid,
    };
    let level = Level { inner: level };
    create().state_for_placement(&level, &mut old).map(|answer| answer.and_then(BlockStateId::state))
}

pub fn can_survive(level: &dyn mod_api::LevelHost, event: &mut mod_api::CanSurviveEvent) -> Option<bool> {
    let mut old = CanSurviveEvent { dimension: event.dimension, pos: event.pos, state: event.state.into() };
    create().can_survive(&Level { inner: level }, &mut old)
}

pub fn update_shape(
    level: &mut dyn mod_api::LevelTicksHost,
    event: &mut mod_api::UpdateShapeEvent,
) -> Option<mod_api::BlockState> {
    let mut old = UpdateShapeEvent {
        dimension: event.dimension,
        pos: event.pos,
        state: event.state.into(),
        direction: event.direction,
        neighbour_pos: event.neighbour_pos,
        neighbour: event.neighbour.into(),
    };
    create().update_shape(&mut LevelTicks { inner: level }, &mut old).and_then(BlockStateId::state)
}

pub fn neighbor_changed(level: &mut dyn mod_api::LevelTicksHost, event: &mut mod_api::NeighborChangedEvent) {
    // The old event named the changed block by its registry number; the new one by a `BlockKey` with no number (gap).
    gap("NeighborChangedEvent::changed");
    let mut old = NeighborChangedEvent {
        dimension: event.dimension,
        pos: event.pos,
        state: event.state.into(),
        changed: BlockId(0),
        moved: event.moved,
    };
    create().neighbor_changed(&mut LevelTicks { inner: level }, &mut old);
}

pub fn tick_block(level: &mut dyn mod_api::LevelWriteHost, event: &mut mod_api::TickBlockEvent) -> mod_api::Flow {
    let mut old = TickBlockEvent { dimension: event.dimension, pos: event.pos, state: event.state.into() };
    // `LevelWriteHost` is a marker in the compat layer; the modules read through `LevelHost`.
    struct Write<'a>(&'a mut dyn mod_api::LevelWriteHost);
    impl LevelHost for Write<'_> {
        fn block_state(&self, pos: [i32; 3]) -> BlockStateId {
            let level: &dyn mod_api::LevelHost = &*self.0;
            level.block_state(pos).map_or_else(|| air(level), Into::into)
        }
        fn is_client_side(&self) -> bool {
            let level: &dyn mod_api::LevelHost = &*self.0;
            false /* gap: see Level::is_client_side */
        }
        fn state_with_property(&self, state: BlockStateId, key: &str, value: &str) -> Option<BlockStateId> {
            let level: &dyn mod_api::LevelHost = &*self.0;
            level.state_with_property(state.state()?, key, value).map(Into::into)
        }
    }
    impl LevelTicksHost for Write<'_> {
        fn schedule_block_tick(&mut self, pos: [i32; 3], block: &str, delay: i32) -> bool {
            let level: &dyn mod_api::LevelHost = &*self.0;
            let Some(key) = level.block_state_id(block).map(|state| state.block()) else { return false };
            self.0.schedule_block_tick(pos, key, delay)
        }
        fn schedule_fluid_tick(&mut self, pos: [i32; 3], fluid: &str, delay: i32) -> bool {
            self.0.schedule_fluid_tick(pos, fluid, delay)
        }
    }
    impl LevelRandomHost for Write<'_> {}
    impl LevelWriteHost for Write<'_> {}
    create().tick_block(&mut Write(level), &mut old)
}

pub fn block_signal(level: &dyn mod_api::LevelHost, event: &mut mod_api::BlockSignalEvent) {
    let mut old = BlockSignalEvent {
        dimension: event.dimension,
        pos: event.pos,
        state: event.state.into(),
        face: event.face,
        direct: event.direct,
        signal: event.signal,
    };
    create().block_signal(&Level { inner: level }, &mut old);
    event.signal = old.signal;
}

pub fn is_signal_source(level: &dyn mod_api::LevelHost, event: &mut mod_api::IsSignalSourceEvent) {
    let mut old = IsSignalSourceEvent { state: event.state.into(), source: event.source };
    create().is_signal_source(&Level { inner: level }, &mut old);
    event.source = old.source;
}

pub fn has_analog_output(level: &dyn mod_api::LevelHost, event: &mut mod_api::HasAnalogOutputEvent) {
    let mut old = HasAnalogOutputEvent { state: event.state.into(), output: event.output };
    create().has_analog_output(&Level { inner: level }, &mut old);
    event.output = old.output;
}

pub fn analog_output(level: &dyn mod_api::LevelHost, event: &mut mod_api::AnalogOutputEvent) {
    let mut old = AnalogOutputEvent {
        dimension: event.dimension,
        pos: event.pos,
        state: event.state.into(),
        face: event.face,
        output: event.output,
    };
    create().analog_output(&Level { inner: level }, &mut old);
    event.output = old.output;
}

pub fn use_item_on(
    host: &mut dyn ServerHost,
    event: &mut mod_api::UseItemOnEvent<'_>,
) -> Option<mod_api::InteractionResult> {
    let click = event.click;
    let Some(stack) = click.stack.and_then(|stack| stack_old(&*host, stack)) else { return None };
    let mut old = UseItemOnEvent { stack: &stack, pos: click.pos, face: click.hit.face };
    let mut adapter = Adapter::Server(host);
    match create().use_item_on(&mut adapter, &mut old) {
        Flow::Continue => None,
        _ => Some(mod_api::InteractionResult::Success),
    }
}

// --- block entity parts --------------------------------------------------------------------------------

pub fn block_entity(block: &str, state: mod_api::BlockState) -> Option<Compound> {
    let _ = state;
    let mut adapter = Adapter::None;
    let mut old = BlockEntityEvent { block };
    // The old hook answered `(entity id, initial data)`; the new type's id is declared at registration.
    create().block_entity(&mut adapter, &mut old).map(|(_, data)| data)
}

pub fn block_entity_ticks(host: &dyn mod_api::Host, block: &str) -> bool {
    let mut adapter = Adapter::Base(host);
    let mut old = BlockEntityTicksEvent { block, ticks: false };
    create().block_entity_ticks(&mut adapter, &mut old);
    old.ticks
}

pub fn block_entity_tick(host: &mut dyn ServerHost, event: &mut mod_api::BlockEntityTickEvent<'_>) {
    let mut old = ModBlockEntity {
        id: event.entity.id().to_owned(),
        nbt: event.entity.nbt.clone(),
        items: items_old(&*host, &event.entity.items),
        data: event.entity.data.clone(),
    };
    let before = old.clone();
    {
        let mut adapter = Adapter::Server(&mut *host);
        let mut e = BlockEntityTickEvent { pos: event.pos, entity: &mut old };
        create().block_entity_tick(&mut adapter, &mut e);
    }
    if old == before {
        // Nothing changed: no revision, no save, no comparator update.
        *event.reply = mod_api::BlockEntityTickReply::Unchanged;
        cache_slots(event.pos, &old.items);
        return;
    }
    cache_slots(event.pos, &old.items);
    if old.nbt != before.nbt {
        event.entity.nbt = old.nbt;
    }
    if old.items != before.items {
        event.entity.items = items_new(&*host, &old.items);
    }
    if old.data != before.data {
        event.entity.data = old.data;
    }
}

pub fn block_entity_update_tag(event: &mut mod_api::BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
    // No host here (the hook can run on another thread), so the slots can't be named: a gap for modules that
    // read `entity.items` while building their tag.
    let old = ModBlockEntity {
        id: event.entity.id().to_owned(),
        nbt: event.entity.nbt.clone(),
        items: cached_slots(event.pos),
        data: event.entity.data.clone(),
    };
    let mut e = BlockEntityUpdateTagEvent { pos: event.pos, entity: &old };
    create().block_entity_update_tag(&mut e)
}

pub fn block_entity_submit(host: &dyn ClientHost, event: &mut mod_api::BlockEntitySubmitEvent<'_>) {
    let old = ModBlockEntity {
        id: event.entity.id().to_owned(),
        nbt: event.entity.nbt.clone(),
        items: Vec::new(),
        data: event.entity.data.clone(),
    };
    // SAFETY: `draw` is dropped at the end of this function, inside the borrow of `event.draw`.
    let mut draw = unsafe { BlockEntityDraw::new(&mut *event.draw) };
    let mut adapter = Adapter::Client(host);
    let mut e = BlockEntitySubmitEvent {
        pos: event.pos,
        block: event.block,
        state: event.state.into(),
        entity: &old,
        draw: &mut draw,
    };
    create().block_entity_submit(&mut adapter, &mut e);
}

pub fn block_entity_render_bounding_box(
    host: &dyn ClientHost,
    event: &mut mod_api::BlockEntityRenderBoundingBoxEvent<'_>,
) {
    let old = ModBlockEntity {
        id: event.entity.id().to_owned(),
        nbt: event.entity.nbt.clone(),
        items: Vec::new(),
        data: event.entity.data.clone(),
    };
    let mut adapter = Adapter::Client(host);
    let mut e = BlockEntityRenderBoundingBoxEvent {
        pos: event.pos,
        block: event.block,
        state: event.state.into(),
        entity: &old,
        bounds: &mut *event.bounds,
    };
    create().block_entity_render_bounding_box(&mut adapter, &mut e);
}

// --- global hooks --------------------------------------------------------------------------------------

pub fn standalone_models(event: &mut mod_api::StandaloneModelsEvent<'_>) {
    let mut old = StandaloneModelsEvent { models: &mut *event.models };
    create().standalone_models(&mut old);
    // Remember the names: `draw_model` resolves them to handles once the client has baked them.
    *adapter::MODEL_NAMES.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = event.models.clone();
}

pub fn server_tick(host: &mut dyn ServerHost, event: &mut mod_api::ServerTickEvent<'_>) {
    let context = NativeLifecycleContext;
    let mut old = ServerTickEvent { context: &context, phase: event.phase };
    let mut adapter = Adapter::Server(host);
    create().server_tick(&mut adapter, &mut old);
}

pub fn level_tick_post(host: HostSide<'_>) {
    match host {
        HostSide::Server(host) => {
            let dimension = host.dimension().to_owned();
            let mut old = LevelTickPostEvent { dimension: &dimension };
            create().level_tick_post(&mut Adapter::Server(host), &mut old);
        }
        HostSide::Client(host) => {
            let dimension = host.dimension().to_owned();
            let mut old = LevelTickPostEvent { dimension: &dimension };
            create().level_tick_post(&mut Adapter::ClientMut(host), &mut old);
        }
    }
}

pub fn client_tick_post(host: &mut dyn ClientHost) {
    let mut old = ClientTickPostEvent {};
    create().client_tick_post(&mut Adapter::ClientMut(host), &mut old);
}

pub fn component_types(event: &mut mod_api::ComponentTypesEvent<'_>) {
    let mut old = ComponentTypesEvent { types: &mut *event.types };
    create().component_types(&mut old);
}

pub fn right_click_block(host: HostSide<'_>, event: &mut mod_api::RightClickBlockEvent<'_>) -> mod_api::Flow {
    let mut old = RightClickBlockEvent { pos: event.click.pos, face: event.click.hit.face };
    match host {
        HostSide::Server(host) => create().right_click_block(&mut Adapter::Server(host), &mut old),
        HostSide::Client(host) => create().right_click_block(&mut Adapter::ClientMut(host), &mut old),
    }
}

pub fn left_click_block(host: &mut dyn ServerHost, event: &mut mod_api::LeftClickBlockEvent) -> mod_api::Flow {
    let mut old = LeftClickBlockEvent { pos: event.pos, face: event.face, state: event.state.into() };
    create().left_click_block(&mut Adapter::Server(host), &mut old)
}

pub fn block_step_on(event: &mut mod_api::BlockStepOnEvent<'_>) {
    let mut old = BlockStepOnEvent {
        dimension: event.dimension,
        entity: event.entity,
        kind: event.kind,
        state: event.state.into(),
        position: event.position,
    };
    create().block_step_on(&mut old);
}

pub fn model_baking_completed(host: &mut dyn ClientHost) {
    adapter::models_baked(host);
}
