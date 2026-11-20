//! Helpers for `adapter.rs`: the game calls of the old `Host` (items, entities, players, sounds, particles,
//! recipes, block entity slots) written once on the new API. `adapter.rs` keeps the one-line trait methods.
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};

use super::adapter::Adapter;
use super::*;

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Adapter<'_> {
    /// The shared host, where a hook was given one (everything but registration time).
    pub(super) fn base(&self) -> Option<&dyn mod_api::Host> {
        match self {
            Adapter::Server(host) => Some(&**host),
            Adapter::Client(host) => Some(&**host),
            Adapter::ClientMut(host) => Some(&**host),
            Adapter::Base(host) => Some(*host),
            Adapter::None => None,
        }
    }

    /// The server host, on the server only.
    pub(super) fn server_mut(&mut self) -> Option<&mut dyn ServerHost> {
        match self {
            Adapter::Server(host) => Some(&mut **host),
            _ => None,
        }
    }

    /// The level the hook runs in.
    pub(super) fn level_here(&self) -> Option<LevelRef> {
        let host = self.base()?;
        host.level(host.dimension())
    }
}

// --- particles -----------------------------------------------------------------------------------------------

/// The new API issues a particle key at registration only; `register_particles` asks for the ones the glue uses.
static PARTICLE_KEYS: LazyLock<Mutex<HashMap<&'static str, mod_api::ParticleKey>>> = LazyLock::new(Mutex::default);

const PARTICLE_IDS: [(ParticleType, &str); 6] = [
    (ParticleType::LargeSmoke, "minecraft:large_smoke"),
    (ParticleType::HappyVillager, "minecraft:happy_villager"),
    (ParticleType::Dust, "minecraft:dust"),
    (ParticleType::Smoke, "minecraft:smoke"),
    (ParticleType::CampfireCosySmoke, "minecraft:campfire_cosy_smoke"),
    (ParticleType::CampfireSignalSmoke, "minecraft:campfire_signal_smoke"),
];

/// Called from `Hooks::register`.
pub fn register_particles(r: &mut dyn mod_api::Registrar) {
    let mut keys = lock(&PARTICLE_KEYS);
    for (_, id) in PARTICLE_IDS {
        keys.insert(id, r.particle_key(id));
    }
}

pub(super) fn particle_new(options: &ParticleOptions) -> Option<mod_api::ParticleOptions> {
    let id = PARTICLE_IDS.iter().find(|(ty, _)| *ty == options.ty)?.1;
    let key = *lock(&PARTICLE_KEYS).get(id)?;
    Some(match options.ty {
        ParticleType::Dust => mod_api::ParticleOptions::dust(key, [options.color[0], options.color[1], options.color[2]], options.scale),
        _ => mod_api::ParticleOptions::simple(key),
    })
}

pub(super) fn sound_source(index: usize) -> mod_api::SoundSource {
    use mod_api::SoundSource::*;
    [Master, Music, Records, Weather, Blocks, Hostile, Neutral, Players, Ambient, Voice, Ui]
        .get(index)
        .copied()
        .unwrap_or(Blocks)
}

// --- stacks --------------------------------------------------------------------------------------------------

/// Two old stacks that would share a slot: everything but the count.
pub(super) fn stackable(a: &ItemStack, b: &ItemStack) -> bool {
    a.item == b.item && a.damage == b.damage && a.extra == b.extra
}

/// A stack with no components and no damage ("plain", as the old `take_items` meant).
pub(super) fn plain(stack: &ItemStack) -> bool {
    stack.damage == 0 && stack.extra.is_none()
}

pub(super) fn max_stack(host: &dyn mod_api::Host, item: &str) -> Option<u32> {
    let id = Identifier::parse(item).ok()?;
    host.max_stack_size(host.item_key(&id)?)
}

/// The slots `Inventory.add` fills: the 36 main ones.
const MAIN: std::ops::Range<u32> = 0..36;

