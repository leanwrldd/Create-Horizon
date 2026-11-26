//! The Sticker (`StickerBlock`, `StickerBlockEntity`, `StickerRenderer`): each rise of the redstone
//! signal toggles it, and an extended one glues the block in front of it to a structure that moves
//! (the engine's side is `create_kinetics::rules::contraption`, `Contraption.moveBlock` and
//! `BlockMovementChecksImpl`; the rules here are `create_kinetics::rules::sticker`).
//!
//! Known differences from Create:
//! - The signal is read by the block entity's tick (a poll, as the Controls do), not by
//!   `neighborChanged`; a Sticker placed beside a signal takes its first reading as its powered
//!   state without extending, as `getStateForPlacement` does.
//! - The hit sound (`playSound`) plays as the head starts to move, not when it arrives; no glue particles (`SuperGlueItem.spawnParticles`) when the head
//!   meets a block, and no slime-block bounce on an upright Sticker (`fallOn`, `stepOn`).
//! - The head's animation is worked out from the game time of the change (`Since`), the same for
//!   every client, not a per-client `LerpedFloat`.

use create_kinetics::rules::bearing as bearing_rules;
use create_kinetics::rules::sticker::{self as rules, Sticker};
use create_kinetics::{Dir, Pos};
use crate::compat::{
    BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent,
    BlockEntityUpdateTagEvent, BlockStateId, Compound, Flow, Hooks, Host, LevelHost, StandaloneModelsEvent,
    StateForPlacementEvent, Tag, UseItemOnEvent,
};

use crate::util;

const ID: &str = rules::ID;
const HEAD: &str = "create:block/sticker/head";
const SINCE: &str = "Since";
const SEEN: &str = "Seen";
const CENTRE: [f32; 3] = [0.5; 3];

struct Stickers;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Stickers)
}

fn state_of(host: &dyn Host, pos: Pos) -> Option<(Dir, Sticker)> {
    let facing = util::facing(host, pos, "facing")?;
    let flag = |key: &str| host.block_property(pos, key).as_deref() == Some("true");
    Some((facing, Sticker { powered: flag("powered"), extended: flag("extended") }))
}

impl Hooks for Stickers {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.push(HEAD.to_owned());
    }

    fn block_entity(&self, _: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        (event.block == ID).then(|| {
            let mut nbt = Compound::new();
            nbt.insert(SINCE.into(), Tag::Long(i64::MIN / 2));
            (ID.to_owned(), nbt)
        })
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    /// `StickerBlock.neighborChanged`: the signal comes from the neighbour event (`signals`).
    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ID {
            return;
        }
        let pos = event.pos;
        let Some((facing, state)) = state_of(&*host, pos) else { return };
        let signal = crate::signals::read(host, &mut event.entity.nbt, pos) > 0;
        let first = !util::boolean(&event.entity.nbt, SEEN);
        if first {
            event.entity.nbt.insert(SEEN.into(), Tag::Byte(1));
        }
        // `getStateForPlacement`: the signal it stands in is its powered state, with no toggle.
        let next = if first {
            (state.powered != signal).then(|| Sticker { powered: signal, extended: state.extended })
        } else {
            state.power_update(signal)
        };
        let Some(next) = next else { return };
        if next.powered != state.powered {
            host.set_block_property(pos, "powered", if next.powered { "true" } else { "false" });
        }
        if next.extended != state.extended {
            host.set_block_property(pos, "extended", if next.extended { "true" } else { "false" });
            event.entity.nbt.insert(SINCE.into(), Tag::Long(util::game_time(&*host)));
            host.block_entity_sync(pos);
            // `StickerBlockEntity.playSound`: `SLIME_ADDED.play(.., 0.35, attach ? 0.75 : 0.2)` when the head meets a
            // block it can glue (`isAttachedToBlock`).
            let attached = super::contraption::HostWorld::new(&*host).is_some_and(|world| {
                create_kinetics::rules::super_glue::is_valid_face(&world, util::add(pos, facing.normal()), facing.opposite())
            });
            if attached {
                util::create_sound_at(host, "slime_added", pos, 0.35, if next.extended { 0.75 } else { 0.2 });
            }
        }
    }

    fn block_entity_update_tag(&self, event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
        (event.entity.id == ID).then(|| {
            let mut tag = Compound::new();
            tag.insert(SINCE.into(), Tag::Long(util::long(&event.entity.nbt, SINCE)));
            tag
        })
    }

    /// `StickerRenderer`: the head, out of the block along the facing.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ID {
            return;
        }
        let Some(facing) = host.block_state_property(event.state, "facing").and_then(Dir::from_name) else {
            return;
        };
        let extended = host.block_state_property(event.state, "extended") == Some("true");
        let since = util::long(&event.entity.nbt, SINCE) as f64;
        let now = event.draw.game_time as f64 + f64::from(event.draw.partial);
        let ticks = (now - since) as f32;
        let (from, target) = if extended { (0.0, 1.0) } else { (1.0, 0.0) };
        let value = rules::head_value(from, target, ticks);
        let draw = &mut *event.draw;
        draw.push();
        let (axis, degrees) = bearing_rules::up_to(facing);
        if degrees != 0.0 {
            draw.rotate_about(axis.unit(), degrees, CENTRE);
        }
        draw.translate([0.0, rules::head_offset(value), 0.0]);
        host.draw_model(draw, HEAD, 0xFF_FFFF);
        draw.pop();
        // Only a settled head is kept: it reads no game time.
        if ticks > 4.0 {
            draw.keep();
        }
    }

    /// `StickerBlock.getStateForPlacement`: facing the way the player looks, away from them when
    /// sneaking.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if &*context.item != ID {
            return None;
        }
        let look = create_kinetics::looking(context.yaw, context.pitch);
        let facing = if context.sneaking { look } else { look.opposite() };
        level.state_with_property(event.state, "facing", facing.name()).map(Some)
    }

    /// The Wrench turns it about the face clicked, or picks it up sneaking.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || host.block(event.pos) != ID {
            return Flow::Continue;
        }
        let (pos, clicked) = (event.pos, Dir::from_normal(event.face));
        util::wrench(host, pos, |host| {
            let (Some(facing), Some(clicked)) = (util::facing(host, pos, "facing"), clicked) else {
                return;
            };
            let next = rules::wrenched(facing, clicked);
            if next != facing {
                host.set_block_property(pos, "facing", next.name());
            }
        })
    }
}
