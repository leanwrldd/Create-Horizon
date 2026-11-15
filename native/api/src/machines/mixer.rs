//! The Mechanical Mixer (`MechanicalMixerBlock`, `MechanicalMixerBlockEntity`,
//! `BasinOperatingBlockEntity`, `MechanicalMixerRenderer`): two blocks above a Basin, it takes
//! rotation through its cogwheel, finds a `create:mixing` recipe the basin's contents and heat
//! satisfy, lowers its head, turns it while the recipe runs and raises it. The cycle is
//! `create_kinetics::rules::mixer`; the basin side is `machines/basin.rs`.
//!
//! Known differences from Create (for the README):
//! - The mixer meshes as a small cogwheel with no shaft ends (`Kind::encased_cog`, as
//!   `ICogWheel` with `hasShaftTowards` false), so it turns beside a small cogwheel or at the
//!   corner of a large one.
//! - Create is told at once when the basin's contents change (`contentsChanged`); the mixer looks
//!   at the basin every five ticks, and again every 200 ticks as Create's backup check does.
//! - No shapeless crafting recipes (Horizon's `recipes_of_type` answers only types outside
//!   `minecraft`) and no brewing (`PotionMixingRecipes`).
//! - `currentRecipe` is kept as `RecipeId`, so a cycle survives a reload; Create forgets it.
//! - Only the whirlpool sound plays; no ingredient or fluid particles, no mixing sound, no
//!   advancement. The block's shape is the player's (14 pixel casing) for everything.
//! - The block entity keeps its `Running` and `Ticks` keys; clients also get `Since`, the game time
//!   `Ticks` was true at, and move the head on from there.

use create_kinetics::rules::mixer::{
    self as rules, BOTTOM, IMPACT, POLL, Mixer,
};
use create_kinetics::{Axis, Kind, Node};
use horizon_client::mod_api::{
    BlockEntityEvent, BlockEntityRenderBoundingBoxEvent, BlockEntitySubmitEvent,
    BlockEntityTickEvent, BlockEntityTicksEvent, BlockEntityUpdateTagEvent, Compound, Flow, Hooks,
    Host, StandaloneModelsEvent, Tag, UseItemOnEvent,
};

use super::basin::{self, Kind as RecipeKind};
use crate::{kinetic, util};

const ID: &str = "create:mechanical_mixer";
/// `AllPartialModels.MECHANICAL_MIXER_POLE` / `MECHANICAL_MIXER_HEAD` / `SHAFTLESS_COGWHEEL`.
const POLE: &str = "create:block/mechanical_mixer/pole";
const HEAD: &str = "create:block/mechanical_mixer/head";
const COG: &str = "create:block/cogwheel_shaftless";
const CENTRE: [f32; 3] = [0.5; 3];

// Create's keys (`MechanicalMixerBlockEntity.write`).
const RUNNING: &str = "Running";
const TICKS: &str = "Ticks";
// Kept here: `processingTicks`, `currentRecipe`, the sync time and the basin check state.
const PROCESSING: &str = "ProcessingTicks";
const RECIPE: &str = "RecipeId";
const SINCE: &str = "Since";
const RECHECK: &str = "Recheck";
const SIGNATURE: &str = "BasinSignature";

struct MechanicalMixer;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(MechanicalMixer)
}

/// The mixer on its network: a small cogwheel about Y with no shaft ends, costing 4 su per RPM.
fn node() -> Node {
    Node {
        impact: IMPACT,
        ..Node::passive(Kind::encased_cog(Axis::Y, false, false, false))
    }
}

fn read(nbt: &Compound) -> Mixer {
    Mixer {
        running: util::boolean(nbt, RUNNING),
        running_ticks: util::int(nbt, TICKS),
        processing_ticks: if util::has(nbt, PROCESSING) {
            util::int(nbt, PROCESSING)
        } else {
            0
        },
    }
}

fn recipe_id(nbt: &Compound) -> Option<String> {
    match util::get(nbt, RECIPE) {
        Some(Tag::String(id)) => Some(id.to_string_lossy()).filter(|id| !id.is_empty()),
        _ => None,
    }
}

