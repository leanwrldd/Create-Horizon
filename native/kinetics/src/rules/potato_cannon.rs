//! The Potato Cannon: `PotatoCannonItem`, `PotatoCannonProjectileType` and Create's data in
//! `data/create/create/potato_projectile/type/` (24 kinds of ammunition), as a table, with what firing
//! one does. No game types: the glue finds the ammo, spawns the projectile and applies the hit.
//!
//! Each type's numbers are Create's datagen output as the jar holds it: `damage` 1, `reload_ticks` 10,
//! `split` 1, `knockback` 1, `velocity_multiplier` 1, `sound_pitch` 1 where a file doesn't say. The block
//! hit actions (planting a crop, placing a melon or pumpkin), the chorus teleport and curing a zombie
//! villager are not in the table: see `machines/potato_cannon.rs`.

pub const POTATO_CANNON: &str = "create:potato_cannon";
/// `AllItems.POTATO_CANNON`: `durability(100)`.
pub const MAX_DAMAGE: u32 = 100;
/// `CEquipment.maxPotatoCannonShots`: free shots a full Copper Backtank gives.
pub const MAX_SHOTS: i32 = 200;
/// `PotatoCannonItem.use`: `.normalize().scale(2).scale(velocityMultiplier)`.
pub const MUZZLE_SPEED: f64 = 2.0;
/// The size of a spray's offset from the line of fire (`new Vec3(0, 0.1, 0)`).
pub const SPRAY_RADIUS: f64 = 0.1;
/// `PotatoCannonProjectileType.damage` and the damage type of a hit (`data/create/damage_type/potato_cannon.json`).
pub const DAMAGE_TYPE: &str = "create:potato_cannon";

/// A status effect a hit gives (`create:potion_effect`, or the effects of `create:food_effects`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Effect {
    pub id: &'static str,
    pub amplifier: i32,
    pub ticks: i32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Type {
    pub name: &'static str,
    pub items: &'static [&'static str],
    pub damage: i32,
    pub reload_ticks: i32,
    /// How many projectiles one shot makes.
    pub split: i32,
    pub knockback: f32,
    pub velocity_multiplier: f32,
    pub sound_pitch: f32,
    pub sticky: bool,
    /// `create:set_on_fire`: ticks the target burns for.
    pub fire_ticks: i32,
    pub effects: &'static [Effect],
    /// `drop_stack`: what falls where it hit (a bowl for the suspicious stew).
    pub drop: Option<&'static str>,
}

