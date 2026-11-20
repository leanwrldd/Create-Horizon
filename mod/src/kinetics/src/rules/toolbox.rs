//! The Toolbox: `ToolboxInventory` (8 compartments of 4 stacks, one item type each), the quick-equip
//! bookkeeping of `ToolboxBlockEntity.tickPlayers` and `ToolboxHandler`, and the radial menu's
//! stand-in layout. No game types: an item is an [`Item`] the glue builds from the stack.
//!
//! Create's `ToolboxInventory` is an `ItemStackHandler` of 32 slots whose `settle` keeps each
//! compartment packed (full stacks first, the remainder in the next, none after a gap), and whose
//! `filters` remember the item a compartment was first given.

pub const COMPARTMENTS: usize = 8;
/// `ToolboxInventory.STACKS_PER_COMPARTMENT`.
pub const STACKS_PER_COMPARTMENT: usize = 4;
pub const SLOTS: usize = COMPARTMENTS * STACKS_PER_COMPARTMENT;
/// `CEquipment.toolboxRange`: how far a toolbox reaches a player's hotbar, in blocks.
pub const RANGE: f64 = 10.0;
/// `ItemStackHandler.getSlotLimit`.
pub const SLOT_LIMIT: u32 = 64;
/// The hotbar.
pub const HOTBAR: usize = 9;

/// An item as the compartments tell items apart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    /// Everything but the id and the count (the components), as one comparable key.
    pub components: String,
    pub max_stack: u32,
    /// `ItemStack.isDamageableItem`.
    pub damageable: bool,
    /// `Item.canFitInsideContainerItems`: shulker boxes and toolboxes don't.
    pub fits: bool,
}

