//! `Host` (the old trait) on the new hosts.
//!
//! The old `Host` was one trait of about 500 methods; the modules call 146 of them (`old_api.rs`). A method that is
//! not overridden here is a logged neutral value (`gap`): MIGRATION.md lists them, and `tools/hook_map.txt` the hooks.
//! What follows is the first group: the world, block entities, drawing and the few services the kinetics core and the
//! press, millstone and goggles paths use. The rest are step 1b, one group at a time.
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};

use super::*;

/// What a hook was handed by the new API: the server's host, a client's, the shared base of both, or nothing
/// (registration time, and the hooks that have no host at all).
pub enum Adapter<'a> {
    Server(&'a mut dyn ServerHost),
    Client(&'a dyn ClientHost),
    ClientMut(&'a mut dyn ClientHost),
    Base(&'a dyn mod_api::Host),
    None,
}

/// The standalone models the modules asked for, and the handles they resolved to once the client baked them.
pub(crate) static MODEL_NAMES: Mutex<Vec<String>> = Mutex::new(Vec::new());
static MODELS: LazyLock<Mutex<HashMap<String, ModelRef>>> = LazyLock::new(Mutex::default);

/// `model_baking_completed`: resolve every registered standalone model to its handle.
pub fn models_baked(host: &mut dyn ClientHost) {
    let names = MODEL_NAMES.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let mut resolved = HashMap::new();
    for name in names {
        if let Ok(id) = Identifier::parse(&name) {
            if let Ok(model) = host.render_assets().model(&id) {
                resolved.insert(name, model);
            }
        }
    }
    *MODELS.lock().unwrap_or_else(PoisonError::into_inner) = resolved;
}

fn model(name: &str) -> Option<ModelRef> {
    MODELS.lock().unwrap_or_else(PoisonError::into_inner).get(name).copied()
}

/// The new flag word for the old integer one.
fn flags_of(flags: i32) -> BlockUpdateFlags {
    const ALL: [BlockUpdateFlags; 10] = [
        BlockUpdateFlags::NEIGHBORS,
        BlockUpdateFlags::CLIENTS,
        BlockUpdateFlags::INVISIBLE,
        BlockUpdateFlags::KNOWN_SHAPE,
        BlockUpdateFlags::SUPPRESS_DROPS,
        BlockUpdateFlags::MOVE_BY_PISTON,
        BlockUpdateFlags::SKIP_SHAPE_UPDATE_ON_WIRE,
        BlockUpdateFlags::SKIP_BLOCK_ENTITY_SIDE_EFFECTS,
        BlockUpdateFlags::SKIP_ON_PLACE,
        BlockUpdateFlags::NONE,
    ];
    ALL.iter().filter(|flag| flag.bits() != 0 && flags & flag.bits() != 0).fold(BlockUpdateFlags::NONE, |a, b| a | *b)
}

/// `block_state_property` hands out a `&str` tied to `&self`; the properties of a state never change, so each is
/// leaked once and remembered.
static PROPERTIES: LazyLock<Mutex<HashMap<(u32, String), Option<&'static str>>>> = LazyLock::new(Mutex::default);

/// The property `key` of the state text `Name[a=b,c=d]`.
fn property_of(text: &str, key: &str) -> Option<String> {
    let inner = text.split_once('[')?.1.strip_suffix(']')?;
    inner.split(',').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| v.to_owned())
    })
}

impl Adapter<'_> {
    fn state_text_at(&self, pos: [i32; 3]) -> Option<String> {
        let host = self.new_host();
        let level = host.level(host.dimension())?;
        let state = level.block_state(host, pos)?;
        host.block_state_string(state)
    }
}