pub static TYPES: &[Type] = &[
    Type {
        name: "apple",
        items: &["minecraft:apple"],
        damage: 5,
        reload_ticks: 10,
        split: 1,
        knockback: 0.5,
        velocity_multiplier: 1.45,
        sound_pitch: 1.1,
        sticky: false,
        fire_ticks: 0,
        effects: &[],
        drop: None,
    },
    Type {
        name: "baked_potato",
        items: &["minecraft:baked_potato"],
        damage: 5,
        reload_ticks: 15,
        split: 1,
        knockback: 0.5,
        velocity_multiplier: 1.25,
        sound_pitch: 1.0,
        sticky: false,
        fire_ticks: 60,
        effects: &[],
        drop: None,
    },
    Type {
        name: "beetroot",
        items: &["minecraft:beetroot"],
        damage: 2,
        reload_ticks: 5,
        split: 1,
        knockback: 0.1,
        velocity_multiplier: 1.6,
        sound_pitch: 1.6,
        sticky: false,
        fire_ticks: 0,
        effects: &[],
        drop: None,
    },
    Type {
        name: "blaze_cake",
        items: &["create:blaze_cake"],
        damage: 15,
        reload_ticks: 20,
        split: 1,
        knockback: 0.3,
        velocity_multiplier: 1.1,
        sound_pitch: 1.0,
        sticky: true,
        fire_ticks: 240,
        effects: &[],
        drop: None,
    },
    Type {
        name: "cake",
        items: &["minecraft:cake"],
        damage: 8,
        reload_ticks: 15,
        split: 1,
        knockback: 0.1,
        velocity_multiplier: 1.1,
        sound_pitch: 1.0,
        sticky: true,
        fire_ticks: 0,
        effects: &[],
        drop: None,
    },
    Type {
        name: "carrot",
        items: &["minecraft:carrot"],
        damage: 4,
        reload_ticks: 12,
        split: 1,
        knockback: 0.3,
        velocity_multiplier: 1.45,
        sound_pitch: 1.5,
        sticky: false,
        fire_ticks: 0,
        effects: &[],
        drop: None,
    },
    Type {
        name: "chocolate_berry",
        items: &["create:chocolate_glazed_berries"],
        damage: 4,
        reload_ticks: 10,
        split: 3,
        knockback: 0.2,
        velocity_multiplier: 1.05,
        sound_pitch: 1.25,
        sticky: false,
        fire_ticks: 0,
        effects: &[],
        drop: None,
    },
    Type {
        name: "chorus_fruit",
        items: &["minecraft:chorus_fruit"],
        damage: 3,
        reload_ticks: 15,
        split: 1,
        knockback: 0.05,
        velocity_multiplier: 1.2,
        sound_pitch: 1.0,
        sticky: false,
        fire_ticks: 0,
        effects: &[],
        drop: None,
    },
    Type {
        name: "enchanted_golden_apple",
        items: &["minecraft:enchanted_golden_apple"],
        damage: 1,
        reload_ticks: 100,
        split: 1,
        knockback: 0.05,
        velocity_multiplier: 1.45,
        sound_pitch: 1.1,
        sticky: false,
        fire_ticks: 0,
        effects: &[Effect { id: "minecraft:regeneration", amplifier: 1, ticks: 400 }, Effect { id: "minecraft:resistance", amplifier: 0, ticks: 6000 }, Effect { id: "minecraft:fire_resistance", amplifier: 0, ticks: 6000 }, Effect { id: "minecraft:absorption", amplifier: 3, ticks: 2400 }],
        drop: None,
    },
    Type {
        name: "fish",
        items: &["minecraft:cod", "minecraft:cooked_cod", "minecraft:salmon", "minecraft:cooked_salmon", "minecraft:tropical_fish"],
        damage: 4,
        reload_ticks: 10,
        split: 1,
        knockback: 0.6,
        velocity_multiplier: 1.3,
        sound_pitch: 1.3,
        sticky: true,
        fire_ticks: 0,
        effects: &[],
        drop: None,
    },
    Type {
        name: "glistering_melon",
        items: &["minecraft:glistering_melon_slice"],
        damage: 5,
        reload_ticks: 8,
        split: 1,
        knockback: 0.1,
        velocity_multiplier: 1.45,
        sound_pitch: 1.5,
        sticky: false,
        fire_ticks: 0,
        effects: &[Effect { id: "minecraft:glowing", amplifier: 0, ticks: 100 }],
        drop: None,
    },
    Type {
        name: "glow_berry",
        items: &["minecraft:glow_berries"],
        damage: 2,
        reload_ticks: 10,
        split: 2,
        knockback: 0.05,
        velocity_multiplier: 1.05,
        sound_pitch: 1.2,
        sticky: false,
        fire_ticks: 0,
        effects: &[Effect { id: "minecraft:glowing", amplifier: 0, ticks: 200 }],
        drop: None,
    },
    Type {
        name: "golden_apple",
        items: &["minecraft:golden_apple"],
        damage: 1,
        reload_ticks: 100,
        split: 1,
        knockback: 0.05,
        velocity_multiplier: 1.45,
        sound_pitch: 1.1,
        sticky: false,
        fire_ticks: 0,
        effects: &[],
        drop: None,
    },
    Type {
        name: "golden_carrot",
        items: &["minecraft:golden_carrot"],
        damage: 12,
        reload_ticks: 15,
        split: 1,
        knockback: 0.5,
        velocity_multiplier: 1.45,
        sound_pitch: 1.5,
        sticky: false,
        fire_ticks: 0,
        effects: &[],
        drop: None,
    },
    Type {
        name: "honeyed_apple",
        items: &["create:honeyed_apple"],
        damage: 6,
        reload_ticks: 15,
        split: 1,
        knockback: 0.1,
        velocity_multiplier: 1.35,
        sound_pitch: 1.1,
        sticky: false,
        fire_ticks: 0,
        effects: &[Effect { id: "minecraft:slowness", amplifier: 1, ticks: 160 }],
        drop: None,
    },
    Type {
        name: "melon_block",
        items: &["minecraft:melon"],
        damage: 8,
        reload_ticks: 20,
        split: 1,
        knockback: 2.0,
        velocity_multiplier: 0.95,
        sound_pitch: 0.9,
        sticky: false,
        fire_ticks: 0,
        effects: &[],
        drop: None,
    },
    Type {
        name: "melon_slice",
        items: &["minecraft:melon_slice"],
        damage: 3,
        reload_ticks: 8,
        split: 1,
        knockback: 0.1,
        velocity_multiplier: 1.45,
        sound_pitch: 1.5,
        sticky: false,
        fire_ticks: 0,
        effects: &[],
        drop: None,
    },
    Type {
        name: "poison_potato",
        items: &["minecraft:poisonous_potato"],
        damage: 5,
        reload_ticks: 15,
        split: 1,
        knockback: 0.05,
        velocity_multiplier: 1.25,
        sound_pitch: 1.0,
        sticky: false,
        fire_ticks: 0,
        effects: &[Effect { id: "minecraft:poison", amplifier: 0, ticks: 160 }],
        drop: None,
    },
    Type {
        name: "potato",
        items: &["minecraft:potato"],
        damage: 5,
        reload_ticks: 15,
        split: 1,
        knockback: 1.5,
        velocity_multiplier: 1.25,
        sound_pitch: 1.0,
        sticky: false,
        fire_ticks: 0,
        effects: &[],
        drop: None,
    },
    Type {
        name: "pufferfish",
        items: &["minecraft:pufferfish"],
        damage: 4,
        reload_ticks: 10,
        split: 1,
        knockback: 0.4,
        velocity_multiplier: 1.1,
        sound_pitch: 1.1,
        sticky: true,
        fire_ticks: 0,
        effects: &[Effect { id: "minecraft:poison", amplifier: 1, ticks: 1200 }, Effect { id: "minecraft:hunger", amplifier: 2, ticks: 300 }, Effect { id: "minecraft:nausea", amplifier: 0, ticks: 300 }],
        drop: None,
    },
    Type {
        name: "pumpkin_block",
        items: &["minecraft:pumpkin"],
        damage: 6,
        reload_ticks: 15,
        split: 1,
        knockback: 2.0,
        velocity_multiplier: 0.95,
        sound_pitch: 0.9,
        sticky: false,
        fire_ticks: 0,
        effects: &[],
        drop: None,
    },
    Type {
        name: "pumpkin_pie",
        items: &["minecraft:pumpkin_pie"],
        damage: 7,
        reload_ticks: 15,
        split: 1,
        knockback: 0.05,
        velocity_multiplier: 1.1,
        sound_pitch: 1.1,
        sticky: true,
        fire_ticks: 0,
        effects: &[],
        drop: None,
    },
    Type {
        name: "suspicious_stew",
        items: &["minecraft:suspicious_stew"],
        damage: 3,
        reload_ticks: 40,
        split: 1,
        knockback: 0.2,
        velocity_multiplier: 0.8,
        sound_pitch: 1.0,
        sticky: false,
        fire_ticks: 0,
        effects: &[],
        drop: Some("minecraft:bowl"),
    },
    Type {
        name: "sweet_berry",
        items: &["minecraft:sweet_berries"],
        damage: 3,
        reload_ticks: 10,
        split: 3,
        knockback: 0.1,
        velocity_multiplier: 1.05,
        sound_pitch: 1.25,
        sticky: false,
        fire_ticks: 0,
        effects: &[],
        drop: None,
    },
];

