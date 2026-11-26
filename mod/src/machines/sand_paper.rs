//! Sand Paper and Red Sand Paper (`SandPaperItem`, `SandPaperPolishingRecipe`): hold the right
//! mouse button for 32 ticks with something to polish in the other hand, or lying on the floor
//! where you look, and the paper polishes it with a `create:sandpaper_polishing` recipe
//! (Rose Quartz into Polished Rose Quartz) and wears by one of its 8. A Deployer holding the
//! paper polishes the items on a belt or depot below it (`deployer_apply.rs`). The rules are
//! `create_kinetics::rules::sandpaper`; this is the glue.
//!
//! Known differences from Create:
//! - Create takes the item out of the world when the use starts, keeps it in the paper
//!   (`AllDataComponents.SAND_PAPER_POLISHING`) and gives it back if the use is let go early.
//!   Horizon can't change the stack in the hand from a use hook (`take_items` works only for a
//!   plain stack, and not on the paper itself), so nothing is taken until the use is over: the
//!   item is looked for again then, and polished if it is still in the other hand or on the floor
//!   within reach. Letting go early costs nothing; swapping the item away during the use
//!   cancels the polish. The paper does not show the item it is polishing.
//! - The use starts only when there is something to polish (`use_item` claims the click
//!   otherwise, as `FAIL`), but a client predicts the eating pose for the click and shows it until
//!   the server's answer; the pose may run its 32 ticks with nothing to polish.
//! - No item particles (Horizon has no item particle for a mod to send) and no
//!   `AllSoundEvents.SANDING_SHORT` through the item's own eating sound: the tick plays
//!   `create:sanding_short` on the schedule `triggerUseEffects` has.
//! - The paper doesn't scrape oxidised copper or take wax off (`useOn`, `ItemAbilities.AXE_SCRAPE`
//!   and `AXE_WAX_OFF`): Horizon can't ask a block for its tool-modified state. A Deployer with an
//!   axe does it (`deploying/*_from_deoxidising`).
//! - The crafting remainder of the polished item is not returned (`getCraftingRemainingItem`),
//!   and an unbreaking enchantment on the paper is not read.

use create_kinetics::rules::drill::is_air;
use create_kinetics::rules::sandpaper::{self as rules, Wear};
use crate::compat::{
    EquipmentSlot, Flow, Hand, Hooks, Host, ItemStack, LivingEntityUseItemFinishEvent,
    LivingEntityUseItemTickEvent, NativeServerPlayer, RandomSource, UseAnimation,
    UseAnimationEvent, UseDurationEvent, UseItemEvent,
};

use crate::{recipes, util};

/// `AllRecipeTypes.SANDPAPER_POLISHING`.
pub const POLISHING: &str = "create:sandpaper_polishing";
/// `SoundSource.PLAYERS`.
const PLAYERS: usize = 7;

struct SandPaper;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(SandPaper)
}

/// `SandPaperPolishingRecipe.getMatchingRecipes(..)[0].assemble`: what polishing one of `stack`
/// makes, or `None` when no recipe takes it.
pub(crate) fn polished(host: &dyn Host, stack: &ItemStack) -> Option<ItemStack> {
    let recipe = recipes::for_stack(host, POLISHING, stack)?;
    recipe.results.first().map(|output| output.stack.clone())
}

/// What a use of the paper is going to polish.
enum Target {
    /// One of the item in the other hand.
    Hand(ItemStack),
    /// One of the stack of the item entity `id`.
    Floor { id: u32, stack: ItemStack },
}

impl Target {
    fn stack(&self) -> &ItemStack {
        match self {
            Target::Hand(stack) | Target::Floor { stack, .. } => stack,
        }
    }
}

fn solid_for_ray(block: &str) -> bool {
    // `ClipContext.Fluid.NONE`: fluids don't stop the ray.
    !is_air(block) && block != "minecraft:water" && block != "minecraft:lava"
}

/// `getPlayerPOVHitResult(.., Fluid.NONE)`: how far along the eye ray the first block is, in steps
/// of a tenth of a block; the reach where there is none.
fn hit_distance(host: &dyn Host, eye: [f64; 3], direction: [f64; 3]) -> f64 {
    let mut distance = 0.0;
    while distance <= rules::REACH {
        let at = [0, 1, 2].map(|i| (eye[i] + direction[i] * distance).floor() as i32);
        if solid_for_ray(&host.block(at)) {
            return distance;
        }
        distance += 0.1;
    }
    rules::REACH
}

