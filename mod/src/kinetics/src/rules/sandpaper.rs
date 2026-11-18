//! Sand Paper and Red Sand Paper: `SandPaperItem` and `SandPaperPolishingRecipe`, with no game
//! types. What the item picks to polish, how long it takes, how it wears, and when it sounds.
//! The use itself (the hooks, the inventory) is `machines/sand_paper.rs`.
//!
//! Create's `SandPaperItem` is `Item.use`: with a polishable item in the other hand, or one lying
//! on the floor where the player looks, it starts using the item for `getUseDuration` = 32 ticks,
//! then `finishUsingItem` polishes it, gives the result and wears the paper by one of 8.

pub const SAND_PAPER: &str = "create:sand_paper";
pub const RED_SAND_PAPER: &str = "create:red_sand_paper";

/// `AllItems.SAND_PAPER`: `properties.durability(8)`.
pub const DURABILITY: u32 = 8;
/// `SandPaperItem.getUseDuration`.
pub const USE_DURATION: i32 = 32;
/// `SandPaperItem.use`: the item entity must be within 3 blocks of the player.
pub const PICK_UP_DISTANCE: f64 = 3.0;
/// `SandPaperItem.use`: the floor item is looked for in a box `inflate(1)` around where the
/// player looks.
pub const PICK_UP_INFLATE: f64 = 1.0;
/// `Player.blockInteractionRange` (`getPlayerPOVHitResult`): how far the ray looks.
pub const REACH: f64 = 4.5;

/// Whether `id` is a `SandPaperItem` (`AllItemTags.SANDPAPER` holds the same two).
pub fn is_sand_paper(id: &str) -> bool {
    id == SAND_PAPER || id == RED_SAND_PAPER
}

/// What wearing the paper by one (`hurtAndBreak(1, ..)`) leaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wear {
    Damaged(u32),
    Broken,
}

pub fn wear(damage: u32) -> Wear {
    let total = damage.saturating_add(1);
    if total >= DURABILITY { Wear::Broken } else { Wear::Damaged(total) }
}

/// `SandPaperItem.triggerUseEffects`: after 6 ticks of use the sound plays every 7th tick.
/// `ticks_using` is `LivingEntity.getTicksUsingItem`.
pub fn sounds_now(ticks_using: i32) -> bool {
    (ticks_using - 6) % 7 == 0
}

/// `Entity.playSound(.., 0.9F + 0.2F * nextFloat, nextFloat * 0.2F + 0.9F)` of the use: the
/// volume and the pitch for two rolls.
pub fn sound_levels(volume_roll: f32, pitch_roll: f32) -> (f32, f32) {
    (0.9 + 0.2 * volume_roll, pitch_roll * 0.2 + 0.9)
}

/// The unit vector a player looks along (`Entity.getViewVector`): yaw 0 faces south (+z).
pub fn view_vector(yaw: f32, pitch: f32) -> [f64; 3] {
    let (yaw, pitch) = (f64::from(yaw).to_radians(), f64::from(pitch).to_radians());
    [-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos()]
}

/// Where the ray `eye` + `direction` * `distance` ends: the point a floor item is looked for
/// around. `distance` is where it met a block, or [`REACH`] where it met none
/// (`BlockHitResult.getLocation` of a miss is the ray's end).
pub fn hit_point(eye: [f64; 3], direction: [f64; 3], distance: f64) -> [f64; 3] {
    [0, 1, 2].map(|i| eye[i] + direction[i] * distance)
}

/// The box `getEntitiesOfClass(ItemEntity.class, new AABB(hit, hit).inflate(1))`: min and max.
pub fn pick_up_box(hit: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    (hit.map(|v| v - PICK_UP_INFLATE), hit.map(|v| v + PICK_UP_INFLATE))
}

/// Which of the item entities found in [`pick_up_box`] the player picks: the first that is
/// within [`PICK_UP_DISTANCE`] of them and that `polishable` takes. `items` are positions, in the
/// order the game listed them.
pub fn pick_item(player: [f64; 3], items: &[[f64; 3]], polishable: impl Fn(usize) -> bool) -> Option<usize> {
    items.iter().enumerate().find_map(|(index, at)| {
        let distance = (0..3).map(|i| (at[i] - player[i]).powi(2)).sum::<f64>().sqrt();
        (distance <= PICK_UP_DISTANCE && polishable(index)).then_some(index)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_papers_are_sand_paper() {
        assert!(is_sand_paper("create:sand_paper"));
        assert!(is_sand_paper("create:red_sand_paper"));
        assert!(!is_sand_paper("create:rose_quartz"));
    }

    #[test]
    fn the_paper_breaks_on_its_eighth_use() {
        let mut damage = 0;
        for use_number in 1..=8 {
            match wear(damage) {
                Wear::Damaged(next) => {
                    assert!(use_number < 8, "still there after eight uses");
                    damage = next;
                }
                Wear::Broken => assert_eq!(use_number, 8),
            }
        }
    }

    #[test]
    fn the_sound_starts_after_six_ticks_and_repeats_every_seventh() {
        let ticks: Vec<i32> = (0..USE_DURATION).filter(|&t| sounds_now(t)).collect();
        assert_eq!(ticks, [6, 13, 20, 27]);
    }

    #[test]
    fn sound_levels_stay_in_create_s_ranges() {
        assert_eq!(sound_levels(0.0, 0.0), (0.9, 0.9));
        let (volume, pitch) = sound_levels(1.0, 1.0);
        assert!((volume - 1.1).abs() < 1e-6 && (pitch - 1.1).abs() < 1e-6);
    }

    #[test]
    fn the_view_vector_follows_the_player() {
        let south = view_vector(0.0, 0.0);
        assert!((south[2] - 1.0).abs() < 1e-9 && south[0].abs() < 1e-9);
        let down = view_vector(0.0, 90.0);
        assert!((down[1] + 1.0).abs() < 1e-9);
        let west = view_vector(90.0, 0.0);
        assert!((west[0] + 1.0).abs() < 1e-9);
    }

    #[test]
    fn the_hit_and_the_box_around_it() {
        let hit = hit_point([0.0, 1.62, 0.0], [0.0, 0.0, 1.0], 2.0);
        assert_eq!(hit, [0.0, 1.62, 2.0]);
        let (min, max) = pick_up_box(hit);
        let near = |a: [f64; 3], b: [f64; 3]| (0..3).all(|i| (a[i] - b[i]).abs() < 1e-9);
        assert!(near(min, [-1.0, 0.62, 1.0]));
        assert!(near(max, [1.0, 2.62, 3.0]));
    }

    #[test]
    fn the_first_polishable_item_within_three_blocks_is_picked() {
        let player = [0.0, 0.0, 0.0];
        let items = [[0.0, 0.0, 3.5], [0.0, 0.0, 2.0], [1.0, 0.0, 1.0]];
        // The first is too far, the second isn't polishable, the third is.
        assert_eq!(pick_item(player, &items, |i| i == 2), Some(2));
        assert_eq!(pick_item(player, &items, |_| true), Some(1));
        assert_eq!(pick_item(player, &items, |_| false), None);
    }
}