/// `PotatoCannonProjectileType.getTypeForItem`.
pub fn type_for(item: &str) -> Option<&'static Type> {
    TYPES.iter().find(|t| t.items.contains(&item))
}

/// Where `Player.getProjectile` looks for ammo: the off hand, then the main inventory in order.
/// `slots` lists the item ids of the off hand and then the 36 inventory slots (`None` for an empty one).
pub fn find_ammo(slots: &[Option<&str>]) -> Option<(usize, &'static Type)> {
    slots.iter().enumerate().find_map(|(i, id)| type_for((*id)?).map(|t| (i, t)))
}

/// The speed a shot leaves at: the lookvector's length (2) times the type's multiplier.
pub fn muzzle_speed(t: &Type) -> f64 {
    MUZZLE_SPEED * f64::from(t.velocity_multiplier)
}

/// The angle round the line of fire of each projectile of a spray, in degrees: a random start
/// (`360 * nextFloat`), turns of `360 / split`, and `40 * (nextFloat - .5)` of imperfection each.
/// `base_roll` and `jitter_rolls` are `nextFloat()` values. Empty for a single shot.
pub fn spray_angles(split: i32, base_roll: f32, jitter_rolls: &[f32]) -> Vec<f32> {
    if split <= 1 {
        return Vec::new();
    }
    let base = 360.0 * base_roll;
    let step = 360.0 / split as f32;
    (0..split as usize)
        .map(|i| base + i as f32 * step + 40.0 * (jitter_rolls.get(i).copied().unwrap_or(0.5) - 0.5))
        .collect()
}