impl Hooks for MechanicalMixer {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        for model in [POLE, HEAD, COG] {
            if !event.models.iter().any(|m| m == model) {
                event.models.push(model.to_owned());
            }
        }
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == ID).then(|| {
            let mut nbt = kinetic::new_entity();
            nbt.insert(RUNNING.into(), Tag::Byte(0));
            nbt.insert(TICKS.into(), Tag::Int(0));
            nbt.insert(SINCE.into(), Tag::Long(0));
            (ID.to_owned(), nbt)
        })
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    /// `createRenderBoundingBox`: the block and 1.5 below it, where the head goes.
    fn block_entity_render_bounding_box(
        &self,
        _: &dyn Host,
        event: &mut BlockEntityRenderBoundingBoxEvent<'_>,
    ) {
        if event.block == ID {
            *event.bounds = [0.0, -1.5, 0.0, 1.0, 1.0, 1.0];
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ID {
            return;
        }
        let pos = event.pos;
        let Some((rpm, speed_changed)) = kinetic::tick(host, pos, event.entity, node()) else {
            return; // The network jammed and the block broke.
        };
        let basin = [pos[0], pos[1] - 2, pos[2]];
        let now = util::game_time(host);
        let old = read(&event.entity.nbt);
        let mut mixer = old;
        let mut sync = speed_changed;
        let mut recheck = util::boolean(&event.entity.nbt, RECHECK);

        // `BasinOperatingBlockEntity.tick`: the basin was emptied or removed, the head goes up.
        let flagged = util::boolean(&event.entity.nbt, basin::OPERATOR_FLAG);
        if flagged || (mixer.running && !basin::is_basin(host, basin)) {
            mixer.basin_removed();
            if flagged {
                event.entity.nbt.insert(basin::OPERATOR_FLAG.into(), Tag::Byte(0));
            }
            write(event.entity, old, mixer, now);
            host.block_entity_sync(pos);
            return;
        }
        // `onSpeedChanged`: look at the basin again.
        if speed_changed {
            recheck = true;
        }
        // Create hears of a change from the basin; here the basin is asked now and then.
        if !mixer.running && rules::speed_fulfilled(rpm) {
            let tick = now.rem_euclid(basin_backup()) == 0;
            if now.rem_euclid(POLL) == 0 || tick {
                let seen = basin::signature(host, basin);
                let kept = match util::get(&event.entity.nbt, SIGNATURE) {
                    Some(Tag::Long(v)) => Some(*v as u64),
                    _ => None,
                };
                if let Some(signature) = seen
                    && (kept != Some(signature) || tick)
                {
                    event.entity.nbt.insert(SIGNATURE.into(), Tag::Long(signature as i64));
                    recheck = true;
                }
            }
        }
        // `updateBasin`: a basin whose contents a recipe takes starts the head.
        if recheck {
            recheck = false;
            if rules::speed_fulfilled(rpm) && rpm != 0.0 && !mixer.running && basin::can_continue(host, basin) {
                if let Some(found) = basin::matching(host, basin, RecipeKind::Mixing).into_iter().next() {
                    event.entity.nbt.insert(RECIPE.into(), Tag::String(found.id.as_str().into()));
                    mixer.start();
                    sync = true;
                }
            }
        }

        // The cycle.
        let id = recipe_id(&event.entity.nbt);
        let duration = if mixer.running && mixer.running_ticks == BOTTOM && mixer.processing_ticks < 0 {
            id.as_deref()
                .and_then(|id| basin::recipe_by_id(host, id))
                .map_or(0, |recipe| recipe.processing_time)
        } else {
            0
        };
        let out = mixer.tick(rpm, duration);
        if out.started
            && basin::contents(host, basin).is_some_and(|c| c.tanks.iter().any(|t| t.amount > 0))
        {
            // `BUBBLE_COLUMN_WHIRLPOOL_AMBIENT`.
            host.play_sound(
                "minecraft:block.bubble_column.whirlpool_ambient",
                util::centre(pos),
                4,
                0.75,
                if rpm.abs() < 65.0 { 0.75 } else { 1.5 },
            );
        }
        if out.apply {
            sync = true;
            if let Some(recipe) = id.as_deref().and_then(|id| basin::recipe_by_id(host, id)) {
                let was_empty = basin::can_continue(host, basin);
                if basin::apply(host, basin, &recipe)
                    && was_empty
                    && basin::matches(host, basin, &recipe)
                {
                    // `continueWithPreviousRecipe`.
                    mixer.continue_recipe();
                }
            }
        }
        if out.recheck {
            recheck = true;
            sync = true;
        }
        if mixer.running != old.running
            || (mixer.running && (mixer.running_ticks - old.running_ticks).abs() != 1)
        {
            sync = true;
        }

        if recheck != util::boolean(&event.entity.nbt, RECHECK) {
            event.entity.nbt.insert(RECHECK.into(), Tag::Byte(i8::from(recheck)));
        }
        write(event.entity, old, mixer, now);
        if sync {
            host.block_entity_sync(pos);
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        if event.entity.id != ID {
            return None;
        }
        let nbt = &event.entity.nbt;
        let mut tag = kinetic::update_tag(event.entity);
        tag.insert(RUNNING.into(), Tag::Byte(i8::from(util::boolean(nbt, RUNNING))));
        tag.insert(TICKS.into(), Tag::Int(util::int(nbt, TICKS)));
        tag.insert(SINCE.into(), Tag::Long(util::long(nbt, SINCE)));
        Some(tag)
    }

    /// `MechanicalMixerRenderer`: the shaftless cog, and the pole and head, lowered by the cycle.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ID {
            return;
        }
        let nbt = &event.entity.nbt;
        let rpm = kinetic::speed(event.entity);
        let running = util::boolean(nbt, RUNNING);
        let [x, _, z] = event.pos;
        let draw = &mut *event.draw;

        // `KineticBlockEntityRenderer.getRotationOffsetForPosition`: a cog is turned 22.5 degrees
        // where the other two coordinates add up even, so neighbours mesh.
        let phase = if (x + z).rem_euclid(2) == 0 { 22.5 } else { 0.0 };
        draw.spin([0.0, 1.0, 0.0], CENTRE, rpm * 0.3, phase);
        host.draw_model(draw, COG, 0xFF_FFFF);
        draw.still();

        let now = draw.game_time as f64 + f64::from(draw.partial);
        let (ticks, partial) = if running {
            let t = rules::client_ticks(util::int(nbt, TICKS), util::long(nbt, SINCE), now);
            (t.floor() as i32, t - t.floor())
        } else {
            (0, 0.0)
        };
        let offset = rules::head_offset(running, ticks, partial);
        draw.push();
        draw.translate([0.0, -offset, 0.0]);
        host.draw_model(draw, POLE, 0xFF_FFFF);
        if running {
            // `angle = time * speed * 6 / 10` degrees, the speed from the head's state.
            let speed = rules::head_rotation_speed(true, ticks, rpm);
            let angle = ((now * f64::from(speed) * 0.6) % 360.0) as f32;
            draw.rotate_about([0.0, 1.0, 0.0], angle, CENTRE);
        } else {
            // At rest it turns at half the shaft's speed: 0.6 * rpm / 2 degrees a tick.
            draw.spin([0.0, 1.0, 0.0], CENTRE, rpm * 0.3, 0.0);
        }
        host.draw_model(draw, HEAD, 0xFF_FFFF);
        draw.pop();
        draw.still();
        // A head in motion reads the partial tick; one at rest turns through `spin` and is kept.
        if !running {
            draw.keep();
        }
    }

    /// The Wrench: sneak-click picks the mixer up (`IWrenchable`; nothing to turn).
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || host.block(event.pos) != ID {
            return Flow::Continue;
        }
        util::wrench(host, event.pos, |_| {})
    }
}

/// `BasinBlockEntity.recipeBackupCheck`: about every 200 ticks.
fn basin_backup() -> i64 {
    create_kinetics::rules::basin::BACKUP_CHECK_TICKS
}

/// Writes what changed. Reaching the entity's fields mutably counts as a change, so nothing is
/// touched when nothing differs.
fn write(entity: &mut horizon_client::mod_api::ModBlockEntity, old: Mixer, new: Mixer, now: i64) {
    if old.running != new.running {
        entity.nbt.insert(RUNNING.into(), Tag::Byte(i8::from(new.running)));
    }
    if old.running_ticks != new.running_ticks {
        entity.nbt.insert(TICKS.into(), Tag::Int(new.running_ticks));
    }
    if old.processing_ticks != new.processing_ticks {
        entity.nbt.insert(PROCESSING.into(), Tag::Int(new.processing_ticks));
    }
    // Clients run the head on from `Ticks` at `Since`: renew the pair when the count moved.
    if new.running && (old.running_ticks != new.running_ticks || !old.running)
        && util::long(&entity.nbt, SINCE) != now
    {
        entity.nbt.insert(SINCE.into(), Tag::Long(now));
    }
}
