//! The Contraption Controls in the game (`ContraptionControlsBlock`, `ContraptionControlsBlockEntity`,
//! `ContraptionControlsRenderer`): a panel with a button and a dial. A click turns it off and on,
//! redstone power turns it off, a filter names the kind of actor it is for, and the button sinks
//! and the dial turns half way round when it is off.
//!
//! What it is *for*, switching the actors (drills, saws...) of a moving contraption by that filter, needs
//! actors on contraptions, which this port does not have yet: until then it holds the state and the filter
//! (in Create's keys `Disabled`, `Powered`, `Filter`, as an assembled contraption would carry them) and shows
//! them, and does nothing else. The rules are `create_kinetics::rules::contraption_controls`; its header lists
//! the known differences. The ones that live here:
//!
//! - **The filter is set by clicking with an item** in `create:contraption_controlled` in hand (Create's
//!   value box), and cleared by clicking with a sneaking empty hand (where Create toggles). A click with
//!   anything else, even a block, toggles it, as `useWithoutItem` does; sneak to place a block on it.
//! - The Wrench turns the panel a quarter turn, and picks it up sneaking.
//! - Redstone is polled each tick (`neighborChanged` in Create).
//! - The status message's words come from Create's language keys.

use create_kinetics::rules::contraption_controls::{
    self as rules, Controls, FILTER_TAG, button_axis, button_at_rest, button_value, model_turn,
};
use create_kinetics::{Axis, Dir, Pos, looking};
use crate::compat::TextExt;
use crate::compat::{
    BlockEntityEvent, BlockEntitySubmitEvent, BlockEntityTickEvent, BlockEntityTicksEvent,
    BlockEntityUpdateTagEvent, BlockStateId, Compound, Flow, Hooks, Host, ItemStack, LevelHost,
    RightClickBlockEvent, StandaloneModelsEvent, StateForPlacementEvent, Tag, Text, UseItemOnEvent,
};

use crate::util;

pub const ID: &str = "create:contraption_controls";
const BUTTON_MODEL: &str = "create:block/contraption_controls/button";
const DISABLED: &str = "Disabled";
const POWERED: &str = "Powered";
const FILTER: &str = "Filter";
/// The game time of the last click, for the button's spring (Create's `button` is client-only).
const PRESSED_AT: &str = "PressedAt";

/// `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;
const CENTRE: [f32; 3] = [0.5; 3];

struct ContraptionControls;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(ContraptionControls)
}

fn indicator_model(i: usize) -> String {
    format!("create:block/contraption_controls/indicator_{i}")
}

fn controls_of(nbt: &Compound) -> Controls {
    Controls {
        disabled: util::boolean(nbt, DISABLED),
        powered: util::boolean(nbt, POWERED),
    }
}

fn write_controls(nbt: &mut Compound, controls: Controls) {
    if util::boolean(nbt, DISABLED) != controls.disabled || !util::has(nbt, DISABLED) {
        nbt.insert(DISABLED.into(), Tag::Byte(i8::from(controls.disabled)));
    }
    if util::boolean(nbt, POWERED) != controls.powered || !util::has(nbt, POWERED) {
        nbt.insert(POWERED.into(), Tag::Byte(i8::from(controls.powered)));
    }
}

fn filter_of(nbt: &Compound) -> Option<ItemStack> {
    util::get(nbt, FILTER).and_then(util::stack_of)
}

/// The name Create shows for an item: a block's translated name.
fn item_name(stack: &ItemStack) -> Text {
    let key = rules::block_item_key(&stack.item);
    Text::translate_with_fallback(&key, &stack.item, &[])
}

/// `sendStatus`.
pub(super) fn status(filter: Option<&ItemStack>, enabled: bool) -> Text {
    let [(line_key, line), (state_key, state)] = rules::status_keys(filter.is_some(), enabled);
    let state = Text::translate_with_fallback(state_key, state, &[]).with_color(rules::status_colour(enabled));
    match filter {
        None => Text::translate_with_fallback(line_key, line, &[state]),
        Some(stack) => Text::translate_with_fallback(line_key, line, &[item_name(stack), state]),
    }
}

fn facing_at(host: &dyn Host, pos: Pos) -> Option<Dir> {
    host.block_property(pos, "facing").as_deref().and_then(Dir::from_name)
}

