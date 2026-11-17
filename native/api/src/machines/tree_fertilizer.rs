//! Tree Fertilizer (`TreeFertilizerItem`): right-click a sapling and it grows into its tree at
//! once, from any soil. The rules (which tree, which saplings make a square) are
//! `create_kinetics::rules::tree_fertilizer`; its header lists the known differences from Create:
//! the tree is placed with the game's `/place feature`, so the usual room checks apply, and the
//! sapling is put back (and the item dropped) when the tree doesn't fit.
//!
//! Horizon refuses a command inside a player's hook, so the click is queued and the tree is
//! placed in `level_tick_post`.

use std::sync::{LazyLock, Mutex};

use create_kinetics::Pos;
use create_kinetics::rules::tree_fertilizer as rules;
use horizon_client::mod_api::{
    Flow, Hooks, Host, ItemStack, LevelTickPostEvent, ParticleOptions, ParticleType, RandomSource,
    Text, UseItemOnEvent,
};

use crate::util;

pub const ID: &str = "create:tree_fertilizer";

/// A tree waiting to be placed.
struct Growth {
    dimension: String,
    /// Each sapling that makes way, with the block it was.
    saplings: Vec<(Pos, String)>,
    origin: Pos,
    feature: &'static str,
}

static QUEUE: LazyLock<Mutex<Vec<Growth>>> = LazyLock::new(Default::default);

struct TreeFertilizer;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(TreeFertilizer)
}

fn tell(host: &mut dyn Host, line: &str) {
    if let Some(player) = host.acting_player() {
        host.action_bar(player.uuid, Text::literal(line.to_owned()));
    }
}

impl Hooks for TreeFertilizer {
    /// `TreeFertilizerItem.useOn`.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != ID {
            return Flow::Continue;
        }
        let pos = event.pos;
        let name = host.block(pos);
        let Some(grower) = rules::grower(&name) else {
            return Flow::Continue;
        };
        // `MangrovePropaguleBlock.HANGING`.
        if !rules::can_fertilise(host.block_property(pos, "hanging").as_deref() == Some("true")) {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if !util::first_click(host, player.uuid, pos) {
            return Flow::Cancel;
        }
        let dimension = host.dimension();
        let (mega_roll, normal_roll) = match host.level_random(dimension) {
            Some(random) => (random.next_float(), random.next_float()),
            None => (0.0, 0.0),
        };
        let same = |dx: i32, dz: i32| host.block([pos[0] + dx, pos[1], pos[2] + dz]) == name;
        let Some(plan) = rules::plan(&grower, mega_roll, normal_roll, same) else {
            tell(host, "This sapling only grows in a square of four");
            return Flow::Cancel;
        };
        // Creative keeps the item (`!player.isCreative()`).
        if player.game_mode != util::CREATIVE && host.take_items(ID, 1) != 1 {
            return Flow::Cancel;
        }
        let at = |[dx, dz]: [i32; 2]| [pos[0] + dx, pos[1], pos[2] + dz];
        if let Ok(mut queue) = QUEUE.lock() {
            queue.push(Growth {
                dimension: dimension.to_owned(),
                saplings: plan.saplings.iter().map(|offset| (at(*offset), name.clone())).collect(),
                origin: at(plan.origin),
                feature: plan.feature,
            });
        }
        // `BoneMealItem.addGrowthParticles(level, pos, 100)`.
        host.send_particles(
            &ParticleOptions::simple(ParticleType::HappyVillager),
            None,
            util::centre(pos),
            [0.5, 0.5, 0.5],
            0.0,
            30,
        );
        Flow::Cancel
    }

    /// Places the trees clicked this tick.
    fn level_tick_post(&self, host: &mut dyn Host, _: &mut LevelTickPostEvent<'_>) {
        let dimension = host.dimension();
        let waiting: Vec<Growth> = match QUEUE.lock() {
            Ok(mut queue) => {
                let (mine, rest): (Vec<Growth>, Vec<Growth>) =
                    std::mem::take(&mut *queue).into_iter().partition(|g| g.dimension == dimension);
                *queue = rest;
                mine
            }
            Err(_) => return,
        };
        for growth in waiting {
            // `TreeGrower.growTree`: the saplings make way, then the feature goes where the first stood.
            for (pos, _) in &growth.saplings {
                host.run_command(&format!(
                    "execute in {dimension} run setblock {} {} {} minecraft:air",
                    pos[0], pos[1], pos[2]
                ));
            }
            let grown = host.run_command(&format!(
                "execute in {dimension} run place feature {} {} {} {}",
                growth.feature, growth.origin[0], growth.origin[1], growth.origin[2]
            ));
            if grown {
                continue;
            }
            // It didn't fit: the saplings go back and the fertilizer is returned.
            for (pos, block) in &growth.saplings {
                host.run_command(&format!(
                    "execute in {dimension} run setblock {} {} {} {block}",
                    pos[0], pos[1], pos[2]
                ));
            }
            let stack = ItemStack { item: ID.into(), count: 1, damage: 0, extra: None };
            util::spawn_item(host, util::centre(growth.origin), &stack, [0.0; 3], 10);
        }
    }
}