impl Item {
    pub fn stackable(&self) -> bool {
        self.max_stack > 1
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stack {
    pub item: Item,
    pub count: u32,
}

impl Stack {
    pub fn new(item: Item, count: u32) -> Self {
        Self { item, count }
    }
    fn with_count(&self, count: u32) -> Self {
        Self { item: self.item.clone(), count }
    }
}

/// `AllItems.BELT_CONNECTOR`.
pub const BELT_CONNECTOR: &str = "create:belt_connector";

/// `ToolboxInventory.canItemsShareCompartment`.
pub fn can_share(a: &Item, b: &Item) -> bool {
    if !a.stackable() && !b.stackable() && a.damageable && b.damageable {
        return a.id == b.id;
    }
    if a.id == BELT_CONNECTOR && b.id == BELT_CONNECTOR {
        return true;
    }
    a.id == b.id && a.components == b.components
}

/// Whether the id names a tool or piece of armour: what Horizon cannot ask an item
/// (`isDamageableItem`), by the names vanilla gives them.
pub fn damageable_id(id: &str) -> bool {
    const SUFFIXES: [&str; 13] = [
        "_pickaxe", "_axe", "_shovel", "_hoe", "_sword", "_helmet", "_chestplate", "_leggings",
        "_boots", "_spear", "_nautilus_armor", "_horse_armor", "_wolf_armor",
    ];
    const NAMES: [&str; 14] = [
        "minecraft:bow", "minecraft:crossbow", "minecraft:shears", "minecraft:fishing_rod",
        "minecraft:flint_and_steel", "minecraft:shield", "minecraft:trident", "minecraft:mace",
        "minecraft:elytra", "minecraft:brush", "minecraft:carrot_on_a_stick",
        "minecraft:warped_fungus_on_a_stick", "create:sand_paper", "create:red_sand_paper",
    ];
    SUFFIXES.iter().any(|s| id.ends_with(s)) || NAMES.contains(&id)
}

/// `ToolboxBlock`'s items: `UncontainableBlockItem`, and vanilla's shulker boxes.
pub fn fits_inside_containers(id: &str) -> bool {
    !(id.ends_with("_toolbox") || id.ends_with("shulker_box"))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toolbox {
    /// `ToolboxInventory.filters`: the item each compartment was first given.
    pub filters: [Option<Item>; COMPARTMENTS],
    pub slots: [Option<Stack>; SLOTS],
}

impl Default for Toolbox {
    fn default() -> Self {
        Self { filters: Default::default(), slots: std::array::from_fn(|_| None) }
    }
}

impl Toolbox {
    /// `ToolboxInventory.isItemValid`. `limited` is `inLimitedMode`: a compartment with no filter
    /// takes nothing.
    pub fn is_item_valid(&self, slot: usize, item: &Item, limited: bool) -> bool {
        if !item.fits || slot >= SLOTS {
            return false;
        }
        match &self.filters[slot / STACKS_PER_COMPARTMENT] {
            None => !limited,
            Some(filter) => can_share(filter, item),
        }
    }

    /// `ItemStackHandler.insertItem` with the toolbox's overrides (filters set on first use, the
    /// compartment settled afterwards): what could not be put in.
    pub fn insert_item(&mut self, slot: usize, stack: Stack, simulate: bool, limited: bool) -> Option<Stack> {
        if stack.count == 0 || !self.is_item_valid(slot, &stack.item, limited) {
            return Some(stack);
        }
        let mut limit = SLOT_LIMIT.min(stack.item.max_stack);
        if let Some(existing) = &self.slots[slot] {
            if !same_stack(&existing.item, &stack.item) {
                return Some(stack);
            }
            limit = limit.saturating_sub(existing.count);
        }
        if limit == 0 {
            return Some(stack);
        }
        let reached = stack.count > limit;
        let moved = stack.count.min(limit);
        if !simulate {
            match &mut self.slots[slot] {
                Some(existing) => existing.count += moved,
                empty => *empty = Some(stack.with_count(moved)),
            }
            let compartment = slot / STACKS_PER_COMPARTMENT;
            if self.filters[compartment].is_none() {
                self.filters[compartment] = Some(stack.item.clone());
            }
            self.settle(compartment);
        }
        reached.then(|| stack.with_count(stack.count - limit))
    }

    /// `ItemStackHandler.extractItem` (then `onContentsChanged`, which settles).
    pub fn extract_item(&mut self, slot: usize, amount: u32, simulate: bool) -> Option<Stack> {
        let existing = self.slots[slot].clone()?;
        if amount == 0 {
            return None;
        }
        let take = amount.min(existing.item.max_stack).min(existing.count);
        if !simulate {
            if existing.count <= take {
                self.slots[slot] = None;
            } else if let Some(left) = &mut self.slots[slot] {
                left.count -= take;
            }
            self.settle(slot / STACKS_PER_COMPARTMENT);
        }
        Some(existing.with_count(take))
    }

    /// `ToolboxInventory.settle`: stackable items are packed into full stacks from the first slot,
    /// the rest in the next; unstackable ones are moved up over gaps.
    pub fn settle(&mut self, compartment: usize) {
        let base = compartment * STACKS_PER_COMPARTMENT;
        let mut total = 0u32;
        let mut valid = true;
        let mut should_be_empty = false;
        let mut sample: Option<Stack> = None;
        for i in 0..STACKS_PER_COMPARTMENT {
            let stack = &self.slots[base + i];
            total += stack.as_ref().map_or(0, |s| s.count);
            if !should_be_empty {
                should_be_empty = stack.as_ref().is_none_or(|s| s.count != s.item.max_stack);
            } else if let Some(stack) = stack {
                valid = false;
                sample = Some(stack.clone());
            }
        }
        if valid {
            return;
        }
        let stackable = sample.as_ref().is_some_and(|s| s.item.stackable());
        if !stackable {
            for i in 0..STACKS_PER_COMPARTMENT {
                if self.slots[base + i].is_some() {
                    continue;
                }
                for j in i + 1..STACKS_PER_COMPARTMENT {
                    if self.slots[base + j].is_none() {
                        continue;
                    }
                    self.slots[base + i] = self.slots[base + j].take();
                    break;
                }
            }
        } else if let Some(sample) = sample {
            for i in 0..STACKS_PER_COMPARTMENT {
                let count = total.min(sample.item.max_stack);
                self.slots[base + i] = (count > 0).then(|| sample.with_count(count));
                total -= count;
            }
        }
    }

    /// `ToolboxInventory.distributeToCompartment`: puts `stack` into the compartment from its last
    /// slot back; what did not fit. A compartment with no filter takes nothing.
    pub fn distribute_to_compartment(&mut self, stack: Stack, compartment: usize, simulate: bool, limited: bool) -> Option<Stack> {
        if stack.count == 0 || self.filters[compartment].is_none() {
            return Some(stack);
        }
        let mut stack = stack;
        for i in (0..STACKS_PER_COMPARTMENT).rev() {
            match self.insert_item(compartment * STACKS_PER_COMPARTMENT + i, stack, simulate, limited) {
                None => return None,
                Some(rest) => stack = rest,
            }
        }
        Some(stack)
    }

    /// `ToolboxInventory.takeFromCompartment`: up to `amount` from the compartment's last slot back.
    pub fn take_from_compartment(&mut self, amount: u32, compartment: usize, simulate: bool) -> Option<Stack> {
        if amount == 0 {
            return None;
        }
        let mut remaining = amount;
        let mut last: Option<Stack> = None;
        for i in (0..STACKS_PER_COMPARTMENT).rev() {
            if let Some(got) = self.extract_item(compartment * STACKS_PER_COMPARTMENT + i, remaining, simulate) {
                remaining -= got.count;
                last = Some(got);
            }
            if remaining == 0 {
                return last.map(|l| l.with_count(amount));
            }
        }
        if remaining == amount {
            return None;
        }
        last.map(|l| l.with_count(amount - remaining))
    }

    /// `ItemHandlerHelper.insertItemStacked` over every slot: onto the slots that hold the same
    /// item first, then the empty ones.
    pub fn insert_stacked(&mut self, stack: Stack, limited: bool) -> Option<Stack> {
        let mut stack = stack;
        for slot in 0..SLOTS {
            if self.slots[slot].as_ref().is_some_and(|s| same_stack(&s.item, &stack.item)) {
                match self.insert_item(slot, stack, false, limited) {
                    None => return None,
                    Some(rest) => stack = rest,
                }
            }
        }
        for slot in 0..SLOTS {
            if self.slots[slot].is_none() {
                match self.insert_item(slot, stack, false, limited) {
                    None => return None,
                    Some(rest) => stack = rest,
                }
            }
        }
        Some(stack)
    }

    /// How many items a compartment holds (`ToolboxMenu.totalCountInCompartment`).
    pub fn total_in_compartment(&self, compartment: usize) -> u32 {
        let base = compartment * STACKS_PER_COMPARTMENT;
        self.slots[base..base + STACKS_PER_COMPARTMENT].iter().flatten().map(|s| s.count).sum()
    }

    /// The comparator's reading, `ItemHelper.calcRedstoneFromInventory`.
    pub fn redstone(&self) -> u8 {
        let mut fill = 0.0f32;
        let mut used = 0;
        for stack in self.slots.iter().flatten() {
            fill += stack.count as f32 / SLOT_LIMIT.min(stack.item.max_stack) as f32;
            used += 1;
        }
        fill /= SLOTS as f32;
        if used == 0 { 0 } else { ((fill * 14.0).floor() as u8 + 1).min(15) }
    }
}

/// `ItemStack.isSameItemSameComponents`.
pub fn same_stack(a: &Item, b: &Item) -> bool {
    a.id == b.id && a.components == b.components
}

// ---- quick equip (`ToolboxBlockEntity.tickPlayers`) -------------------------------------------

/// What one tick does for a hotbar slot linked to a compartment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Nothing,
    /// Take this many from the toolbox into the slot.
    Take(u32),
    /// Put this many from the slot into the toolbox.
    Deposit(u32),
    /// The link is over (the compartment's filter is gone, or the slot holds something else).
    Unlink,
}

/// The stack a linked slot is kept at: half of a full stack, rounded up.
pub fn target_amount(max_stack: u32) -> u32 {
    max_stack.div_ceil(2)
}

/// One tick of a link: `slot` is what the hotbar slot holds, `filter` the compartment's item.
pub fn tick_link(slot: Option<&Stack>, filter: Option<&Item>) -> Step {
    let Some(filter) = filter else {
        return Step::Unlink;
    };
    let target = target_amount(filter.max_stack);
    match slot {
        Some(stack) if !can_share(&stack.item, filter) => Step::Unlink,
        Some(stack) if stack.count > target => Step::Deposit(stack.count - target),
        Some(stack) if stack.count < target => Step::Take(target - stack.count),
        Some(_) => Step::Nothing,
        None => Step::Take(target),
    }
}

/// `ToolboxHandler.withinRange`: the squared distance from a player to the box (its centre in x
/// and z, its floor in y) against the range.
pub fn within_range(player: [f64; 3], box_pos: [i32; 3]) -> bool {
    let dx = player[0] - (f64::from(box_pos[0]) + 0.5);
    let dy = player[1] - f64::from(box_pos[1]);
    let dz = player[2] - (f64::from(box_pos[2]) + 0.5);
    dx * dx + dy * dy + dz * dz < RANGE * RANGE
}

// ---- the stand-in screen ----------------------------------------------------------------------

/// Columns of the container view: one per compartment, the ninth unused.
pub const MENU_COLUMNS: usize = 9;
pub const MENU_ROWS: usize = STACKS_PER_COMPARTMENT;
pub const MENU_SLOTS: usize = MENU_COLUMNS * MENU_ROWS;

/// Where a toolbox slot shows in the 9 x 4 view: a compartment is a column, its stacks run down.
pub fn menu_slot(slot: usize) -> usize {
    (slot % STACKS_PER_COMPARTMENT) * MENU_COLUMNS + slot / STACKS_PER_COMPARTMENT
}

/// The compartment of a view slot, none for the unused ninth column.
pub fn menu_compartment(view_slot: usize) -> Option<usize> {
    let column = view_slot % MENU_COLUMNS;
    (column < COMPARTMENTS).then_some(column)
}

impl Toolbox {
    /// The slots as the 9 x 4 view shows them.
    pub fn to_menu(&self) -> Vec<Option<Stack>> {
        let mut view = vec![None; MENU_SLOTS];
        for (slot, stack) in self.slots.iter().enumerate() {
            view[menu_slot(slot)] = stack.clone();
        }
        view
    }

    /// Takes the view back as the player left it. A compartment keeps its filter if one of the
    /// stacks now in it shares it, else the first stack becomes the filter; a compartment left
    /// empty keeps its filter only if `keep_empty` says so (a player is equipped on it). Stacks
    /// that don't fit (another item than the compartment's, the unused column, a shulker box) are
    /// returned.
    pub fn apply_menu(&mut self, view: Vec<Option<Stack>>, keep_empty: [bool; COMPARTMENTS]) -> Vec<Stack> {
        let mut rejected = Vec::new();
        let mut next = Toolbox::default();
        for compartment in 0..COMPARTMENTS {
            let column: Vec<Stack> = (0..STACKS_PER_COMPARTMENT)
                .filter_map(|i| view.get(i * MENU_COLUMNS + compartment).cloned().flatten())
                .filter(|s| s.count > 0)
                .collect();
            let (fit, unfit): (Vec<Stack>, Vec<Stack>) = column.into_iter().partition(|s| s.item.fits);
            rejected.extend(unfit);
            let old = &self.filters[compartment];
            let filter = match old {
                Some(old) if fit.iter().any(|s| can_share(old, &s.item)) => Some(old.clone()),
                _ => fit.first().map(|s| s.item.clone()),
            };
            let Some(filter) = filter else {
                next.filters[compartment] = if keep_empty[compartment] { old.clone() } else { None };
                continue;
            };
            next.filters[compartment] = Some(filter.clone());
            for stack in fit {
                if can_share(&filter, &stack.item) {
                    // Stacks over 64 can't be in a slot already; put it where it fits.
                    if let Some(rest) = next.distribute_to_compartment(stack, compartment, false, false) {
                        rejected.push(rest);
                    }
                } else {
                    rejected.push(stack);
                }
            }
        }
        for i in 0..STACKS_PER_COMPARTMENT {
            if let Some(stack) = view.get(i * MENU_COLUMNS + COMPARTMENTS).cloned().flatten() {
                rejected.push(stack);
            }
        }
        *self = next;
        rejected
    }
}

// ---- the quick-equip stand-in's choice --------------------------------------------------------

/// What a sneak-click on the toolbox does, from the hand and the hotbar slot's link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Click {
    /// Link the selected hotbar slot to this compartment.
    Equip(usize),
    /// End the slot's link, putting its stack back.
    Unequip,
    Nothing,
}

/// The stand-in for the radial menu (`RadialToolboxMenu`): with an item in hand, equips the
/// compartment it belongs to (or ends the link if that is the one already linked); with an empty
/// hand, steps to the next compartment that holds something, and past the last one ends the link.
pub fn sneak_click(filters: &[Option<Item>; COMPARTMENTS], counts: [u32; COMPARTMENTS], held: Option<&Item>, linked: Option<usize>) -> Click {
    match held {
        Some(item) => {
            let Some(compartment) = filters.iter().position(|f| f.as_ref().is_some_and(|f| can_share(f, item))) else {
                return Click::Nothing;
            };
            if linked == Some(compartment) { Click::Unequip } else { Click::Equip(compartment) }
        }
        None => {
            let start = linked.map_or(0, |l| l + 1);
            match (start..COMPARTMENTS).find(|&c| filters[c].is_some() && counts[c] > 0) {
                Some(next) => Click::Equip(next),
                None if linked.is_some() => Click::Unequip,
                None => Click::Nothing,
            }
        }
    }
}

// ---- dyeing -----------------------------------------------------------------------------------

/// `DyeColor`'s serialized names, in order: the toolbox of each (`AllBlocks.TOOLBOXES`).
pub const COLOURS: [&str; 16] = [
    "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray",
    "cyan", "purple", "blue", "brown", "green", "red", "black",
];

/// `create:<colour>_toolbox`.
pub fn colour_of(block: &str) -> Option<&'static str> {
    let name = block.strip_prefix("create:")?.strip_suffix("_toolbox")?;
    COLOURS.iter().copied().find(|c| *c == name)
}