impl Hooks for ContraptionControls {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.push(BUTTON_MODEL.to_owned());
        for i in 0..8 {
            event.models.push(indicator_model(i));
        }
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == ID).then(|| (ID.to_owned(), Compound::new()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == ID {
            event.ticks = true;
        }
    }

    /// `updatePoweredState`: the block follows its redstone signal.
    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != ID {
            return;
        }
        // `ContraptionControlsBlock.neighborChanged`: the signal the neighbour event left (`signals`).
        let signal = crate::signals::read(host, &mut event.entity.nbt, event.pos) > 0;
        if let Some(next) = controls_of(&event.entity.nbt).power_update(signal) {
            write_controls(&mut event.entity.nbt, next);
            host.block_entity_sync(event.pos);
        }
    }

    fn block_entity_update_tag(
        &self,
        event: &mut BlockEntityUpdateTagEvent<'_>,
    ) -> Option<Compound> {
        if event.entity.id != ID {
            return None;
        }
        let nbt = &event.entity.nbt;
        let mut tag = Compound::new();
        tag.insert(DISABLED.into(), Tag::Byte(i8::from(util::boolean(nbt, DISABLED))));
        tag.insert(PRESSED_AT.into(), Tag::Long(util::long(nbt, PRESSED_AT)));
        Some(tag)
    }

    /// `ContraptionControlsRenderer`: the button, sunk while it eases back up, and the dial.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != ID {
            return;
        }
        let Some(facing) = host.block_state_property(event.state, "facing") else { return };
        let disabled = util::boolean(&event.entity.nbt, DISABLED);
        let pressed_at = util::long(&event.entity.nbt, PRESSED_AT);
        let draw = &mut *event.draw;
        let ticks = draw.game_time as f64 + f64::from(draw.partial) - pressed_at as f64;
        let sink = button_value(ticks) * rules::BUTTON_DEPTH;
        let axis = button_axis(facing);
        let turn = model_turn(facing);

        draw.push();
        draw.translate([-axis[0] * sink, -axis[1] * sink, -axis[2] * sink]);
        draw.rotate_about(Axis::Y.unit(), -turn, CENTRE);
        host.draw_model(draw, BUTTON_MODEL, 0xFF_FFFF);
        draw.pop();

        let controls = Controls { disabled, powered: false };
        draw.push();
        draw.rotate_about(Axis::Y.unit(), -turn, CENTRE);
        host.draw_model(draw, &indicator_model(controls.indicator()), 0xFF_FFFF);
        draw.pop();
        if button_at_rest(ticks) {
            draw.keep();
        }
    }

    /// `ControlsBlock.getStateForPlacement`: facing the player, away from them when sneaking.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        let item: &str = &context.item;
        if item != ID {
            return None;
        }
        let horizontal = looking(context.yaw, 0.0);
        let facing = if context.sneaking { horizontal } else { horizontal.opposite() };
        level.state_with_property(event.state, "facing", facing.name()).map(Some)
    }

    /// `useWithoutItem`: the click toggles; an item of the filter's kind sets the filter; sneaking with an
    /// empty hand clears it.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else { return Flow::Continue };
        if player.game_mode == util::SPECTATOR || player.game_mode == 2 {
            return Flow::Continue;
        }
        let held = host.held_stack(player.uuid).filter(|stack| stack.count > 0);
        // The Wrench has its own job (`use_item_on`), and a sneaking player with a block places it.
        if held.as_ref().is_some_and(|stack| &*stack.item == util::WRENCH) {
            return Flow::Continue;
        }
        if player.sneaking && held.is_some() {
            return Flow::Continue;
        }
        if !util::first_click(host, player.uuid, pos) {
            return Flow::Cancel;
        }

        if let Some(stack) = held.as_ref()
            && host.server_item_in_tag(stack, FILTER_TAG).unwrap_or(false)
        {
            let mut one = stack.clone();
            one.count = 1;
            let tag = util::stack_tag(&one);
            host.change_mod_block_entity(pos, &mut |entity| {
                entity.nbt.insert(FILTER.into(), tag.clone());
            });
            let line = Text::translate_with_fallback("create.contraptions.contoller.target", "Targeted Component", &[])
                .append(Text::literal(": "))
                .append(item_name(&one));
            host.action_bar(player.uuid, line);
            return Flow::Cancel;
        }
        if player.sneaking {
            host.change_mod_block_entity(pos, &mut |entity| {
                util::remove(&mut entity.nbt, FILTER);
            });
            host.action_bar(
                player.uuid,
                Text::translate_with_fallback("create.contraptions.contoller.target", "Targeted Component", &[])
                    .append(Text::literal(": ")),
            );
            return Flow::Cancel;
        }

        let now = util::game_time(host);
        let mut shown = None;
        host.change_mod_block_entity(pos, &mut |entity| {
            let next = controls_of(&entity.nbt).clicked();
            write_controls(&mut entity.nbt, next);
            entity.nbt.insert(PRESSED_AT.into(), Tag::Long(now));
            shown = Some((next.disabled, filter_of(&entity.nbt)));
        });
        if let Some((disabled, filter)) = shown {
            host.block_entity_sync(pos);
            host.action_bar(player.uuid, status(filter.as_ref(), !disabled));
            // `CONTROLLER_CLICK.play(level, null, pos, 1, disabled ? 0.8f : 1.5f)`.
            util::create_sound_at(host, "controller_click", pos, 1.0, rules::click_pitch(disabled));
        }
        Flow::Cancel
    }

    /// The Wrench turns the panel a quarter turn, or picks it up sneaking.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        if &*event.stack.item != util::WRENCH || host.block(pos) != ID {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else { return Flow::Continue };
        if !util::first_click(host, player.uuid, pos) {
            return Flow::Cancel;
        }
        util::wrench(host, pos, |host| {
            if let Some(facing) = facing_at(host, pos) {
                host.set_block_property(pos, "facing", facing.clockwise_about(Axis::Y).name());
            }
        })
    }
}
