//! Step 2, items: the new `ItemPart` methods and the item-use globals, turned into the old hooks.
//!
//! Every Create item (and every block with an item) has one `CreateItem` Part (`parts.rs`); each method builds the
//! old event, runs it through the old dispatcher and writes the answer back.
use super::events::{stack_new, stack_old};
use super::*;

fn create() -> &'static dyn Hooks {
    crate::create()
}

/// Runs `f` with the old host for whichever side the hook fired on.
pub fn with_side<R>(host: HostSide<'_>, f: impl FnOnce(&mut Adapter<'_>) -> R) -> R {
    match host {
        HostSide::Server(host) => f(&mut Adapter::Server(host)),
        HostSide::Client(host) => f(&mut Adapter::ClientMut(host)),
    }
}

fn animation_old(animation: mod_api::UseAnimation) -> UseAnimation {
    match animation {
        mod_api::UseAnimation::None => UseAnimation::None,
        mod_api::UseAnimation::Eat => UseAnimation::Eat,
        mod_api::UseAnimation::Drink => UseAnimation::Drink,
        mod_api::UseAnimation::Block => UseAnimation::Block,
        mod_api::UseAnimation::Bow => UseAnimation::Bow,
        mod_api::UseAnimation::Trident => UseAnimation::Trident,
        mod_api::UseAnimation::Crossbow => UseAnimation::Crossbow,
        mod_api::UseAnimation::Spyglass => UseAnimation::Spyglass,
        mod_api::UseAnimation::TootHorn => UseAnimation::TootHorn,
        mod_api::UseAnimation::Brush => UseAnimation::Brush,
        mod_api::UseAnimation::Bundle => UseAnimation::Bundle,
        mod_api::UseAnimation::Spear => UseAnimation::Spear,
        #[allow(unreachable_patterns)]
        _ => UseAnimation::None,
    }
}

fn animation_new(animation: UseAnimation) -> mod_api::UseAnimation {
    match animation {
        UseAnimation::None => mod_api::UseAnimation::None,
        UseAnimation::Eat => mod_api::UseAnimation::Eat,
        UseAnimation::Drink => mod_api::UseAnimation::Drink,
        UseAnimation::Block => mod_api::UseAnimation::Block,
        UseAnimation::Bow => mod_api::UseAnimation::Bow,
        UseAnimation::Trident => mod_api::UseAnimation::Trident,
        UseAnimation::Crossbow => mod_api::UseAnimation::Crossbow,
        UseAnimation::Spyglass => mod_api::UseAnimation::Spyglass,
        UseAnimation::TootHorn => mod_api::UseAnimation::TootHorn,
        UseAnimation::Brush => mod_api::UseAnimation::Brush,
        UseAnimation::Bundle => mod_api::UseAnimation::Bundle,
        UseAnimation::Spear => mod_api::UseAnimation::Spear,
    }
}

/// `ItemPart::use_item`: the old hook answered `Flow::Cancel` to claim the click.
pub fn use_item(host: HostSide<'_>, event: &mut mod_api::UseItemEvent<'_>) -> Option<mod_api::InteractionResult> {
    let stack = match &host {
        HostSide::Server(host) => stack_old(&**host, event.stack),
        HostSide::Client(host) => stack_old(&**host, event.stack),
    }?;
    let mut old = UseItemEvent { stack: &stack };
    let flow = with_side(host, |adapter| create().use_item(adapter, &mut old));
    match flow {
        Flow::Continue => None,
        _ => Some(mod_api::InteractionResult::Success),
    }
}

/// `ItemPart::use_on`, the held item's own `useOn`: the game reaches it when the block did not take the click, and a
/// sneaking player's held item skips the block's `useItemOn` altogether, so this is where a sneak-click with the Wrench
/// (`WrenchItem.useOn` -> `onSneakWrenched`, pick the block up) arrives. The old `use_item_on` hook ran for every
/// click; the block part's `use_item_on` still covers the clicks of a player who is not sneaking.
pub fn use_on(host: HostSide<'_>, event: &mut mod_api::UseOnEvent<'_>) -> Option<mod_api::InteractionResult> {
    let HostSide::Server(host) = host else { return None };
    let sneaking = host.player(event.player).and_then(|player| player.is_secondary_use_active(&*host)).unwrap_or(false);
    if !sneaking {
        return None;
    }
    let stack = stack_old(&*host, event.stack)?;
    let mut old = UseItemOnEvent { stack: &stack, pos: event.pos, face: event.face };
    match create().use_item_on(&mut Adapter::Server(&mut *host), &mut old) {
        Flow::Continue => None,
        _ => Some(mod_api::InteractionResult::Success),
    }
}