impl Host for Adapter<'_> {
    fn new_host(&self) -> &dyn mod_api::Host {
        match self {
            Adapter::Server(host) => &**host,
            Adapter::Client(host) => &**host,
            Adapter::ClientMut(host) => &**host,
            Adapter::Base(host) => *host,
            Adapter::None => panic!("compat: no host at registration time"),
        }
    }
    fn new_server(&mut self) -> &mut dyn ServerHost {
        match self {
            Adapter::Server(host) => &mut **host,
            _ => panic!("compat: a server-only call off the server"),
        }
    }

    // --- the level and its blocks --------------------------------------------------------------------------

    fn dimension(&self) -> &'static str {
        match self {
            Adapter::None => "minecraft:overworld",
            _ => self.new_host().dimension(),
        }
    }

    fn level(&self, dimension: &str) -> Option<LevelRef> {
        self.new_host().level(dimension)
    }

    fn block(&self, pos: [i32; 3]) -> String {
        self.state_text_at(pos).map(|text| text.split('[').next().unwrap_or("").to_owned()).unwrap_or_default()
    }

    fn block_property(&self, pos: [i32; 3], key: &str) -> Option<String> {
        property_of(&self.state_text_at(pos)?, key)
    }

    fn block_state_property(&self, state: BlockStateId, key: &str) -> Option<&str> {
        let mut cache = PROPERTIES.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(found) = cache.get(&(state.0, key.to_owned())) {
            return *found;
        }
        let text = state.state().and_then(|s| self.new_host().block_state_string(s));
        let value = text.and_then(|text| property_of(&text, key)).map(|v| &*Box::leak(v.into_boxed_str()));
        cache.insert((state.0, key.to_owned()), value);
        value
    }

    fn default_state(&self, block: &str) -> Option<BlockStateId> {
        self.new_host().block_state_id(block).map(Into::into)
    }

    fn set_block_property(&mut self, pos: [i32; 3], key: &str, value: &str) -> bool {
        match self {
            Adapter::Server(host) => host.set_block_property(pos, key, value),
            _ => false,
        }
    }

    fn set_block(&mut self, dimension: &str, pos: [i32; 3], state: BlockStateId, flags: i32) -> bool {
        let Adapter::Server(host) = self else { return false };
        let (Some(level), Some(state)) = (host.level(dimension), state.state()) else { return false };
        level.set_block(&mut **host, pos, state, flags_of(flags)).unwrap_or(false)
    }

    // --- block entities ------------------------------------------------------------------------------------

    fn mod_block_entity(&self, pos: [i32; 3]) -> Option<ModBlockEntity> {
        let host = self.new_host();
        let level = host.level(host.dimension())?;
        let entity = host.block_entity(level, pos)?;
        let id = entity.kind(host).map(|kind| kind.id(host)).unwrap_or_default();
        Some(ModBlockEntity {
            id,
            nbt: entity.data(host)?,
            items: host_services::slots_of(host, entity),
            data: entity.data_slots(host).unwrap_or_default(),
        })
    }

    fn change_mod_block_entity(&mut self, pos: [i32; 3], change: &mut dyn FnMut(&mut ModBlockEntity)) -> bool {
        let Some(mut entity) = self.mod_block_entity(pos) else { return false };
        let before = entity.clone();
        change(&mut entity);
        if entity == before {
            return true;
        }
        let Adapter::Server(host) = self else { return false };
        let Some(level) = host.level(host.dimension()) else { return false };
        let Some(handle) = host.block_entity(level, pos) else { return false };
        let mut ok = true;
        if entity.nbt != before.nbt {
            ok &= handle.replace_data(&mut **host, entity.nbt.clone()).is_ok();
        }
        if entity.items != before.items {
            // Each changed slot is written in one transaction, as the new API journals them.
            let slots: Vec<(u32, Option<mod_api::ItemStack>)> = entity
                .items
                .iter()
                .enumerate()
                .filter(|(index, stack)| before.items.get(*index) != Some(*stack))
                .map(|(index, stack)| (index as u32, stack.as_ref().and_then(|stack| events::stack_new(&**host, stack))))
                .collect();
            ok &= host
                .transaction(|host, tx| -> Result<(), ()> {
                    for (slot, stack) in slots {
                        handle.set_item_slot(host, tx, slot, stack).map_err(|_| ())?;
                    }
                    Ok(())
                })
                .is_ok();
        }
        if entity.items != before.items {
            events::cache_slots(pos, &entity.items);
        }
        if entity.data != before.data {
            ok &= handle.set_data_slots(&mut **host, entity.data.clone()).is_ok();
        }
        ok
    }

    fn block_entity_sync(&mut self, pos: [i32; 3]) {
        if let Adapter::Server(host) = self {
            host.block_entity_sync(pos);
        }
    }

    // --- drawing (client, inside block_entity_submit) --------------------------------------------------------

    fn draw_model(&self, draw: &mut BlockEntityDraw, model_name: &str, tint: u32) -> bool {
        let Some(handle) = model(model_name) else { return false };
        draw.ctx().model(handle, tint).is_ok()
    }

    fn draw_block(&self, draw: &mut BlockEntityDraw, block: &str, properties: &[(&str, &str)]) -> bool {
        let Some(key) = self.new_host().block_state_id(block).map(|state| state.block()) else { return false };
        draw.ctx().block(key, properties)
    }

    fn draw_cube(
        &self,
        draw: &mut BlockEntityDraw,
        from: [f32; 3],
        to: [f32; 3],
        faces: [Option<CubeFace<'_>>; 6],
        tint: u32,
    ) -> bool {
        fn face(face: Option<CubeFace<'_>>) -> Option<mod_api::CubeFace<'_>> {
            face.map(|face| {
                let new = mod_api::CubeFace::new(face.sprite);
                match face.uv {
                    Some(uv) => new.with_uv(uv),
                    None => new,
                }
            })
        }
        let [a, b, c, d, e, f] = faces;
        let faces = [face(a), face(b), face(c), face(d), face(e), face(f)];
        draw.ctx().cube(from, to, faces, tint)
    }

    fn draw_quad(&self, draw: &mut BlockEntityDraw, sprite: &str, corners: [[f32; 3]; 4], uv: [f32; 4], tint: u32) -> bool {
        draw.ctx().quad(sprite, corners, uv, tint)
    }

    fn draw_item(
        &self,
        draw: &mut BlockEntityDraw,
        stack: &ItemStack,
        display: DisplayContext,
        owner_yaw: Option<f32>,
        seed: i32,
    ) -> bool {
        let Some(new) = events::stack_new(self.new_host(), stack) else { return false };
        draw.ctx().item(&new, display.into(), owner_yaw, seed)
    }

    // --- small services ------------------------------------------------------------------------------------

    fn run_command(&mut self, line: &str) -> bool {
        match self {
            Adapter::Server(host) => host.commands().run_command(line),
            _ => false,
        }
    }


    // --- entities ------------------------------------------------------------------------------------------------

    fn entity(&self, id: u32) -> Option<EntityRef> {
        self.base()?.entity(id)
    }
    fn entity_position(&self, entity: EntityRef) -> Option<[f64; 3]> {
        entity.position(self.base()?)
    }
    fn entity_velocity(&self, entity: EntityRef) -> Option<[f64; 3]> {
        entity.velocity(self.base()?)
    }
    fn entity_bounding_box(&self, entity: EntityRef) -> Option<[[f64; 3]; 2]> {
        entity.bounding_box(self.base()?)
    }
    fn entity_rotation(&self, entity: EntityRef) -> Option<[f32; 2]> {
        entity.rotation(self.base()?)
    }
    fn entity_on_ground(&self, entity: EntityRef) -> Option<bool> {
        entity.on_ground(self.base()?)
    }
    fn entity_kind(&self, entity: EntityRef) -> Option<String> {
        let host = self.base()?;
        Some(entity.kind(host)?.id(host))
    }
    fn entity_fire_immune(&self, entity: EntityRef) -> Option<bool> {
        entity.fire_immune(self.base()?)
    }
    fn entity_fire_ticks(&self, entity: EntityRef) -> Option<i32> {
        entity.fire_ticks(self.base()?)
    }
    fn entity_head_yaw(&self, entity: EntityRef) -> Option<f32> {
        entity.head_yaw(self.base()?)
    }
    fn entity_is_shift_key_down(&self, entity: EntityRef) -> Option<bool> {
        entity.is_shift_key_down(self.base()?)
    }
    fn entity_uuid(&mut self, entity: EntityRef) -> Option<u128> {
        entity.uuid(self.server_mut()?)
    }
    fn entity_nbt(&mut self, entity: EntityRef) -> Option<Compound> {
        entity.nbt(self.server_mut()?)
    }
    fn entity_push(&mut self, entity: EntityRef, delta: [f64; 3]) -> bool {
        self.server_mut().is_some_and(|host| entity.push(host, delta))
    }
    fn entity_set_fire(&mut self, entity: EntityRef, ticks: i32) -> bool {
        self.server_mut().is_some_and(|host| entity.set_remaining_fire_ticks(host, ticks))
    }
    fn entity_set_head_yaw(&mut self, entity: EntityRef, yaw: f32) -> bool {
        self.server_mut().is_some_and(|host| entity.set_head_yaw(host, yaw))
    }
    fn entity_set_rotation(&mut self, entity: EntityRef, yaw: f32, pitch: f32) -> bool {
        self.server_mut().is_some_and(|host| entity.set_rotation(host, yaw, pitch))
    }
    fn entity_set_velocity(&mut self, entity: EntityRef, velocity: [f64; 3]) -> bool {
        self.server_mut().is_some_and(|host| entity.set_velocity(host, velocity))
    }
    fn entity_teleport_to(&mut self, entity: EntityRef, pos: [f64; 3], rotation: Option<[f32; 2]>) -> bool {
        self.server_mut().is_some_and(|host| entity.teleport_to(host, pos, rotation))
    }
    fn entities_in(&mut self, dimension: &str, min: [f64; 3], max: [f64; 3]) -> Vec<u32> {
        let Some(host) = self.base() else { return Vec::new() };
        let Some(level) = host.level(dimension) else { return Vec::new() };
        level.entities_in(host, min, max).unwrap_or_default().into_iter().map(|entity| entity.id()).collect()
    }
    fn vehicle_of(&self, entity: u32) -> Option<u32> {
        let host = self.base()?;
        Some(host.entity(entity)?.vehicle(host)?.id())
    }
    fn dismount(&mut self, entity: u32) -> bool {
        let Some(host) = self.server_mut() else { return false };
        host.entity(entity).is_some_and(|entity| entity.stop_riding(host))
    }
    fn remove_entity(&mut self, _dimension: &str, id: u32) -> bool {
        let Some(host) = self.server_mut() else { return false };
        host.entity(id).is_some_and(|entity| entity.discard(host))
    }
    fn remove_mob(&mut self, dimension: &str, id: u32) -> bool {
        self.remove_entity(dimension, id)
    }
    fn living(&self, entity: EntityRef) -> Option<LivingRef> {
        self.base()?.living(entity)
    }
    fn living_set_equipment(&mut self, living: LivingRef, slot: EquipmentSlot, stack: Option<ItemStack>) -> bool {
        let Some(host) = self.server_mut() else { return false };
        let new = match stack {
            Some(stack) if stack.count > 0 => match events::stack_new(&*host, &stack) {
                Some(new) => Some(new),
                None => return false,
            },
            _ => None,
        };
        living.set_equipment(host, slot, new).unwrap_or(false)
    }
    fn hurt_entity(&mut self, _dimension: &str, target: u32, amount: f32, damage_type: &str, attacker: Option<u32>) -> bool {
        let Some(host) = self.server_mut() else { return false };
        let Some(target) = host.entity(target) else { return false };
        let attacker = attacker.and_then(|id| host.entity(id));
        target.hurt(host, damage_type, amount, attacker, attacker).unwrap_or(false)
    }
    fn summon(&mut self, dimension: &str, kind: &str, pos: [f64; 3], nbt: Compound) -> Option<u32> {
        self.server_mut()?.summon(dimension, kind, pos, nbt)
    }
    fn summon_with_uuid(&mut self, dimension: &str, kind: &str, pos: [f64; 3], mut nbt: Compound, uuid: u128) -> Option<u32> {
        // An entity's data names its UUID as four ints, most significant first.
        let ints = [(uuid >> 96) as i32, (uuid >> 64) as i32, (uuid >> 32) as i32, uuid as i32];
        nbt.insert("UUID".into(), Tag::IntArray(ints.to_vec()));
        self.server_mut()?.summon(dimension, kind, pos, nbt)
    }

    // --- item entities -------------------------------------------------------------------------------------------

    fn item_entities(&self, dimension: &str, min: [f64; 3], max: [f64; 3]) -> Vec<NativeItemEntity> {
        let Some(host) = self.base() else { return Vec::new() };
        let Some(level) = host.level(dimension) else { return Vec::new() };
        level
            .entities_in(host, min, max)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|entity| {
                let item = host.item_entity(entity)?;
                Some(NativeItemEntity {
                    id: entity.id(),
                    position: entity.position(host)?,
                    stack: events::stack_old(host, &item.stack(host)?)?,
                    pickup_delay: item.pickup_delay(host).unwrap_or(0).max(0) as u32,
                })
            })
            .collect()
    }
    fn set_item_entity(&mut self, _dimension: &str, id: u32, stack: ItemStack) -> bool {
        let Some(host) = self.server_mut() else { return false };
        let Some(entity) = host.entity(id) else { return false };
        if stack.count == 0 {
            return entity.discard(host);
        }
        let (Some(item), Some(new)) = (host.item_entity(entity), events::stack_new(&*host, &stack)) else { return false };
        item.set_stack(host, new).unwrap_or(false)
    }

    // --- items and the player's inventory -----------------------------------------------------------------------

    fn max_stack_size(&self, item: &str) -> Option<u32> {
        host_services::max_stack(self.base()?, item)
    }
    fn take_items(&mut self, item: &str, count: u32) -> u32 {
        let Some(host) = self.server_mut() else { return 0 };
        let Some(player) = host.players().acting_player() else { return 0 };
        host_services::take(host, player, item, count)
    }
    fn give_items(&mut self, item: &str, count: u32) -> u32 {
        let Some(host) = self.server_mut() else { return 0 };
        let Some(player) = host.players().acting_player() else { return 0 };
        host_services::give(host, player, &ItemStack::new(item, count))
    }
    fn give_stack(&mut self, uuid: u128, stack: ItemStack) -> Option<ItemStack> {
        let Some(host) = self.server_mut() else { return Some(stack) };
        let Some(player) = host.player(uuid) else { return Some(stack) };
        let gave = host_services::give(host, player, &stack);
        (gave < stack.count).then(|| stack.with_count(stack.count - gave))
    }
    fn player_equipment(&self, uuid: u128, slot: EquipmentSlot) -> Option<ItemStack> {
        let host = self.base()?;
        let living = host.living(host.player(uuid)?.entity())?;
        events::stack_old(host, &living.equipment(host, slot)?)
    }
    fn equipment(&self, slot: EquipmentSlot) -> Option<ItemStack> {
        // The acting player on a server, the window's own player on a client.
        let host = self.base()?;
        let entity = match self {
            Adapter::Server(server) => server.players().acting_player()?.entity(),
            Adapter::Client(client) => client.camera_entity()?,
            Adapter::ClientMut(client) => client.camera_entity()?,
            _ => return None,
        };
        events::stack_old(host, &host.living(entity)?.equipment(host, slot)?)
    }
    fn player_stacks(&self, uuid: u128) -> Option<Vec<Option<ItemStack>>> {
        let host = self.base()?;
        let slots = host.player(uuid)?.inventory_slots(host)?;
        let mut out = vec![None; 36];
        for (slot, stack) in slots {
            if let Some(place) = out.get_mut(slot as usize) {
                *place = events::stack_old(host, &stack);
            }
        }
        Some(out)
    }
    fn player_food(&self, uuid: u128) -> Option<(i32, f32, f32)> {
        let host = self.base()?;
        let food = host.player(uuid)?.food(host)?;
        Some((food.level, food.saturation, food.exhaustion))
    }
    fn set_player_status(
        &mut self,
        uuid: u128,
        health: Option<f32>,
        food: Option<(i32, f32, f32)>,
        xp: Option<(i32, f32, i32)>,
    ) -> bool {
        let Some(host) = self.server_mut() else { return false };
        let Some(player) = host.player(uuid) else { return false };
        let mut ok = true;
        if let Some((level, saturation, exhaustion)) = food {
            ok &= player.set_food_level(host, level);
            ok &= player.set_saturation(host, saturation);
            ok &= player.set_exhaustion(host, exhaustion);
        }
        if let Some((level, _progress, _total)) = xp {
            ok &= player.set_experience_levels(host, level);
        }
        if let Some(health) = health {
            if let Some(living) = host.living(player.entity()) {
                ok &= living.set_health(host, health);
            }
        }
        ok
    }
    fn attribute(&mut self, uuid: u128, attribute: &str) -> Option<(f64, f64)> {
        let host = self.base()?;
        let living = host.living(host.player(uuid).map(PlayerRef::entity).or_else(|| host.entity(uuid as u32))?)?;
        Some((living.attribute_value(host, attribute)?, living.attribute_base(host, attribute)?))
    }
    fn set_attribute_base(&mut self, uuid: u128, attribute: &str, base: f64) -> bool {
        let Some(host) = self.server_mut() else { return false };
        let Some(entity) = host.player(uuid).map(PlayerRef::entity) else { return false };
        host.living(entity).is_some_and(|living| living.set_attribute_base(host, attribute, base))
    }
    fn player_break_block(&mut self, uuid: u128, pos: [i32; 3], into_inventory: bool) -> bool {
        let Some(host) = self.server_mut() else { return false };
        host.player(uuid).is_some_and(|player| player.destroy_block(host, pos, into_inventory))
    }
    fn server_players(&self) -> Vec<NativeServerPlayer> {
        let Adapter::Server(server) = self else { return Vec::new() };
        let host = self.new_host();
        server.players().server_players().into_iter().map(|player| host_services::player_view(host, player)).collect()
    }

    // --- recipes -------------------------------------------------------------------------------------------------

    fn recipes_of_type(&self, kind: &str) -> &[CustomRecipe] {
        match self.base() {
            Some(host) => host_services::recipes(host, kind),
            None => &[],
        }
    }
    fn recipe_ingredient(&self, json: &serde_json::Value) -> Option<Ingredient> {
        let host = self.base()?;
        host.recipe_ingredient(&json.to_string()).ok().flatten().map(Ingredient)
    }
    fn recipe_result(&self, json: &serde_json::Value) -> Option<ItemStack> {
        let host = self.base()?;
        events::stack_old(host, &host.recipe_result(&json.to_string()).ok().flatten()?)
    }

    // --- sounds and particles ------------------------------------------------------------------------------------

    fn play_sound(&mut self, sound: &str, at: [f64; 3], source: usize, volume: f32, pitch: f32) -> bool {
        let Some(level) = self.level_here() else { return false };
        let Some(host) = self.server_mut() else { return false };
        level.play_sound(host, sound, at, host_services::sound_source(source), volume, pitch)
    }
    fn send_particles(
        &mut self,
        options: &ParticleOptions,
        state: Option<BlockStateId>,
        at: [f64; 3],
        spread: [f64; 3],
        speed: f64,
        count: i32,
    ) -> bool {
        let (Some(level), Some(options)) = (self.level_here(), host_services::particle_new(options)) else { return false };
        let Some(host) = self.server_mut() else { return false };
        level.send_particles(host, &options, state.and_then(BlockStateId::state), at, spread, speed, count)
    }
    fn send_particles_to(
        &mut self,
        _uuid: u128,
        options: &ParticleOptions,
        state: Option<BlockStateId>,
        at: [f64; 3],
        spread: [f64; 3],
        speed: f64,
        count: i32,
        _force: bool,
    ) -> bool {
        // The new API sends particles to everyone near, not to one player: the player filter is dropped.
        self.send_particles(options, state, at, spread, speed, count)
    }

    // --- the world around a block --------------------------------------------------------------------------------

    fn chunk_is_loaded(&self, dimension: &str, chunk: [i32; 2]) -> bool {
        let Some(host) = self.base() else { return false };
        host.level(dimension).is_some_and(|level| level.is_loaded(host, [chunk[0] * 16, 0, chunk[1] * 16]))
    }
    fn level_is_loaded(&self, level: LevelRef, pos: [i32; 3]) -> bool {
        self.base().is_some_and(|host| level.is_loaded(host, pos))
    }
    fn block_collision(&self, pos: [i32; 3]) -> Vec<[f64; 6]> {
        self.base().map(|host| host.block_collision(pos)).unwrap_or_default()
    }
    fn light(&self, pos: [i32; 3]) -> (u8, u8) {
        let (Some(host), Some(level)) = (self.base(), self.level_here()) else { return (0, 0) };
        let read = |layer| level.brightness(host, layer, pos).unwrap_or(0).clamp(0, 15) as u8;
        (read(mod_api::LightLayer::Block), read(mod_api::LightLayer::Sky))
    }
    fn day_time(&self) -> i64 {
        let (Some(host), Some(level)) = (self.base(), self.level_here()) else { return 0 };
        level.day_time(host).unwrap_or(0)
    }
    fn world_height(&self) -> (i32, i32) {
        let (Some(host), Some(level)) = (self.base(), self.level_here()) else { return (0, 0) };
        level.height(host).unwrap_or((0, 0))
    }
    fn world_border(&self, dimension: &str) -> Option<[f64; 4]> {
        let host = self.base()?;
        host.level(dimension)?.world_border(host)
    }
    fn level_difficulty(&self, level: LevelRef) -> Option<&'static str> {
        level.difficulty(self.base()?)
    }
    fn set_break_progress(&mut self, breaker: i32, pos: [i32; 3], progress: i32) {
        let Some(level) = self.level_here() else { return };
        if let Some(host) = self.server_mut() {
            level.set_break_progress(host, breaker, pos, progress);
        }
    }
    fn update_signal_neighbors(&mut self, dimension: &str, pos: [i32; 3]) -> bool {
        let Some(host) = self.server_mut() else { return false };
        let Some(level) = host.level(dimension) else { return false };
        let Some(state) = level.block_state(&*host, pos) else { return false };
        level.update_neighbors(host, pos, state, mod_api::NeighbourUpdate::Neighbours).unwrap_or(false)
    }
    fn update_output_neighbors(&mut self, dimension: &str, pos: [i32; 3]) -> bool {
        let Some(host) = self.server_mut() else { return false };
        let Some(level) = host.level(dimension) else { return false };
        let Some(state) = level.block_state(&*host, pos) else { return false };
        level.update_neighbors(host, pos, state, mod_api::NeighbourUpdate::OutputSignal).unwrap_or(false)
    }
    fn destroy_block(&mut self, pos: [i32; 3], tool: Option<&ItemStack>, collect: bool) -> Option<Vec<ItemStack>> {
        let level = self.level_here()?;
        let host = self.server_mut()?;
        let tool = tool.and_then(|tool| events::stack_new(&*host, tool));
        let drops = level.destroy_block(host, pos, true, tool.as_ref(), collect).ok()??;
        Some(drops.iter().filter_map(|stack| events::stack_old(&*host, stack)).collect())
    }
    fn level_random(&mut self, dimension: &str) -> Option<&mut LegacyRandom> {
        host_services::level_random(self.server_mut()?, dimension)
    }
    fn set_block_property_with(&mut self, pos: [i32; 3], key: &str, value: &str, flags: i32) -> bool {
        self.server_mut().is_some_and(|host| host.set_block_property_with(pos, key, value, flags))
    }
    fn block_entity_tag(&mut self, dimension: &str, pos: [i32; 3]) -> Option<Tag> {
        let host = self.base()?;
        let level = host.level(dimension)?;
        Some(Tag::Compound(host.block_entity(level, pos)?.data(host)?))
    }
    fn set_block_tag(&mut self, dimension: &str, pos: [i32; 3], tag: Tag) -> bool {
        let Tag::Compound(data) = tag else { return false };
        let Some(host) = self.server_mut() else { return false };
        let Some(level) = host.level(dimension) else { return false };
        let Some(entity) = host.block_entity(level, pos) else { return false };
        entity.replace_data(host, data).unwrap_or(false)
    }
    fn block_name_in_tag(&self, block: &str, tag: &str) -> Result<bool, NativeBlockReadError> {
        let host = self.base().ok_or(NativeBlockReadError::InactiveContext)?;
        let key = host.block_state_id(block).map(|state| state.block()).ok_or(NativeBlockReadError::InvalidState)?;
        host.block_in_tag(key, tag).ok_or(NativeBlockReadError::TagsUnavailable)
    }


    // --- drawing, more (client, inside block_entity_submit) --------------------------------------------------

    fn draw_text(
        &self,
        draw: &mut BlockEntityDraw,
        text: &Text,
        at: [f32; 2],
        color: u32,
        glowing: bool,
        outline: u32,
    ) -> f32 {
        draw.ctx().text(text, at, color, glowing, (outline != 0).then_some(outline)).unwrap_or(0.0)
    }
    fn draw_terrain_model(&self, draw: &mut BlockEntityDraw, model_name: &str, options: NativeModelRenderOptions<'_>) -> bool {
        let Some(handle) = model(model_name) else { return false };
        let mut new = mod_api::TerrainModelOptions::default();
        new.material = match options.material {
            NativeModelMaterial::Model => mod_api::TerrainMaterial::Model,
            NativeModelMaterial::Solid => mod_api::TerrainMaterial::Solid,
            NativeModelMaterial::Cutout => mod_api::TerrainMaterial::Cutout,
            NativeModelMaterial::CutoutMipped => mod_api::TerrainMaterial::CutoutMipped,
        };
        new.shade = match options.shade {
            NativeModelShade::Model => mod_api::TerrainShade::Model,
            NativeModelShade::Off => mod_api::TerrainShade::Off,
            NativeModelShade::Entity => mod_api::TerrainShade::Entity,
        };
        new.uv_shift = options
            .uv_shift
            .map(|shift| mod_api::TerrainUvShift::new(shift.source_sprite, shift.target_sprite, shift.scale, shift.offset));
        new.uv_shrink_ratio = options.uv_shrink_ratio;
        new.tint = options.tint;
        draw.ctx().terrain_model(handle, new).is_ok()
    }
    fn draw_fluid_box(
        &self,
        draw: &mut BlockEntityDraw,
        stack: &NativeFluidStack,
        bounds: [f32; 6],
        options: NativeFluidRenderOptions,
    ) -> bool {
        let Some(host) = self.base() else { return false };
        let Some(key) = host.fluid_key(&stack.fluid) else { return false };
        let mut new = mod_api::FluidBoxOptions::new(options.render_bottom, options.uv_shrink_ratio);
        new.argb = options.argb;
        draw.ctx().fluid_box(&mod_api::FluidStack::new(key, stack.amount.max(0) as u32), bounds, new).is_ok()
    }

    // --- rays and the world ----------------------------------------------------------------------------------

    fn fluid_flow(&self, dimension: &str, pos: [i32; 3]) -> Option<[f64; 3]> {
        let host = self.base()?;
        let flow = host.level(dimension)?.fluid_flow(host, pos)?;
        Some([flow.x, flow.y, flow.z])
    }
    fn collision_boxes(&self, dimension: &str, aabb: [f64; 6]) -> Vec<[f64; 6]> {
        let Some(host) = self.base() else { return Vec::new() };
        host.level(dimension).and_then(|level| level.collision_boxes(host, aabb)).unwrap_or_default()
    }
    fn clear_between(&self, dimension: &str, from: [f64; 3], to: [f64; 3]) -> bool {
        let Some(host) = self.base() else { return false };
        let Some(level) = host.level(dimension) else { return false };
        level
            .clip(host, from, to, mod_api::ClipBlock::Collider, mod_api::ClipFluid::None)
            .is_some_and(|hit| hit.miss)
    }
    fn pick_block(&self, eye: [f64; 3], dir: [f64; 3], range: f64, _fluids: bool) -> Option<NativeHit> {
        let host = self.base()?;
        let length = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
        if length == 0.0 || !length.is_finite() {
            return None;
        }
        let to = [eye[0] + dir[0] / length * range, eye[1] + dir[1] / length * range, eye[2] + dir[2] / length * range];
        // The new clip has no fluid mode (a gap): a ray never stops at a fluid.
        let hit = self.level_here()?.clip(host, eye, to, mod_api::ClipBlock::Outline, mod_api::ClipFluid::None)?;
        if hit.miss {
            return None;
        }
        let block = host.block_state_string(self.level_here()?.block_state(host, hit.block)?)?;
        Some(NativeHit {
            block_x: hit.block[0],
            block_y: hit.block[1],
            block_z: hit.block[2],
            x: hit.location[0],
            y: hit.location[1],
            z: hit.location[2],
            block: block.split('[').next().unwrap_or("").to_owned(),
        })
    }
    fn instance_dir(&self) -> std::path::PathBuf {
        self.base().map(|host| host.instance_dir()).unwrap_or_default()
    }
    fn profile_by_uuid(&self, uuid: u128) -> Option<String> {
        let host = self.base()?;
        host.player(uuid)?.name(host)
    }

    // --- the window's own player (client) --------------------------------------------------------------------

    fn playing(&self) -> bool {
        match self {
            Adapter::Client(host) => host.playing(),
            Adapter::ClientMut(host) => host.playing(),
            _ => false,
        }
    }
    fn player(&self) -> Option<NativePlayer> {
        let host = self.base()?;
        let entity = match self {
            Adapter::Client(client) => client.camera_entity()?,
            Adapter::ClientMut(client) => client.camera_entity()?,
            _ => return None,
        };
        let [x, y, z] = entity.position(host)?;
        let [yaw, pitch] = entity.rotation(host)?;
        Some(NativePlayer { x, y, z, yaw, pitch, dimension: host.dimension() })
    }
    fn sneaking(&self) -> bool {
        let Some(host) = self.base() else { return false };
        let entity = match self {
            Adapter::Client(client) => client.camera_entity(),
            Adapter::ClientMut(client) => client.camera_entity(),
            _ => None,
        };
        entity.and_then(|entity| entity.is_shift_key_down(host)).unwrap_or(false)
    }
    fn push_player(&mut self, kick: [f32; 3]) {
        if let Adapter::ClientMut(host) = self {
            host.push_player(kick);
        }
    }
    fn raycast(&self, distance: f32) -> Option<NativeHit> {
        let hit = match self {
            Adapter::Client(host) => host.raycast(distance)?,
            Adapter::ClientMut(host) => host.raycast(distance)?,
            _ => return None,
        };
        let host = self.base()?;
        Some(NativeHit {
            block_x: hit.block_x,
            block_y: hit.block_y,
            block_z: hit.block_z,
            x: hit.x,
            y: hit.y,
            z: hit.z,
            block: hit.block.id(host),
        })
    }
    fn view_hit(&self) -> Option<(f32, NativeHit)> {
        let (distance, hit) = match self {
            Adapter::Client(host) => host.view_hit()?,
            Adapter::ClientMut(host) => host.view_hit()?,
            _ => return None,
        };
        let host = self.base()?;
        let hit = NativeHit {
            block_x: hit.block_x,
            block_y: hit.block_y,
            block_z: hit.block_z,
            x: hit.x,
            y: hit.y,
            z: hit.z,
            block: hit.block.id(host),
        };
        Some((distance, hit))
    }

    // --- containers by capability ----------------------------------------------------------------------------

    fn insert_item(&mut self, pos: [i32; 3], face: Direction, stack: ItemStack) -> Option<ItemStack> {
        let Some(level) = self.level_here() else { return Some(stack) };
        let Some(host) = self.server_mut() else { return Some(stack) };
        let Some(new) = events::stack_new(&*host, &stack) else { return Some(stack) };
        let Some(cap) = level.capability(host, &mod_api::ITEMS, pos, Some(face)) else { return Some(stack) };
        let resource = mod_api::ItemResource::of(&new);
        let moved = host
            .transaction(|host, tx| -> Result<u32, ()> {
                let mut moved = 0;
                for slot in 0..cap.size(host) {
                    if moved >= stack.count {
                        break;
                    }
                    moved += cap.insert(host, tx, slot, &resource, stack.count - moved).min(stack.count - moved);
                }
                Ok(moved)
            })
            .unwrap_or(0);
        (moved < stack.count).then(|| stack.with_count(stack.count - moved))
    }
    fn extract_item(&mut self, dimension: &str, pos: [i32; 3], face: Direction, slot: usize, count: u32) -> Option<ItemStack> {
        let host = self.server_mut()?;
        let level = host.level(dimension)?;
        let cap = level.capability(host, &mod_api::ITEMS, pos, Some(face))?;
        let resource = cap.resource(host, slot)?;
        let taken = host
            .transaction(|host, tx| -> Result<u32, ()> { Ok(cap.extract(host, tx, slot, &resource, count)) })
            .ok()?;
        events::stack_old(&*host, &resource.to_stack(taken)?)
    }

    fn set_inventory_slot_typed(
        &mut self,
        uuid: u128,
        _kind: NativeInventoryKind,
        slot: usize,
        expected: Option<&ItemStack>,
        new: Option<ItemStack>,
    ) -> Result<Option<ItemStack>, NativeInventoryError> {
        let Some(host) = self.server_mut() else { return Err(NativeInventoryError::NoServer) };
        let Some(player) = host.player(uuid) else { return Err(NativeInventoryError::NotOnline) };
        if slot > 42 {
            return Err(NativeInventoryError::NoSuchSlot);
        }
        let held = player
            .inventory_slots(&*host)
            .and_then(|slots| slots.into_iter().find(|(index, _)| *index as usize == slot))
            .and_then(|(_, stack)| events::stack_old(&*host, &stack));
        if held.as_ref() != expected {
            return Err(NativeInventoryError::StackChanged);
        }
        let stack = match new {
            Some(stack) if stack.count > 0 => Some(events::stack_new(&*host, &stack).ok_or(NativeInventoryError::NoRoom)?),
            _ => None,
        };
        player.set_inventory_slot(host, slot as u32, stack).map_err(|_| NativeInventoryError::Dead)?;
        Ok(held)
    }

    // --- components on a stack ---------------------------------------------------------------------------------

    fn component(&self, stack: &ItemStack, id: &str) -> Result<Option<Tag>, NativeComponentError> {
        if id == "minecraft:damage" {
            return Ok((stack.damage != 0).then(|| Tag::Int(stack.damage as i32)));
        }
        Ok(stack.data().and_then(|data| data.other(id)).cloned())
    }
    fn set_component(&self, stack: &mut ItemStack, id: &str, value: Tag) -> Result<(), NativeComponentError> {
        if id == "minecraft:damage" {
            if let Tag::Int(damage) = value {
                stack.damage = damage.max(0) as u32;
                return Ok(());
            }
            return Err(NativeComponentError::InvalidValue("damage is an int".into()));
        }
        let data = stack.data_mut();
        data.patch.remove(&format!("!{id}").as_str().into());
        data.set_other(id, Some(value));
        Ok(())
    }
    fn remove_component(&self, stack: &mut ItemStack, id: &str) -> Result<(), NativeComponentError> {
        if id == "minecraft:damage" {
            stack.damage = 0;
            return Ok(());
        }
        let data = stack.data_mut();
        data.set_other(id, None);
        // A removal of a default component is written `!id` in the patch, as the game saves it.
        data.patch.insert(format!("!{id}").as_str().into(), Tag::Compound(Compound::default()));
        Ok(())
    }

    fn upgrade_block_state(&self, name: &str, props: &[(String, String)], _data_version: i32) -> (String, Vec<(String, String)>) {
        // The new API has no datafixer: a schematic written by this game's version needs none, an older one keeps
        // its old names (gap).
        (name.to_owned(), props.to_vec())
    }
    fn block_sprites_version(&self) -> u64 {
        match self {
            Adapter::Client(host) => host.block_sprites_version(),
            Adapter::ClientMut(host) => host.block_sprites_version(),
            _ => 0,
        }
    }

    fn add_effect(&mut self, uuid: u128, effect: &NativeEffect) -> bool {
        let Some(host) = self.server_mut() else { return false };
        let Some(key) = host.effect_key(&effect.id) else { return false };
        // The old API took the entity id of a mob or the player's uuid; a player is found by uuid first.
        let Some(entity) = host.player(uuid).map(PlayerRef::entity).or_else(|| host.entity(uuid as u32)) else { return false };
        let Some(living) = host.living(entity) else { return false };
        let effect = mod_api::ActiveEffect::new(key, effect.duration, effect.amplifier as i32, effect.ambient, effect.particles, effect.particles);
        living.add_effect(host, &effect)
    }

    fn server_item_in_tag(&self, stack: &ItemStack, tag: &str) -> Result<bool, NativeBlockReadError> {
        let host = self.base().ok_or(NativeBlockReadError::InactiveContext)?;
        let item = Identifier::parse(&stack.item).ok().and_then(|id| host.item_key(&id)).ok_or(NativeBlockReadError::InvalidState)?;
        let tags = host.tags();
        let key = Identifier::parse(tag.trim_start_matches('#')).ok().and_then(|id| tags.key(&id)).ok_or(NativeBlockReadError::TagsUnavailable)?;
        tags.is_in_tag(item, key).ok_or(NativeBlockReadError::TagsUnavailable)
    }

    // --- the client's window and view ----------------------------------------------------------------------

    /// The client's resources, read through the new host (`Host::resource`); used within the hook that made the handle.
    fn client_resources(&self) -> ClientResources {
        match self {
            Adapter::Server(_) | Adapter::None => ClientResources::default(),
            // SAFETY: the handle is read inside the hook call that holds this adapter.
            _ => unsafe { ClientResources::of(self.new_host()) },
        }
    }

    /// The old one-call snapshot of the client, rebuilt from the new client host's calls. The GUI size comes from
    /// the last draw context (`types::gui_size`); there is one window, so its id is a constant (Ponder keys its
    /// per-window state by it).
    fn client_state(&self) -> Option<NativeClientState> {
        let host: &dyn ClientHost = match self {
            Adapter::Client(host) => *host,
            Adapter::ClientMut(host) => &**host,
            _ => return None,
        };
        let camera = host.camera();
        Some(NativeClientState {
            window_id: 1,
            focused: host.window_focused(),
            gui_hidden: host.hud_mode() != mod_api::HudMode::AllVisible,
            gui_size: super::types::gui_size(),
            logged_in: host.camera_entity().is_some(),
            world_partial_tick: camera.as_ref().map_or(0.0, |c| f64::from(c.partial_tick)),
            eye_position: camera
                .as_ref()
                .map(|c| [c.position[0], c.position[1] + f64::from(c.eye_height), c.position[2]]),
            yaw: camera.as_ref().map_or(0.0, |c| c.yaw),
            pitch: camera.as_ref().map_or(0.0, |c| c.pitch),
            menu_blur: host.menu_blur_radius(),
            screen: if host.playing() { NativeScreenKind::Playing } else { NativeScreenKind::Other },
            ..NativeClientState::default()
        })
    }

    // --- players -------------------------------------------------------------------------------------------

    fn acting_player(&self) -> Option<NativeServerPlayer> {
        // `acting_player` is a server service: nobody acts on a client.
        let Adapter::Server(server) = self else { return None };
        Some(host_services::player_view(self.new_host(), server.players().acting_player()?))
    }

    fn held_stack(&self, uuid: u128) -> Option<ItemStack> {
        let host = self.new_host();
        let player = host.player(uuid)?;
        let slot = player.selected_slot(host)?;
        let stack = player.inventory_slots(host)?.into_iter().find(|(index, _)| *index as usize == slot)?.1;
        events::stack_old(host, &stack)
    }

    fn tell(&mut self, uuid: u128, line: Text) -> bool {
        let Adapter::Server(host) = self else { return false };
        let Some(player) = host.player(uuid) else { return false };
        player.tell(&mut **host, line).unwrap_or(false)
    }

    fn action_bar(&mut self, uuid: u128, line: Text) -> bool {
        let Adapter::Server(host) = self else { return false };
        let Some(player) = host.player(uuid) else { return false };
        player.action_bar(&mut **host, line).unwrap_or(false)
    }

    fn neighbor_signal(&mut self, dimension: &str, pos: [i32; 3]) -> i32 {
        let host = self.new_host();
        host.level(dimension).and_then(|level| level.best_neighbor_signal(host, pos)).unwrap_or(0)
    }

    fn window(&self) -> bool {
        matches!(self, Adapter::Client(_) | Adapter::ClientMut(_))
    }

    // --- step 2 hooks: payloads, keys, HUD and screens, commands
    fn fill(&self, batch: &mut HudBatch, rect: [f32; 4], argb: u32) {
        self.hud_fill(batch, rect, argb)
    }
    fn quad(&self, batch: &mut HudBatch, points: [[f32; 2]; 4], argb: u32) {
        self.hud_quad(batch, points, argb)
    }
    fn text(&self, batch: &mut HudBatch, text: &str, at: [f32; 2], rgb: u32, shadow: bool) {
        self.hud_text(batch, text, at, rgb, shadow)
    }
    fn sprite(&self, batch: &mut HudBatch, name: &str, src: [f32; 4], dst: [f32; 4]) -> bool {
        self.hud_sprite(batch, name, src, dst)
    }
    fn item(&self, batch: &mut HudBatch, item: &str, at: [f32; 2], size: f32) -> bool {
        self.hud_item(batch, item, at, size)
    }
    fn gui_tooltip(&self, batch: &mut HudBatch, lines: &[Text], at: [f32; 2]) {
        self.hud_tooltip(batch, lines, at)
    }
    fn text_width(&self, text: &str) -> f32 {
        self.hud_text_width(text)
    }
    fn draw_scene(&self, batch: &mut HudBatch, rect: [f32; 4], blocks: &[NativeSceneBlock<'_>], view: NativeSceneView) {
        self.hud_scene(batch, rect, blocks, view)
    }
    fn has_shift_down(&self) -> bool {
        self.modifiers_down().shift()
    }
    fn has_control_down(&self) -> bool {
        self.modifiers_down().control()
    }
    fn has_alt_down(&self) -> bool {
        self.modifiers_down().alt()
    }
    fn is_key_down(&self, key: KeyCode) -> bool {
        self.key_down(key)
    }
    fn net_send_to_player(&mut self, player: u128, channel: &str, data: &[u8]) -> Result<(), String> {
        self.net_to_player(player, channel, data)
    }
    fn net_send_to_server(&mut self, channel: &str, data: &[u8]) -> Result<(), String> {
        self.net_to_server(channel, data)
    }
    fn mod_screen(&self) -> Option<&str> {
        self.screen_of_mod()
    }
    fn mod_screen_open(&mut self, mod_id: &str) -> bool {
        self.open_mod_screen(mod_id)
    }
    fn mod_screen_close(&mut self, mod_id: &str) {
        self.close_mod_screen(mod_id)
    }
}

impl From<DisplayContext> for mod_api::ItemDisplayContext {
    fn from(context: DisplayContext) -> Self {
        match context {
            DisplayContext::None => Self::None,
            DisplayContext::ThirdPersonLeftHand => Self::ThirdPersonLeftHand,
            DisplayContext::ThirdPersonRightHand => Self::ThirdPersonRightHand,
            DisplayContext::FirstPersonLeftHand => Self::FirstPersonLeftHand,
            DisplayContext::FirstPersonRightHand => Self::FirstPersonRightHand,
            DisplayContext::Head => Self::Head,
            DisplayContext::Gui => Self::Gui,
            DisplayContext::Ground => Self::Ground,
            DisplayContext::Fixed => Self::Fixed,
            DisplayContext::OnShelf => Self::OnShelf,
        }
    }
}
