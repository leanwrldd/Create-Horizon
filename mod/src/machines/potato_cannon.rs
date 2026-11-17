//! The Potato Cannon (`PotatoCannonItem`, `PotatoProjectileTypeManager`, `PotatoProjectileEntity`): a
//! right click fires one item of ammunition from the off hand or the inventory (Create's 24 types: potato,
//! baked potato, carrot, apple, fish, melon, pumpkin, berries, golden apples...) as a projectile that
//! hurts what it hits. The table and the numbers are `create_kinetics::rules::potato_cannon`; this is
//! the glue.
//!
//! **The stand-in for Create's projectile entity.** Horizon gives a mod no entity of its own, so a shot
//! is a vanilla snowball summoned at the barrel with the ammunition as its item (a snowball shows the
//! item it carries, so a potato flies as a potato). Its impact is heard (`projectile_impact`), cancelled and
//! done by the mod: `hurt_entity` with the `create:potato_cannon` damage type, burning, status effects and
//! extra knockback from the type's table, and the snowball removed.
//!
//! Known differences from Create, for the README:
//! - A snowball flies with vanilla's gravity (0.03) and no drag where Create's projectile uses 0.05 and
//!   0.99, doesn't spin or tumble (`PotatoProjectileRenderMode`), leaves no air trail and doesn't stick
//!   to what it hits (`sticky`); it just vanishes. It never drops the item back (no Potato Recovery
//!   enchantment), except the suspicious stew's bowl.
//! - Not done: planting crops and placing melons and pumpkins where a block is hit
//!   (`on_block_hit`), the chorus fruit's teleport, curing a zombie villager with a golden apple, and
//!   the Power and Punch enchantment bonuses. The hit effects that are potion effects (poison, glowing,
//!   slowness, the golden apples' and pufferfish's) are given with `/effect`.
//! - The shot leaves from a point beside the eye, not Create's barrel (`getGunBarrelVec`), the cannon
//!   isn't held in Create's pose and has no recoil animation.
//! - The cooldown is the type's `reload_ticks`, kept by the mod (Horizon gives a mod no item cooldown).
//! - The shot wears the cannon by 1 of 100 (written on the next inventory tick, main hand only) unless a
//!   worn Backtank pays 4 air (`maxPotatoCannonShots`); creative players use nothing up.
//! - The ammunition is taken from the first matching stack of the inventory (`take_items`), the off hand
//!   not looked at first.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use create_kinetics::rules::backtank as tank;
use create_kinetics::rules::potato_cannon::{self as rules, Type, Wear};
use create_kinetics::rules::sandpaper::view_vector;
use crate::compat::{
    Compound, Flow, HearsProjectileImpactsEvent, Hooks, Host, InventoryTickEvent, ItemStack,
    LevelTickPostEvent, ProjectileImpactEvent, RandomSource, Tag, UseItemEvent,
};

use super::diving;
use crate::util;

struct Cannon;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Cannon)
}

#[derive(Clone)]
struct Shot {
    kind: usize,
    owner: u32,
    born: i64,
    dimension: &'static str,
}

/// Projectiles in flight, by entity id.
static SHOTS: Mutex<BTreeMap<u32, Shot>> = Mutex::new(BTreeMap::new());
/// When each player may fire again.
static READY: Mutex<BTreeMap<u128, i64>> = Mutex::new(BTreeMap::new());
/// Shots waiting to be written to the held cannon.
static WEAR: Mutex<BTreeMap<u128, u32>> = Mutex::new(BTreeMap::new());
static WEAR_COUNT: AtomicUsize = AtomicUsize::new(0);