/// `SandPaperItem.use`: the item in the other hand if a recipe takes it, else the first item on
/// the floor around where the player looks that one does. `main` is the hand the paper is in.
fn find_target(host: &mut dyn Host, player: &NativeServerPlayer, main: bool) -> Option<Target> {
    let other = if main {
        host.player_equipment(player.uuid, EquipmentSlot::OffHand)
    } else {
        host.held_stack(player.uuid)
    };
    // Only a plain stack can be taken from a hand in a player's hook (`take_items`).
    if let Some(stack) = other
        && stack.count > 0
        && stack.extra.is_none()
        && polished(&*host, &stack).is_some()
    {
        return Some(Target::Hand(stack));
    }
    let eye = [player.position[0], player.position[1] + f64::from(player.eye_height), player.position[2]];
    let direction = rules::view_vector(player.yaw, player.pitch);
    let hit = rules::hit_point(eye, direction, hit_distance(&*host, eye, direction));
    let (min, max) = rules::pick_up_box(hit);
    let dimension = host.dimension();
    let items = host.item_entities(dimension, min, max);
    let positions: Vec<[f64; 3]> = items.iter().map(|item| item.position).collect();
    let index = rules::pick_item(player.position, &positions, |i| polished(&*host, &items[i].stack).is_some())?;
    let item = items.into_iter().nth(index)?;
    Some(Target::Floor { id: item.id, stack: item.stack })
}

impl Hooks for SandPaper {
    /// `SandPaperItem.getUseDuration`: the paper is used over time, on both sides.
    fn use_duration(&self, _host: &dyn Host, event: &mut UseDurationEvent<'_>) {
        if rules::is_sand_paper(&event.stack.item) {
            event.duration = rules::USE_DURATION;
        }
    }

    /// `SandPaperItem.getUseAnimation`.
    fn use_animation(&self, _host: &dyn Host, event: &mut UseAnimationEvent<'_>) {
        if rules::is_sand_paper(&event.stack.item) {
            *event.animation = UseAnimation::Eat;
        }
    }

    /// `SandPaperItem.use`: the use starts only with something to polish (the click is claimed,
    /// as Create's `FAIL`, otherwise).
    fn use_item(&self, host: &mut dyn Host, event: &mut UseItemEvent<'_>) -> Flow {
        if !rules::is_sand_paper(&event.stack.item) {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        // The paper is in the main hand unless the main hand holds something else.
        let main = host.held_stack(player.uuid).is_some_and(|held| rules::is_sand_paper(&held.item));
        if find_target(host, &player, main).is_some() { Flow::Continue } else { Flow::Cancel }
    }

    /// `SandPaperItem.triggerUseEffects`: the sanding sound after six ticks, every seventh.
    fn living_entity_use_item_tick(
        &self,
        host: &mut dyn Host,
        event: &mut LivingEntityUseItemTickEvent<'_>,
    ) -> Flow {
        if host.window() || !rules::is_sand_paper(&event.stack.item) {
            return Flow::Continue;
        }
        let used = rules::USE_DURATION - event.duration;
        if rules::sounds_now(used)
            && let Some(player) = host.acting_player()
        {
            let dimension = host.dimension();
            let (volume, pitch) = match host.level_random(dimension) {
                Some(random) => rules::sound_levels(random.next_float(), random.next_float()),
                None => (1.0, 1.0),
            };
            util::create_sound(host, "sanding_short", player.position, volume, pitch);
        }
        Flow::Continue
    }

    /// `SandPaperItem.finishUsingItem`: the polished item goes to the player, the paper wears.
    fn living_entity_use_item_finish(
        &self,
        host: &mut dyn Host,
        event: &mut LivingEntityUseItemFinishEvent<'_>,
    ) {
        if host.window() || !rules::is_sand_paper(&event.stack.item) {
            return;
        }
        let Some(player) = host.acting_player() else {
            return;
        };
        if player.game_mode == crate::util::SPECTATOR {
            return;
        }
        let Some(target) = find_target(host, &player, event.hand == Hand::Main) else {
            return;
        };
        let Some(made) = polished(&*host, target.stack()) else {
            return;
        };
        // Take one of it.
        let taken = match &target {
            Target::Hand(stack) => host.take_items(&stack.item, 1) == 1,
            Target::Floor { id, stack } => {
                let dimension = host.dimension();
                let mut rest = stack.clone();
                rest.count -= 1;
                host.set_item_entity(dimension, *id, rest)
            }
        };
        if !taken {
            return;
        }
        // `playerInv.placeItemBackInInventory(polished)`: what doesn't fit drops at the player.
        if let Some(rest) = host.give_stack(player.uuid, made) {
            crate::util::spawn_item(host, player.position, &rest, [0.0; 3], 0);
        }
        // `stack.hurtAndBreak(1, ..)`: no wear for a creative player.
        if player.game_mode == crate::util::CREATIVE {
            return;
        }
        let paper = event.result.clone().unwrap_or_else(|| event.stack.clone());
        match rules::wear(paper.damage) {
            Wear::Damaged(damage) => {
                let mut worn = paper;
                worn.damage = damage;
                *event.result = Some(worn);
            }
            Wear::Broken => {
                *event.result = None;
                host.play_sound("minecraft:entity.item.break", player.position, PLAYERS, 0.8, 1.0);
            }
        }
    }
}
