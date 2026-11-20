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

/// The slots the old `Host::carried_stack` handed out during one `inventory_tick`: the old stack as the module
/// edits it, and what it was when read. Written back after the hook if it changed.
struct Carried {
    slot: i32,
    stack: Option<ItemStack>,
    before: Option<ItemStack>,
}

/// The tick in progress: whose inventory, which slot is the hook's own and its (new) stack, valid for the hook only.
struct TickScope {
    player: PlayerRef,
    slot: i32,
    stack: *const mod_api::ItemStack,
    carried: Vec<Box<Carried>>,
}

thread_local! {
    static TICK: std::cell::RefCell<Option<TickScope>> = const { std::cell::RefCell::new(None) };
}

/// `Host::carried_stack`: the stack in `slot` of the player whose `inventory_tick` is running, to read and change in
/// place (the change is written to the slot when the hook returns). `None` outside the hook and for an empty slot.
/// The pointer lives until the hook returns.
pub fn carried_stack(host: &dyn mod_api::Host, slot: i32) -> Option<*mut ItemStack> {
    TICK.with(|cell| {
        let mut scope = cell.borrow_mut();
        let scope = scope.as_mut()?;
        if let Some(have) = scope.carried.iter_mut().find(|carried| carried.slot == slot) {
            return have.stack.as_mut().map(|stack| stack as *mut ItemStack);
        }
        let old = if slot == scope.slot {
            // SAFETY: `inventory_tick` below keeps the event's stack alive until it drops the scope.
            stack_old(host, unsafe { &*scope.stack })
        } else {
            let number = create_kinetics::rules::inventory_slots::inventory_number(slot)?;
            let (_, found) = scope.player.inventory_slots(host)?.into_iter().find(|(index, _)| *index == number)?;
            stack_old(host, &found)
        };
        scope.carried.push(Box::new(Carried { slot, before: old.clone(), stack: old }));
        scope.carried.last_mut()?.stack.as_mut().map(|stack| stack as *mut ItemStack)
    })
}

/// `Host::carried_items`: the plain stacks of the main inventory of the acting player, summed by item.
pub fn carried_items(host: &dyn mod_api::Host, player: PlayerRef) -> Vec<(String, u32)> {
    let stacks: Vec<ItemStack> = player
        .inventory_slots(host)
        .unwrap_or_default()
        .into_iter()
        .filter(|(slot, _)| *slot < 36)
        .filter_map(|(_, stack)| stack_old(host, &stack))
        .filter(|stack| stack.damage == 0 && stack.extra.is_none())
        .collect();
    create_kinetics::rules::inventory_slots::tally(stacks.iter().map(|stack| (&*stack.item, stack.count)))
}

pub fn inventory_tick(host: &mut dyn ServerHost, event: &mut mod_api::InventoryTickEvent<'_>) {
    let mut old = InventoryTickEvent { slot: event.slot, selected: event.selected };
    let player = host.players().acting_player();
    if let Some(player) = player {
        TICK.with(|cell| {
            *cell.borrow_mut() =
                Some(TickScope { player, slot: event.slot, stack: event.stack as *const _, carried: Vec::new() });
        });
    }
    create().inventory_tick(&mut Adapter::Server(&mut *host), &mut old);
    let Some(player) = player else { return };
    let Some(scope) = TICK.with(|cell| cell.borrow_mut().take()) else { return };
    for carried in scope.carried {
        let Carried { slot, stack, before } = *carried;
        if stack == before {
            continue;
        }
        let Some(number) = create_kinetics::rules::inventory_slots::inventory_number(slot) else { continue };
        let new = stack.as_ref().and_then(|stack| stack_new(&*host, stack));
        if let Err(refusal) = player.set_inventory_slot(&mut *host, number, new) {
            eprintln!("create: carried stack in slot {slot} was not written: {refusal:?}");
        }
    }
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
