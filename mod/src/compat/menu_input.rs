//! `Host::open_menu_with_input` (the Toolbox's view) on the new API.
//!
//! The old call opened a chest view whose input slots worked by vanilla's rules, and `menu_input_closed` handed the
//! slots back when it closed. The new API has the chest menu (`ServerHost::open_menu`, `generic_9x<rows>`), but it
//! moves nothing on a click: each click reaches `menu_clicked`, and the menu's own slots are gone when `menu_closed`
//! comes. So this layer
//!
//! * opens the chest menu and remembers the view it shows (`OPEN`, by window id);
//! * answers the clicks of such a window itself (`clicked`): the click rules are `rules::menu_input`; the player's slots
//!   and the cursor are written through `ItemPlace` in one transaction (a refused write leaves them as they were), and
//!   the view is stored and sent whole with `menu_set_items`, which also sends the inventory and the cursor: the engine
//!   answers every click of a native menu at once with the menu as it was, syncs a mod's slot writes only with the next
//!   tick, and never syncs the cursor, so a client that predicted the click is corrected only by that call;
//! * on `menu_closed`, gives the module its old `menu_input_closed` with the remembered view, and hands what the
//!   module leaves in the slots to the player (`PlayerRef::give_stack`: what doesn't fit is thrown, as `/give` does).
//!
//! No content menu type is declared: `open_menu` needs none (vanilla's own `generic_9x3`/`9x4` screen is drawn), and
//! `open_mod_menu` would need a screen of the mod's own on the client.
use std::collections::HashMap;
use std::ops::Range;
use std::sync::{LazyLock, Mutex, PoisonError};

use create_kinetics::rules::menu_input as rules;
use create_kinetics::rules::toolbox::{Item, Stack};

use super::events::{items_new, items_old};
use super::*;

/// A view this layer opened and has not seen closed.
struct Open {
    /// The slots of the view (`rows * 9`).
    size: usize,
    /// The slots that take items; the rest only give.
    input: Range<usize>,
    /// What the view shows now.
    view: Vec<Option<mod_api::ItemStack>>,
}

static OPEN: LazyLock<Mutex<HashMap<i32, Open>>> = LazyLock::new(Mutex::default);

fn open_views() -> std::sync::MutexGuard<'static, HashMap<i32, Open>> {
    OPEN.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Real stacks and the plain items the rules tell apart, for one click.
#[derive(Default)]
struct Palette(Vec<(ItemResource, Item)>);

impl Palette {
    fn to_rules(&mut self, host: &dyn mod_api::Host, stack: &mod_api::ItemStack) -> Stack {
        let known = self.0.iter().find(|(resource, _)| resource.matches(stack)).map(|(_, item)| item.clone());
        let item = known.unwrap_or_else(|| {
            let item = Item {
                id: format!("#{}", self.0.len()),
                components: String::new(),
                max_stack: host.max_stack_size(stack.item()).unwrap_or(64),
                damageable: false,
                fits: true,
            };
            self.0.push((ItemResource::of(stack), item.clone()));
            item
        });
        Stack::new(item, stack.count())
    }

    fn to_real(&self, stack: &Stack) -> Option<mod_api::ItemStack> {
        let (resource, _) = self.0.iter().find(|(_, item)| *item == stack.item)?;
        resource.to_stack(stack.count)
    }
}

/// `Host::open_menu_with_input`: the window id, None when the menu did not open.
pub fn open(
    host: &mut dyn ServerHost,
    uuid: u128,
    rows: u8,
    title: Text,
    items: Vec<Option<ItemStack>>,
    input: Range<usize>,
) -> Option<i32> {
    let size = usize::from(rows.clamp(1, 6)) * 9;
    let mut view = items_new(&*host, &items);
    view.resize(size, None);
    let window = host.open_menu(uuid, rows, title, view.clone(), MenuReplace::CloseFirst).ok().flatten()?;
    open_views().insert(window, Open { size, input, view });
    Some(window)
}