pub fn is_toolbox(block: &str) -> bool {
    colour_of(block).is_some()
}

/// The colour a dye item colours a toolbox (`DyeColor.getColor(stack)`): `minecraft:<colour>_dye`.
pub fn dye_colour(item: &str) -> Option<&'static str> {
    let name = item.strip_prefix("minecraft:")?.strip_suffix("_dye")?;
    COLOURS.iter().copied().find(|c| *c == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, max: u32) -> Item {
        Item { id: id.into(), components: String::new(), max_stack: max, damageable: false, fits: true }
    }
    fn tool(id: &str) -> Item {
        Item { id: id.into(), components: String::new(), max_stack: 1, damageable: true, fits: true }
    }
    fn stack(id: &str, max: u32, count: u32) -> Stack {
        Stack::new(item(id, max), count)
    }

    #[test]
    fn the_first_item_in_sets_the_compartments_filter() {
        let mut t = Toolbox::default();
        assert!(t.insert_item(0, stack("minecraft:stone", 64, 10), false, false).is_none());
        assert_eq!(t.filters[0], Some(item("minecraft:stone", 64)));
        // Another item can't go into that compartment.
        let rest = t.insert_item(1, stack("minecraft:dirt", 64, 5), false, false);
        assert_eq!(rest, Some(stack("minecraft:dirt", 64, 5)));
        // But a free compartment takes it.
        assert!(t.insert_item(4, stack("minecraft:dirt", 64, 5), false, false).is_none());
        assert_eq!(t.filters[1], Some(item("minecraft:dirt", 64)));
    }

    #[test]
    fn limited_mode_refuses_a_compartment_with_no_filter() {
        let t = Toolbox::default();
        assert!(!t.is_item_valid(0, &item("minecraft:stone", 64), true));
        assert!(t.is_item_valid(0, &item("minecraft:stone", 64), false));
    }

    #[test]
    fn a_shulker_box_does_not_fit() {
        let mut shulker = item("minecraft:red_shulker_box", 1);
        shulker.fits = false;
        assert!(!Toolbox::default().is_item_valid(0, &shulker, false));
        assert!(!fits_inside_containers("create:red_toolbox"));
        assert!(fits_inside_containers("minecraft:stone"));
    }

    #[test]
    fn settle_packs_stackable_items_into_full_stacks_first() {
        let mut t = Toolbox::default();
        t.slots[0] = Some(stack("minecraft:stone", 64, 10));
        t.slots[2] = Some(stack("minecraft:stone", 64, 64));
        t.filters[0] = Some(item("minecraft:stone", 64));
        t.settle(0);
        assert_eq!(t.slots[0].as_ref().unwrap().count, 64);
        assert_eq!(t.slots[1].as_ref().unwrap().count, 10);
        assert!(t.slots[2].is_none() && t.slots[3].is_none());
    }

    #[test]
    fn settle_moves_unstackable_items_up_over_gaps() {
        let mut t = Toolbox::default();
        t.slots[2] = Some(Stack::new(tool("minecraft:iron_pickaxe"), 1));
        t.slots[3] = Some(Stack::new(tool("minecraft:iron_pickaxe"), 1));
        t.settle(0);
        assert!(t.slots[0].is_some() && t.slots[1].is_some());
        assert!(t.slots[2].is_none() && t.slots[3].is_none());
    }

    #[test]
    fn insertion_settles_so_slots_fill_in_order() {
        let mut t = Toolbox::default();
        let rest = t.insert_item(3, stack("minecraft:stone", 64, 70), false, false);
        // One slot's worth went in (the last one), the rest comes back; settling moves it up.
        assert_eq!(rest, Some(stack("minecraft:stone", 64, 6)));
        assert_eq!(t.slots[0].as_ref().unwrap().count, 64);
        assert!(t.slots[3].is_none());
    }

    #[test]
    fn a_compartment_holds_four_stacks() {
        let mut t = Toolbox::default();
        let rest = t.distribute_to_compartment(stack("minecraft:stone", 64, 10), 0, false, false);
        // No filter yet: nothing goes in.
        assert_eq!(rest, Some(stack("minecraft:stone", 64, 10)));
        let rest = t.insert_stacked(stack("minecraft:stone", 64, 300), false);
        // The fifth stack's worth goes into the next compartment, which has no filter yet.
        assert!(rest.is_none());
        assert_eq!(t.total_in_compartment(0), 256);
        assert_eq!(t.total_in_compartment(1), 44);
        // `distributeToCompartment` puts one slot's worth in per call, as Create's does (it settles
        // between slots): all a quick-equip refill needs.
        let mut t = Toolbox::default();
        t.insert_item(0, stack("minecraft:stone", 64, 1), false, false);
        assert!(t.distribute_to_compartment(stack("minecraft:stone", 64, 20), 0, false, false).is_none());
        assert_eq!(t.total_in_compartment(0), 21);
    }

    #[test]
    fn taking_from_a_compartment_empties_its_last_slot_first_and_settles() {
        let mut t = Toolbox::default();
        t.insert_stacked(stack("minecraft:stone", 64, 100), false);
        let got = t.take_from_compartment(40, 0, false).unwrap();
        assert_eq!(got.count, 40);
        assert_eq!(t.total_in_compartment(0), 60);
        // Taking more than is there gives what is left.
        let got = t.take_from_compartment(100, 0, false).unwrap();
        assert_eq!(got.count, 60);
        assert_eq!(t.total_in_compartment(0), 0);
        assert!(t.take_from_compartment(5, 0, false).is_none());
    }

    #[test]
    fn simulating_changes_nothing() {
        let mut t = Toolbox::default();
        t.insert_item(0, stack("minecraft:stone", 64, 10), false, false);
        let before = t.clone();
        t.take_from_compartment(5, 0, true);
        t.distribute_to_compartment(stack("minecraft:stone", 64, 5), 0, true, false);
        assert_eq!(t, before);
    }

    #[test]
    fn damageable_tools_share_a_compartment_whatever_their_components() {
        let mut worn = tool("minecraft:iron_pickaxe");
        worn.components = "enchanted".into();
        assert!(can_share(&tool("minecraft:iron_pickaxe"), &worn));
        assert!(!can_share(&tool("minecraft:iron_pickaxe"), &tool("minecraft:iron_axe")));
        // Stackable items need the same components.
        let mut named = item("minecraft:stone", 64);
        named.components = "named".into();
        assert!(!can_share(&item("minecraft:stone", 64), &named));
        assert!(can_share(&item("create:belt_connector", 64), &named_belt()));
    }

    fn named_belt() -> Item {
        let mut belt = item("create:belt_connector", 64);
        belt.components = "first_shaft".into();
        belt
    }

    #[test]
    fn a_linked_slot_is_kept_at_half_a_stack() {
        let filter = item("minecraft:stone", 64);
        assert_eq!(target_amount(64), 32);
        assert_eq!(target_amount(1), 1);
        assert_eq!(target_amount(16), 8);
        assert_eq!(tick_link(None, Some(&filter)), Step::Take(32));
        assert_eq!(tick_link(Some(&stack("minecraft:stone", 64, 10)), Some(&filter)), Step::Take(22));
        assert_eq!(tick_link(Some(&stack("minecraft:stone", 64, 32)), Some(&filter)), Step::Nothing);
        assert_eq!(tick_link(Some(&stack("minecraft:stone", 64, 50)), Some(&filter)), Step::Deposit(18));
    }

    #[test]
    fn a_link_ends_when_the_slot_holds_something_else_or_the_filter_goes() {
        let filter = item("minecraft:stone", 64);
        assert_eq!(tick_link(Some(&stack("minecraft:dirt", 64, 5)), Some(&filter)), Step::Unlink);
        assert_eq!(tick_link(Some(&stack("minecraft:stone", 64, 5)), None), Step::Unlink);
    }

    #[test]
    fn the_range_is_ten_blocks() {
        assert!(within_range([0.5, 0.0, 0.5], [0, 0, 0]));
        assert!(within_range([9.0, 0.0, 0.5], [0, 0, 0]));
        assert!(!within_range([11.0, 0.0, 0.5], [0, 0, 0]));
        assert!(!within_range([0.5, 10.0, 0.5], [0, 0, 0]));
    }

    #[test]
    fn the_view_lays_compartments_out_as_columns() {
        assert_eq!(menu_slot(0), 0);
        assert_eq!(menu_slot(1), 9);
        assert_eq!(menu_slot(3), 27);
        assert_eq!(menu_slot(4), 1);
        assert_eq!(menu_slot(31), 3 * 9 + 7);
        assert_eq!(menu_compartment(8), None);
        assert_eq!(menu_compartment(9 + 2), Some(2));
    }

    #[test]
    fn the_view_round_trips() {
        let mut t = Toolbox::default();
        t.insert_item(0, stack("minecraft:stone", 64, 70), false, false);
        t.insert_item(8, stack("minecraft:dirt", 64, 5), false, false);
        let view = t.to_menu();
        let mut back = t.clone();
        let rejected = back.apply_menu(view, [false; COMPARTMENTS]);
        assert!(rejected.is_empty());
        assert_eq!(back, t);
    }

    #[test]
    fn items_of_another_kind_in_a_compartment_are_returned() {
        let mut t = Toolbox::default();
        t.insert_item(0, stack("minecraft:stone", 64, 10), false, false);
        let mut view = t.to_menu();
        view[menu_slot(1)] = Some(stack("minecraft:dirt", 64, 3));
        view[8] = Some(stack("minecraft:cobblestone", 64, 1)); // the unused column
        let rejected = t.apply_menu(view, [false; COMPARTMENTS]);
        assert_eq!(rejected.len(), 2);
        assert_eq!(t.total_in_compartment(0), 10);
    }

    #[test]
    fn emptying_a_compartment_in_the_view_clears_its_filter_unless_equipped() {
        let mut t = Toolbox::default();
        t.insert_item(0, stack("minecraft:stone", 64, 10), false, false);
        let mut kept = t.clone();
        kept.apply_menu(vec![None; MENU_SLOTS], { let mut k = [false; COMPARTMENTS]; k[0] = true; k });
        assert!(kept.filters[0].is_some());
        t.apply_menu(vec![None; MENU_SLOTS], [false; COMPARTMENTS]);
        assert!(t.filters[0].is_none());
    }

    #[test]
    fn swapping_a_compartments_item_in_the_view_changes_its_filter() {
        let mut t = Toolbox::default();
        t.insert_item(0, stack("minecraft:stone", 64, 10), false, false);
        let mut view = vec![None; MENU_SLOTS];
        view[0] = Some(stack("minecraft:dirt", 64, 4));
        let rejected = t.apply_menu(view, [false; COMPARTMENTS]);
        assert!(rejected.is_empty());
        assert_eq!(t.filters[0], Some(item("minecraft:dirt", 64)));
    }

    #[test]
    fn insert_stacked_fills_matching_slots_before_empty_ones() {
        let mut t = Toolbox::default();
        t.insert_item(4, stack("minecraft:dirt", 64, 10), false, false);
        t.insert_item(0, stack("minecraft:stone", 64, 10), false, false);
        let rest = t.insert_stacked(stack("minecraft:dirt", 64, 5), false);
        assert!(rest.is_none());
        assert_eq!(t.total_in_compartment(1), 15);
        // Limited mode refuses an item no compartment holds.
        let rest = t.insert_stacked(stack("minecraft:sand", 64, 5), true);
        assert_eq!(rest, Some(stack("minecraft:sand", 64, 5)));
    }

    #[test]
    fn the_comparator_reads_how_full_it_is() {
        let mut t = Toolbox::default();
        assert_eq!(t.redstone(), 0);
        t.insert_item(0, stack("minecraft:stone", 64, 1), false, false);
        assert_eq!(t.redstone(), 1);
        let mut t = Toolbox::default();
        for c in 0..COMPARTMENTS {
            assert!(t.insert_stacked(stack(&format!("minecraft:item_{c}"), 64, 256), false).is_none());
        }
        assert_eq!(t.redstone(), 15);
    }

    #[test]
    fn a_sneak_click_with_an_item_equips_its_compartment_or_ends_the_link() {
        let mut filters: [Option<Item>; COMPARTMENTS] = Default::default();
        filters[2] = Some(item("minecraft:stone", 64));
        let counts = [0, 0, 5, 0, 0, 0, 0, 0];
        let stone = item("minecraft:stone", 64);
        assert_eq!(sneak_click(&filters, counts, Some(&stone), None), Click::Equip(2));
        assert_eq!(sneak_click(&filters, counts, Some(&stone), Some(2)), Click::Unequip);
        assert_eq!(sneak_click(&filters, counts, Some(&item("minecraft:dirt", 64)), None), Click::Nothing);
    }

    #[test]
    fn a_sneak_click_with_an_empty_hand_steps_through_the_compartments() {
        let mut filters: [Option<Item>; COMPARTMENTS] = Default::default();
        filters[1] = Some(item("minecraft:stone", 64));
        filters[5] = Some(item("minecraft:dirt", 64));
        let counts = [0, 3, 0, 0, 0, 9, 0, 0];
        assert_eq!(sneak_click(&filters, counts, None, None), Click::Equip(1));
        assert_eq!(sneak_click(&filters, counts, None, Some(1)), Click::Equip(5));
        assert_eq!(sneak_click(&filters, counts, None, Some(5)), Click::Unequip);
        assert_eq!(sneak_click(&filters, [0; 8], None, None), Click::Nothing);
    }

    #[test]
    fn colours() {
        assert_eq!(colour_of("create:red_toolbox"), Some("red"));
        assert_eq!(colour_of("create:light_blue_toolbox"), Some("light_blue"));
        assert_eq!(colour_of("create:toolbox"), None);
        assert!(is_toolbox("create:brown_toolbox"));
        assert_eq!(dye_colour("minecraft:lime_dye"), Some("lime"));
        assert_eq!(dye_colour("minecraft:bone_meal"), None);
    }

    #[test]
    fn tools_and_armour_are_damageable_by_name() {
        assert!(damageable_id("minecraft:diamond_pickaxe"));
        assert!(damageable_id("minecraft:shears"));
        assert!(!damageable_id("minecraft:stone"));
    }
}