fn lock<T>(m: &'static Mutex<T>) -> std::sync::MutexGuard<'static, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn is_cannon(stack: &ItemStack) -> bool {
    &*stack.item == rules::POTATO_CANNON
}

fn index_of(t: &Type) -> usize {
    rules::TYPES.iter().position(|o| o.name == t.name).unwrap_or(0)
}

/// A vector of length 1 at `angle` degrees round `axis`, perpendicular to it: the spray's direction.
fn perpendicular(axis: [f64; 3], angle: f32) -> [f64; 3] {
    let up = if axis[1].abs() > 0.99 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    let cross = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let norm = |v: [f64; 3]| {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        [v[0] / l, v[1] / l, v[2] / l]
    };
    let right = norm(cross(axis, up));
    let up = norm(cross(right, axis));
    let (s, c) = f64::from(angle).to_radians().sin_cos();
    [0, 1, 2].map(|i| right[i] * c + up[i] * s)
}

impl Hooks for Cannon {
    /// `PotatoCannonItem.use`.
    fn use_item(&self, host: &mut dyn Host, event: &mut UseItemEvent<'_>) -> Flow {
        if !is_cannon(event.stack) {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == util::SPECTATOR {
            return Flow::Continue;
        }
        let uuid = player.uuid;
        let now = util::game_time(&*host);
        // One shot for a click, whichever hand the game tries (`use_item` is heard for each).
        if lock(&READY).get(&uuid).is_some_and(|ready| *ready > now) {
            return Flow::Cancel;
        }
        // Ammunition: the off hand, then the inventory in order.
        let creative = player.game_mode == util::CREATIVE;
        let mut slots: Vec<Option<String>> = vec![host.player_equipment(uuid, crate::compat::EquipmentSlot::OffHand).filter(|s| s.extra.is_none()).map(|s| s.item.to_string())];
        slots.extend(host.player_stacks(uuid).unwrap_or_default().into_iter().map(|s| s.filter(|s| s.extra.is_none()).map(|s| s.item.to_string())));
        let ids: Vec<Option<&str>> = slots.iter().map(Option::as_deref).collect();
        let Some((_, kind)) = rules::find_ammo(&ids) else {
            return Flow::Continue;
        };
        let item = kind.items.iter().find(|i| ids.iter().any(|s| *s == Some(**i))).copied().unwrap_or(kind.items[0]);
        if !creative && host.take_items(item, 1) == 0 {
            return Flow::Continue;
        }

        // The projectile(s).
        let dimension = host.dimension();
        let look = view_vector(player.yaw, player.pitch);
        let eye = [player.position[0], player.position[1] + f64::from(player.eye_height), player.position[2]];
        // A point ahead of the eye, a little to the right and low (`getGunBarrelVec(.75, -.15, 1.5)`).
        let right = [-look[2], 0.0, look[0]];
        let rl = (right[0] * right[0] + right[2] * right[2]).sqrt().max(1e-6);
        let barrel = [
            eye[0] + look[0] * 1.5 + right[0] / rl * 0.4,
            eye[1] + look[1] * 1.5 - 0.15,
            eye[2] + look[2] * 1.5 + right[2] / rl * 0.4,
        ];
        let speed = rules::muzzle_speed(kind);
        let motion = look.map(|c| c * speed);
        let (base_roll, jitter, pitch_roll) = match host.level_random(dimension) {
            Some(random) => (random.next_float(), (0..kind.split.max(1)).map(|_| random.next_float()).collect::<Vec<_>>(), random.next_float()),
            None => (0.0, vec![0.5; kind.split.max(1) as usize], 0.5),
        };
        let angles = rules::spray_angles(kind.split, base_roll, &jitter);
        for i in 0..kind.split.max(1) as usize {
            let mut m = motion;
            if let Some(angle) = angles.get(i) {
                let offset = perpendicular(look, *angle);
                for k in 0..3 {
                    m[k] += offset[k] * rules::SPRAY_RADIUS * speed;
                }
            }
            let mut nbt = Compound::new();
            nbt.insert("Motion".into(), Tag::List(m.iter().map(|c| Tag::Double(c.to_bits())).collect()));
            let mut stack = Compound::new();
            stack.insert("id".into(), Tag::String(item.into()));
            stack.insert("count".into(), Tag::Int(1));
            nbt.insert("Item".into(), Tag::Compound(stack));
            if let Some(id) = host.summon(dimension, "minecraft:snowball", barrel, nbt) {
                lock(&SHOTS).insert(id, Shot { kind: index_of(kind), owner: player.entity_id as u32, born: now, dimension });
            }
        }
        host.play_sound("create:fwoomp", barrel, 7, 1.0, kind.sound_pitch + (pitch_roll - 0.5) / 4.0);

        lock(&READY).insert(uuid, now + i64::from(kind.reload_ticks));
        // Wear, or a worn backtank's air.
        if !creative && !diving::owe_air(uuid, rules::backtank_cost(tank::AIR_IN_BACKTANK)) {
            let mut wear = lock(&WEAR);
            *wear.entry(uuid).or_insert(0) += 1;
            WEAR_COUNT.store(wear.len(), Ordering::Relaxed);
        }
        Flow::Cancel
    }

    /// Asked once a tick: only a game with shots in the air wants to hear impacts.
    fn hears_projectile_impacts(&self, _: &dyn Host, event: &mut HearsProjectileImpactsEvent) {
        if !lock(&SHOTS).is_empty() {
            event.hears = true;
        }
    }

    /// `PotatoProjectileEntity.onHitEntity` and `onHitBlock`: the shot lands.
    fn projectile_impact(&self, host: &mut dyn Host, event: &mut ProjectileImpactEvent<'_>) -> Flow {
        let Some(shot) = lock(&SHOTS).remove(&event.projectile) else {
            return Flow::Continue;
        };
        let kind = &rules::TYPES[shot.kind];
        let dimension = shot.dimension;
        if let Some(target) = event.entity {
            if target == shot.owner && shot.owner != 0 {
                // It can't hit its owner: it flies on.
                lock(&SHOTS).insert(event.projectile, shot);
                return Flow::Cancel;
            }
            let attacker = (shot.owner != 0).then_some(shot.owner);
            if kind.damage > 0 {
                host.hurt_entity(dimension, target, kind.damage as f32, rules::DAMAGE_TYPE, attacker);
            }
            if let Some(entity) = host.entity(target) {
                if kind.fire_ticks > 0 {
                    entity.set_remaining_fire_ticks(host.new_server(), kind.fire_ticks / 20 * 20);
                }
                if let Some(uuid) = entity.uuid(host.new_server()) {
                    for effect in kind.effects {
                        host.run_command(&rules::effect_command(uuid, effect));
                    }
                }
                let flight = host.entity(event.projectile).and_then(|p| p.velocity(host.new_host())).map_or([0.0, 0.0], |v| [v[0], v[2]]);
                if let Some(push) = rules::extra_knockback_push(kind, flight) {
                    entity.push(host.new_server(), push);
                }
            }
        }
        if let Some(drop) = kind.drop {
            let stack = ItemStack { item: drop.into(), count: 1, damage: 0, extra: None };
            util::spawn_item(host, event.at, &stack, [0.0; 3], 10);
        }
        host.play_sound("create:potato_hit", event.at, 7, 0.9, 1.0);
        host.remove_entity(dimension, event.projectile);
        Flow::Cancel
    }

    /// Forgets shots that never landed.
    fn level_tick_post(&self, host: &mut dyn Host, event: &mut LevelTickPostEvent<'_>) {
        let now = util::game_time(&*host);
        lock(&SHOTS).retain(|_, shot| shot.dimension != event.dimension || now - shot.born < 400);
        lock(&READY).retain(|_, ready| *ready > now - 200);
    }

    /// The shots are written to the held cannon.
    fn inventory_tick(&self, host: &mut dyn Host, event: &mut InventoryTickEvent) {
        if !event.selected || WEAR_COUNT.load(Ordering::Relaxed) == 0 {
            return;
        }
        let Some(player) = host.acting_player() else {
            return;
        };
        let Some(shots) = lock(&WEAR).get(&player.uuid).copied() else {
            return;
        };
        let Some(stack) = host.carried_stack(event.slot) else {
            return;
        };
        if !is_cannon(stack) {
            return;
        }
        match rules::wear(stack.damage, shots) {
            Wear::Damaged(damage) => stack.damage = damage,
            Wear::Broken => stack.count = 0,
        }
        let mut wear = lock(&WEAR);
        wear.remove(&player.uuid);
        WEAR_COUNT.store(wear.len(), Ordering::Relaxed);
    }
}