/// The player's main inventory as old stacks, by slot.
fn main_slots(host: &dyn mod_api::Host, player: PlayerRef) -> Vec<(u32, ItemStack)> {
    player
        .inventory_slots(host)
        .unwrap_or_default()
        .into_iter()
        .filter(|(slot, _)| MAIN.contains(slot))
        .filter_map(|(slot, stack)| Some((slot, events::stack_old(host, &stack)?)))
        .collect()
}

/// How many of `stack` (whatever its count) the main inventory has room for.
pub(super) fn room_for(host: &dyn mod_api::Host, player: PlayerRef, stack: &ItemStack) -> u32 {
    let Some(limit) = max_stack(host, &stack.item) else { return 0 };
    let limit = if stack.damage > 0 { 1 } else { limit };
    let held = main_slots(host, player);
    let mut room = 0;
    for slot in MAIN {
        match held.iter().find(|(index, _)| *index == slot) {
            None => room += limit,
            Some((_, have)) if stackable(have, stack) => room += limit.saturating_sub(have.count),
            Some(_) => {}
        }
    }
    room
}

/// `Inventory.add`: gives what fits and says how much that was.
pub(super) fn give(host: &mut dyn ServerHost, player: PlayerRef, stack: &ItemStack) -> u32 {
    let gave = stack.count.min(room_for(&*host, player, stack));
    if gave == 0 {
        return 0;
    }
    let Some(new) = events::stack_new(&*host, &stack.with_count(gave)) else { return 0 };
    if player.give_stack(host, new).is_ok() { gave } else { 0 }
}

/// Takes up to `count` plain `item` from the player, the selected slot first.
pub(super) fn take(host: &mut dyn ServerHost, player: PlayerRef, item: &str, count: u32) -> u32 {
    let selected = player.selected_slot(&*host).unwrap_or(0) as u32;
    let mut slots = main_slots(&*host, player);
    slots.sort_by_key(|(slot, _)| (*slot != selected, *slot));
    let mut taken = 0;
    for (slot, stack) in slots {
        if taken >= count {
            break;
        }
        if &*stack.item != item || !plain(&stack) {
            continue;
        }
        let take = stack.count.min(count - taken);
        let left = stack.with_count(stack.count - take);
        let new = if left.count == 0 { None } else { events::stack_new(&*host, &left) };
        if player.set_inventory_slot(host, slot, new).is_ok() {
            taken += take;
        }
    }
    taken
}

// --- players -------------------------------------------------------------------------------------------------

/// What the old `NativeServerPlayer` carried, read through the new handles.
pub(super) fn player_view(host: &dyn mod_api::Host, player: PlayerRef) -> NativeServerPlayer {
    let entity = player.entity();
    let living = host.living(entity);
    let abilities = player.abilities(host);
    let experience = player.experience(host);
    let [yaw, pitch] = entity.rotation(host).unwrap_or([0.0; 2]);
    NativeServerPlayer {
        uuid: player.uuid(),
        name: player.name(host).unwrap_or_default(),
        permission: player.permission_level(host).unwrap_or(0),
        dimension: player.dimension(host).map(|id| id.to_string()).unwrap_or_else(|| host.dimension().to_owned()),
        position: entity.position(host).unwrap_or([0.0; 3]),
        game_mode: player.game_mode(host).map_or(0, |mode| mode as i32),
        health: living.and_then(|living| living.health(host)).unwrap_or(20.0),
        yaw,
        pitch,
        entity_id: entity.id() as i32,
        address: player.address(host).unwrap_or_default(),
        latency: 0,
        protocol: 0,
        xp_level: experience.map_or(0, |xp| xp.level),
        xp_progress: experience.map_or(0.0, |xp| xp.progress),
        on_ground: entity.on_ground(host).unwrap_or(true),
        flying: abilities.is_some_and(|a| a.flying),
        may_fly: abilities.is_some_and(|a| a.may_fly),
        invulnerable: abilities.is_some_and(|a| a.invulnerable),
        walk_speed: abilities.map_or(0.1, |a| a.walk_speed),
        fly_speed: abilities.map_or(0.05, |a| a.fly_speed),
        sneaking: player.is_secondary_use_active(host).unwrap_or(false),
        eye_height: entity.eye_height(host).unwrap_or(1.62) as f32,
        selected_slot: player.selected_slot(host).unwrap_or(0) as u8,
        nickname: None,
        hidden: None,
        fake: player.is_fake(host).unwrap_or(false),
        using_item: living.and_then(|living| living.item_use_remaining(host)).is_some_and(|ticks| ticks > 0),
        blocking: living.and_then(|living| living.is_blocking(host)).unwrap_or(false),
        sprinting: entity.is_sprinting(host).unwrap_or(false),
        gliding: living.and_then(|living| living.is_gliding(host)).unwrap_or(false),
    }
}