/// A click in a window this layer opened: true when it was one (and has been dealt with).
pub fn clicked(host: &mut dyn ServerHost, event: &mut mod_api::MenuClickedEvent) -> bool {
    let (size, input) = match open_views().get(&event.window) {
        Some(open) => (open.size, open.input.clone()),
        None => return false,
    };
    let Some(menu) = host.menu(event.uuid, event.window) else { return true };
    let total = size + rules::PLAYER_SLOTS;
    if menu.slot_count(&*host) != Some(total) {
        return true;
    }
    let mut real: Vec<Option<mod_api::ItemStack>> = Vec::with_capacity(total);
    for index in 0..total {
        let Some(stack) = menu.slot(&*host, index) else { return true };
        real.push(stack);
    }
    let Some(carried) = menu.carried(&*host) else { return true };

    let mut palette = Palette::default();
    let before: Vec<Option<Stack>> =
        real.iter().map(|slot| slot.as_ref().map(|stack| palette.to_rules(&*host, stack))).collect();
    let cursor_before = carried.as_ref().map(|stack| palette.to_rules(&*host, stack));
    let mut state = rules::Menu::new(before[..size].to_vec(), before[size..].to_vec(), input);
    state.cursor = cursor_before.clone();
    state.click(event.slot, event.button, event.mode);

    let changed: Vec<usize> = (0..total).filter(|&index| state.slots[index] != before[index]).collect();
    let cursor_changed = state.cursor != cursor_before;
    if changed.is_empty() && !cursor_changed {
        return true;
    }

    // The player's slots and the cursor change in one transaction (everything that changed comes out first, then goes
    // in as it now is: one that fails puts it all back). The view's own slots are not written through `ItemPlace`
    // (the engine syncs those with the next tick only, and never the cursor): the whole view goes back with
    // `menu_set_items`, which stores it, and sends it with the inventory and the cursor at once, as `sendAllDataToRemote`.
    // The engine has already answered the click with the menu as it was, so a client that predicted it needs this.
    let written = host.transaction(|host, tx| -> Result<(), String> {
        let mut places: Vec<(ItemPlace, Option<mod_api::ItemStack>, Option<Stack>)> = changed
            .iter()
            .filter(|&&index| index >= size)
            .map(|&index| (ItemPlace::MenuSlot { menu, slot: index }, real[index].clone(), state.slots[index].clone()))
            .collect();
        if cursor_changed {
            places.push((menu.cursor(), carried.clone(), state.cursor.clone()));
        }
        for (place, old, _) in &places {
            if let Some(old) = old {
                let taken = place.extract(host, tx, &ItemResource::of(old), old.count());
                if taken != old.count() {
                    return Err(format!("took {taken} of {} from {place:?}", old.count()));
                }
            }
        }
        for (place, _, new) in &places {
            if let Some(new) = new {
                let stack = palette.to_real(new).ok_or_else(|| "a stack with no real item".to_owned())?;
                let put = place.insert(host, tx, &ItemResource::of(&stack), stack.count());
                if put != stack.count() {
                    return Err(format!("put {put} of {} into {place:?}", stack.count()));
                }
            }
        }
        Ok(())
    });
    let view: Vec<Option<mod_api::ItemStack>> = match &written {
        Ok(()) => state.view().iter().map(|slot| slot.as_ref().and_then(|stack| palette.to_real(stack))).collect(),
        Err(refused) => {
            eprintln!("create: Toolbox view click refused: {refused:?}");
            real[..size].to_vec()
        }
    };
    if let Err(refusal) = host.menu_set_items(event.uuid, event.window, view.clone()) {
        eprintln!("create: Toolbox view not sent: {refusal:?}");
    }
    if written.is_ok() && let Some(open) = open_views().get_mut(&event.window) {
        open.view = view;
    }
    true
}

/// A window closed: the view this layer remembers for it, if it opened it.
pub fn closed(window: i32) -> Option<Vec<Option<mod_api::ItemStack>>> {
    open_views().remove(&window).map(|open| open.view)
}

/// `Hooks::menu_input_closed` for a closed window of this layer, then the rest of the slots to the player.
pub fn closed_to_modules(host: &mut dyn ServerHost, event: &mod_api::MenuClosedEvent, view: Vec<Option<mod_api::ItemStack>>) {
    let mut slots = items_old(&*host, &view);
    let mut old = MenuInputClosedEvent { uuid: event.uuid, window: event.window, slots: &mut slots };
    crate::create().menu_input_closed(&mut Adapter::Server(&mut *host), &mut old);
    let Some(player) = host.player(event.uuid) else { return };
    for stack in items_new(&*host, &slots).into_iter().flatten() {
        let _ = player.give_stack(host, stack);
    }
}