pub fn inventory_tick(host: &mut dyn ServerHost, event: &mut mod_api::InventoryTickEvent<'_>) {
    let mut old = InventoryTickEvent { slot: event.slot, selected: event.selected };
    create().inventory_tick(&mut Adapter::Server(host), &mut old);
}

pub fn tooltip(host: &dyn ClientHost, event: &mut mod_api::TooltipEvent<'_>) {
    let Some(stack) = stack_old(host, event.stack) else { return };
    let mut old = TooltipEvent { stack: &stack, lines: &mut *event.lines };
    create().tooltip(&Adapter::Client(host), &mut old);
}

pub fn item_bar(host: &dyn ClientHost, event: &mut mod_api::ItemBarEvent<'_>) {
    let Some(stack) = stack_old(host, event.stack) else { return };
    let mut old = ItemBarEvent { stack: &stack, bar: &mut *event.bar };
    create().item_bar(&Adapter::Client(host), &mut old);
}

pub fn use_duration(host: &dyn mod_api::Host, event: &mut mod_api::UseDurationEvent<'_>) {
    let Some(stack) = stack_old(host, event.stack) else { return };
    let mut old = UseDurationEvent { stack: &stack, duration: event.duration };
    create().use_duration(&Adapter::Base(host), &mut old);
    event.duration = old.duration;
}

pub fn use_animation(host: &dyn ClientHost, event: &mut mod_api::UseAnimationEvent<'_>) {
    let Some(stack) = stack_old(host, event.stack) else { return };
    let mut animation = animation_old(*event.animation);
    let mut old = UseAnimationEvent { stack: &stack, animation: &mut animation };
    create().use_animation(&Adapter::Client(host), &mut old);
    *event.animation = animation_new(animation);
}

pub fn release_using(host: HostSide<'_>, event: &mut mod_api::ReleaseUsingEvent<'_>) {
    let stack = match &host {
        HostSide::Server(host) => stack_old(&**host, event.stack),
        HostSide::Client(host) => stack_old(&**host, event.stack),
    };
    let Some(stack) = stack else { return };
    let mut old =
        ReleaseUsingEvent { stack: &stack, entity: event.entity, time_left: event.time_left, used: event.used };
    with_side(host, |adapter| create().release_using(adapter, &mut old));
    event.used = old.used;
}

/// `Hooks::living_entity_use_item_tick`: the old event had no way to cancel.
pub fn living_entity_use_item_tick(
    host: HostSide<'_>,
    event: &mut mod_api::LivingEntityUseItemTickEvent<'_>,
) -> mod_api::Flow {
    let stack = match &host {
        HostSide::Server(host) => stack_old(&**host, event.stack),
        HostSide::Client(host) => stack_old(&**host, event.stack),
    };
    let Some(stack) = stack else { return mod_api::Flow::Continue };
    let mut old =
        LivingEntityUseItemTickEvent { stack: &stack, entity: event.entity, hand: event.hand, duration: event.duration };
    with_side(host, |adapter| create().living_entity_use_item_tick(adapter, &mut old))
}

/// `Hooks::living_entity_use_item_finish`: a module may replace what the hand holds afterwards.
pub fn living_entity_use_item_finish(
    host: HostSide<'_>,
    event: &mut mod_api::LivingEntityUseItemFinishEvent<'_>,
) {
    let stack = match &host {
        HostSide::Server(host) => stack_old(&**host, event.stack),
        HostSide::Client(host) => stack_old(&**host, event.stack),
    };
    let Some(stack) = stack else { return };
    let mut result = event.result.as_ref().and_then(|stack| match &host {
        HostSide::Server(host) => stack_old(&**host, stack),
        HostSide::Client(host) => stack_old(&**host, stack),
    });
    let before = result.clone();
    let mut old = LivingEntityUseItemFinishEvent {
        stack: &stack,
        entity: event.entity,
        hand: event.hand,
        duration: event.duration,
        result: &mut result,
    };
    // Convert the answer back with the same host the hook ran on.
    let mut host = host;
    match &mut host {
        HostSide::Server(h) => {
            create().living_entity_use_item_finish(&mut Adapter::Server(&mut **h), &mut old);
        }
        HostSide::Client(h) => {
            create().living_entity_use_item_finish(&mut Adapter::ClientMut(&mut **h), &mut old);
        }
    }
    if result != before {
        let base: &dyn mod_api::Host = match &host {
            HostSide::Server(h) => &**h,
            HostSide::Client(h) => &**h,
        };
        *event.result = result.as_ref().and_then(|stack| stack_new(base, stack));
    }
}