// --- recipes -------------------------------------------------------------------------------------------------

type RecipeTable = HashMap<String, (std::sync::Arc<[mod_api::Recipe]>, &'static [CustomRecipe])>;
static RECIPES: LazyLock<Mutex<RecipeTable>> = LazyLock::new(Mutex::default);

/// The recipes of `kind` as the old API listed them. The game hands out the same table until a reload, so a
/// slice is built (and leaked) only when it changes.
pub(super) fn recipes(host: &dyn mod_api::Host, kind: &str) -> &'static [CustomRecipe] {
    let fresh = host.recipes_of_type(kind);
    let mut table = lock(&RECIPES);
    if let Some((old, list)) = table.get(kind) {
        if std::sync::Arc::ptr_eq(old, &fresh) {
            return list;
        }
    }
    let list: Vec<CustomRecipe> =
        fresh.iter().map(|recipe| CustomRecipe { id: recipe.id.clone(), json: recipe.json.clone() }).collect();
    let list: &'static [CustomRecipe] = Box::leak(list.into_boxed_slice());
    table.insert(kind.to_owned(), (fresh, list));
    list
}

// --- block entity slots --------------------------------------------------------------------------------------

/// The item slots of a mod block entity that declares a container (none for the others).
pub(super) fn slots_of(host: &dyn mod_api::Host, entity: BlockEntityRef) -> Vec<Option<ItemStack>> {
    // Asking for a slot past the end answers with the slot count.
    let Err(mod_api::BlockEntityItemError::SlotOutOfBounds { slots, .. }) = entity.item_slot(host, u32::MAX) else {
        return Vec::new();
    };
    (0..slots)
        .map(|slot| match entity.item_slot(host, slot) {
            Ok(Some(stack)) => events::stack_old(host, &stack),
            _ => None,
        })
        .collect()
}

// --- the level's random --------------------------------------------------------------------------------------

struct RandomPtr(*mut LegacyRandom);
// SAFETY: only the server thread draws from it, as the level's own random is.
unsafe impl Send for RandomPtr {}

static RANDOMS: LazyLock<Mutex<HashMap<String, RandomPtr>>> = LazyLock::new(Mutex::default);

/// A random of the mod's own per dimension, seeded once from the level's stream (the new API lends a copy of the
/// level's stream through `with_random` only, not a borrow that outlives the call).
pub(super) fn level_random(host: &mut dyn ServerHost, dimension: &str) -> Option<&'static mut LegacyRandom> {
    let mut table = lock(&RANDOMS);
    if !table.contains_key(dimension) {
        let seed = host.level(dimension)?.random(&*host)?;
        let seed = i64::from_ne_bytes(seed.state().to_ne_bytes());
        table.insert(dimension.to_owned(), RandomPtr(Box::leak(Box::new(LegacyRandom::new(seed)))));
    }
    let ptr = table.get(dimension)?.0;
    // SAFETY: leaked once, never freed; one thread draws.
    Some(unsafe { &mut *ptr })
}