/// `recoveryChance` is 0 for all but the first of a spray; Create's enchantment (Potato Recovery) isn't
/// in the port, so nothing is recovered.
pub fn extra_knockback_push(t: &Type, flight: [f64; 2]) -> Option<[f64; 3]> {
    let len = (flight[0] * flight[0] + flight[1] * flight[1]).sqrt();
    if len == 0.0 || t.knockback <= 0.0 {
        return None;
    }
    let strength = f64::from(t.knockback) * 0.6;
    Some([flight[0] / len * strength, (strength).min(0.4), flight[1] / len * strength])
}

/// `BacktankUtil.canAbsorbDamage`'s cost of a shot, in air.
pub fn backtank_cost(max_air: i32) -> i32 {
    (max_air / MAX_SHOTS).max(1)
}

/// What a shot does to the cannon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wear {
    Damaged(u32),
    Broken,
}

pub fn wear(damage: u32, shots: u32) -> Wear {
    let total = damage.saturating_add(shots);
    if total >= MAX_DAMAGE { Wear::Broken } else { Wear::Damaged(total) }
}

/// A UUID as a command wants it (`8-4-4-4-12` hex).
pub fn uuid_text(uuid: u128) -> String {
    let h = format!("{uuid:032x}");
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// `effect give <target> <id> <seconds> <amplifier> true` for one of a hit's effects.
pub fn effect_command(uuid: u128, e: &Effect) -> String {
    format!("effect give {} {} {} {} true", uuid_text(uuid), e.id, (e.ticks / 20).max(1), e.amplifier)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_type_of_ammunition_is_in() {
        assert_eq!(TYPES.len(), 24);
        assert!(type_for("minecraft:potato").is_some());
        assert!(type_for("minecraft:cod").is_some_and(|t| t.name == "fish"));
        assert!(type_for("minecraft:salmon").is_some_and(|t| t.name == "fish"));
        assert!(type_for("minecraft:stone").is_none());
        // Every item belongs to one type only.
        for t in TYPES {
            for item in t.items {
                assert_eq!(TYPES.iter().filter(|o| o.items.contains(item)).count(), 1, "{item}");
            }
        }
    }

    #[test]
    fn the_potato_is_create_s_numbers() {
        let t = type_for("minecraft:potato").unwrap();
        assert_eq!((t.damage, t.reload_ticks, t.split), (5, 15, 1));
        assert_eq!(t.knockback, 1.5);
        assert_eq!(t.velocity_multiplier, 1.25);
        assert!((muzzle_speed(t) - 2.5).abs() < 1e-9);
    }

    #[test]
    fn defaults_apply_where_a_file_is_silent() {
        // `golden_apple` has no damage; the default is 1 and it reloads for 100.
        let t = type_for("minecraft:golden_apple").unwrap();
        assert_eq!((t.damage, t.reload_ticks), (1, 100));
        // `apple` doesn't give a reload time: 10.
        assert_eq!(type_for("minecraft:apple").unwrap().reload_ticks, 10);
    }

    #[test]
    fn sprays_split_into_several_projectiles() {
        assert_eq!(type_for("minecraft:sweet_berries").unwrap().split, 3);
        assert_eq!(type_for("minecraft:glow_berries").unwrap().split, 2);
        assert!(spray_angles(1, 0.3, &[]).is_empty());
        let a = spray_angles(3, 0.0, &[0.5, 0.5, 0.5]);
        assert_eq!(a, vec![0.0, 120.0, 240.0]);
        let b = spray_angles(2, 0.5, &[1.0, 0.0]);
        assert_eq!(b, vec![180.0 + 20.0, 360.0 - 20.0]);
    }

    #[test]
    fn fire_and_effects_are_in_the_table() {
        assert_eq!(type_for("minecraft:baked_potato").unwrap().fire_ticks, 60);
        assert_eq!(type_for("create:blaze_cake").unwrap().fire_ticks, 240);
        let poison = type_for("minecraft:poisonous_potato").unwrap();
        assert_eq!(poison.effects, &[Effect { id: "minecraft:poison", amplifier: 0, ticks: 160 }]);
        assert_eq!(type_for("minecraft:enchanted_golden_apple").unwrap().effects.len(), 4);
        assert_eq!(type_for("minecraft:suspicious_stew").unwrap().drop, Some("minecraft:bowl"));
    }

    #[test]
    fn ammunition_is_found_in_the_off_hand_first_then_in_order() {
        let slots = [Some("minecraft:stone"), Some("minecraft:carrot"), Some("minecraft:potato")];
        assert_eq!(find_ammo(&slots).map(|(i, t)| (i, t.name)), Some((1, "carrot")));
        assert!(find_ammo(&[None, Some("minecraft:dirt")]).is_none());
    }

    #[test]
    fn the_extra_knockback_pushes_along_the_flight() {
        let potato = type_for("minecraft:potato").unwrap();
        let push = extra_knockback_push(potato, [0.0, 3.0]).unwrap();
        assert!((push[2] - 0.9).abs() < 1e-9 && push[0].abs() < 1e-9);
        assert_eq!(push[1], 0.4);
        assert!(extra_knockback_push(potato, [0.0, 0.0]).is_none());
    }

    #[test]
    fn a_backtank_pays_four_air_a_shot_and_the_cannon_breaks_after_a_hundred() {
        assert_eq!(backtank_cost(900), 4);
        assert_eq!(wear(98, 1), Wear::Damaged(99));
        assert_eq!(wear(99, 1), Wear::Broken);
    }

    #[test]
    fn commands_name_the_target_by_uuid() {
        assert_eq!(uuid_text(1), "00000000-0000-0000-0000-000000000001");
        let e = Effect { id: "minecraft:slowness", amplifier: 1, ticks: 160 };
        assert_eq!(effect_command(0xab, &e), "effect give 00000000-0000-0000-0000-0000000000ab minecraft:slowness 8 1 true");
    }
}
