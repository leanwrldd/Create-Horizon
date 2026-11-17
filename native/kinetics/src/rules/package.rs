//! Packages: the cardboard box (`PackageItem`, `PackageStyles`, Catnip's `Glob`) that holds up to
//! nine stacks and an address, the address rules every package port and chain junction applies
//! (`PackageItem.matchAddress`), how a Packager fills a box from an inventory
//! (`PackagerBlockEntity.attemptToSend`, with and without the Stock Ticker's requests) and
//! whether a box's items fit an inventory (`DefaultUnpackingHandler.unpack`, simulated).
//!
//! No game types: a stack is a key `K` (the item with its components, equal when
//! `ItemStack.isSameItemSameComponents`) and a count, so the unit tests run in seconds. The glue
//! is `machines/package.rs` (the items) and `machines/packager.rs` (the block).
//!
//! Known differences from Create, for the README:
//! - Create's package entity (`PackageEntity`, thrown or set down with a click) is not there:
//!   Horizon has no entity type of a mod's own. A thrown or set-down package is an item entity
//!   (`machines/package.rs`). It can be picked up as any item can.
//! - Catnip's `Glob` source is not published with the mod: [`Glob`] follows the reference port's
//!   reading of it (`*` is `.*`, `?` is `.`, `[a-z]` and `{a,b}` as in a shell glob, `{!x}` a
//!   negative look-around, every other character literal).
//! - `InventorySummary` keeps its stacks in the order they were added (Create keeps them in an
//!   identity hash map, whose order isn't defined).

/// `PackageItem.SLOTS`.
pub const SLOTS: usize = 9;

/// `AllDataComponents.PACKAGE_ADDRESS`, `PACKAGE_CONTENTS`, `PACKAGE_ORDER_DATA` and
/// `PACKAGE_ORDER_CONTEXT`.
pub const ADDRESS: &str = "create:package_address";
pub const CONTENTS: &str = "create:package_contents";
pub const ORDER_DATA: &str = "create:package_order_data";
pub const ORDER_CONTEXT: &str = "create:package_order_context";
/// Every component a package carries.
pub const COMPONENTS: [&str; 4] = [ADDRESS, CONTENTS, ORDER_DATA, ORDER_CONTEXT];

/// `PackageStyles.PackageStyle`: a box's size in pixels and its rigging offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    /// `getItemId`: `cardboard_package_12x12`, `rare_creeper_package`.
    pub item: &'static str,
    pub width: u8,
    pub height: u8,
    pub rigging: f32,
    pub rare: bool,
}

const fn cardboard(item: &'static str, width: u8, height: u8, rigging: f32) -> Style {
    Style { item, width, height, rigging, rare: false }
}

/// `PackageStyles.rare(name)`: 12 by 10, rigging 21.
const fn rare(item: &'static str) -> Style {
    Style { item, width: 12, height: 10, rigging: 21.0, rare: true }
}

/// `PackageStyles.STYLES`, in Create's order (which is `ALL_BOXES`'): four cardboard boxes, then
/// the ten rare ones.
pub const STYLES: [Style; 14] = [
    cardboard("create:cardboard_package_12x12", 12, 12, 23.0),
    cardboard("create:cardboard_package_10x12", 10, 12, 22.0),
    cardboard("create:cardboard_package_10x8", 10, 8, 18.0),
    cardboard("create:cardboard_package_12x10", 12, 10, 21.0),
    rare("create:rare_creeper_package"),
    rare("create:rare_darcy_package"),
    rare("create:rare_evan_package"),
    rare("create:rare_jinx_package"),
    rare("create:rare_kryppers_package"),
    rare("create:rare_simi_package"),
    rare("create:rare_starlotte_package"),
    rare("create:rare_thunder_package"),
    rare("create:rare_up_package"),
    rare("create:rare_vector_package"),
];

/// `PackageStyles.RARE_CHANCE`.
pub const RARE_CHANCE: u32 = 7500;

/// The style of a package item, `None` for any other item.
pub fn style(item: &str) -> Option<&'static Style> {
    STYLES.iter().find(|s| s.item == item)
}

/// `PackageItem.isPackage`.
pub fn is_package(item: &str) -> bool {
    style(item).is_some()
}

/// `PackageStyles.getRandomBox`: one in [`RARE_CHANCE`] comes from the rare boxes, else a
/// cardboard one; `next(n)` is `Random.nextInt(n)`.
pub fn random_box(mut next: impl FnMut(u32) -> u32) -> &'static str {
    let (from, len) = if next(RARE_CHANCE) == 0 { (4, 10) } else { (0, 4) };
    STYLES[from + (next(len as u32) as usize).min(len - 1)].item
}

/// `PackageStyles.getDefaultBox`.
pub fn default_box() -> &'static str {
    STYLES[0].item
}

/// `PackageItem.getWidth`, in blocks.
pub fn width(item: &str) -> f32 {
    style(item).map_or(1.0, |s| f32::from(s.width) / 16.0)
}

/// `PackageItem.getHeight`, in blocks.
pub fn height(item: &str) -> f32 {
    style(item).map_or(1.0, |s| f32::from(s.height) / 16.0)
}

/// `PackageItem.getHookDistance`, in blocks.
pub fn hook_distance(item: &str) -> f32 {
    style(item).map_or(1.0, |s| s.rigging / 16.0)
}

// --- addresses -------------------------------------------------------------------------------

/// Java's `Character.isWhitespace`.
fn java_whitespace(c: char) -> bool {
    match c {
        '\u{00A0}' | '\u{2007}' | '\u{202F}' | '\u{0085}' => false,
        '\u{001C}'..='\u{001F}' => true,
        c => c.is_whitespace(),
    }
}

/// Java's `String.isBlank`.
pub fn is_blank(text: &str) -> bool {
    text.chars().all(java_whitespace)
}

/// `PackageItem.matchAddress(boxAddress, address)`: a blank address takes only a blank box, `*`
/// on either side matches, an equal address matches, and else either one read as a glob matches
/// the other as a whole.
pub fn match_address(box_address: &str, address: &str) -> bool {
    if is_blank(address) {
        return is_blank(box_address);
    }
    if address == "*" || box_address == "*" || address == box_address {
        return true;
    }
    Glob::new(box_address).matches(address) || Glob::new(address).matches(box_address)
}

/// One element of a glob as it becomes a regular expression.
#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Lit(char),
    /// `.`: any character but a line terminator.
    Any,
    /// `.*`.
    Star,
    /// `[...]`: ranges, negated with `!`.
    Set(bool, Vec<(char, char)>),
    /// `{a,b}`: `(?:a|b)`.
    Group(Vec<Vec<Tok>>),
    /// `{!a,b}` where the glob is anchored: `(?!a|b)`, followed by `.*`.
    NotAhead(Vec<Vec<Tok>>),
    /// `{!a,b}` after a `*`: `(?<!a|b)`.
    NotBehind(Vec<Vec<Tok>>),
}

/// A glob compiled as `Glob.toRegexPattern(glob, "")`. A glob it rejects is the empty pattern,
/// which matches only the empty text.
#[derive(Clone, Debug, PartialEq)]
pub struct Glob(Vec<Tok>);

impl Glob {
    pub fn new(glob: &str) -> Glob {
        Glob(parse(glob).unwrap_or_default())
    }

    /// `String.matches`: the whole text. A look-behind holding a `*` has no bounded length,
    /// which Java's regex rejects (Create would throw): it matches nothing here.
    pub fn matches(&self, text: &str) -> bool {
        let chars: Vec<char> = text.chars().collect();
        go(&self.0, &chars, 0, &mut |end| end == chars.len())
    }
}

fn parse(glob: &str) -> Option<Vec<Tok>> {
    let g: Vec<char> = glob.chars().collect();
    let next = |i: usize| g.get(i).copied().unwrap_or('\0');
    let mut out: Vec<Tok> = Vec::new();
    // Inside `{...}`: the alternatives so far, and whether it is a negative look-around.
    let mut group: Option<(Vec<Vec<Tok>>, bool)> = None;
    let mut anchored = true;
    let mut negative_anchored = true;
    let mut i = 0;
    let push = |out: &mut Vec<Tok>, group: &mut Option<(Vec<Vec<Tok>>, bool)>, t: Tok| match group {
        Some((alternatives, _)) => alternatives.last_mut().expect("a group has an alternative").push(t),
        None => out.push(t),
    };
    while i < g.len() {
        let c = g[i];
        i += 1;
        match c {
            '*' => {
                push(&mut out, &mut group, Tok::Star);
                if group.is_none() {
                    anchored = false;
                }
            }
            '?' => {
                push(&mut out, &mut group, Tok::Any);
                if group.is_none() {
                    anchored = true;
                }
            }
            ',' => match &mut group {
                Some((alternatives, _)) => alternatives.push(Vec::new()),
                None => {
                    out.push(Tok::Lit(','));
                    anchored = true;
                }
            },
            '[' => {
                if next(i) == ']' || next(i) == '!' && next(i + 1) == ']' {
                    return None;
                }
                let mut negated = false;
                let mut items: Vec<(char, char)> = Vec::new();
                if next(i) == '^' {
                    items.push(('^', '^'));
                    i += 1;
                } else {
                    if next(i) == '!' {
                        negated = true;
                        i += 1;
                    }
                    if next(i) == '-' {
                        items.push(('-', '-'));
                        i += 1;
                    }
                }
                let mut range_start = false;
                let mut last = '\0';
                let mut c = c;
                while i < g.len() {
                    c = g[i];
                    i += 1;
                    if c == ']' {
                        break;
                    }
                    if c == '\\' {
                        if i == g.len() {
                            return None;
                        }
                        let escaped = next(i);
                        i += 1;
                        items.push((escaped, escaped));
                        continue;
                    }
                    if c == '-' {
                        if !range_start {
                            return None;
                        }
                        c = next(i);
                        i += 1;
                        if c == '\0' || c == ']' {
                            // `[a-]`: the dash is itself.
                            items.push(('-', '-'));
                            break;
                        }
                        if c < last {
                            return None;
                        }
                        items.pop();
                        items.push((last, c));
                        range_start = false;
                    } else {
                        items.push((c, c));
                        range_start = true;
                        last = c;
                    }
                }
                if c != ']' {
                    return None;
                }
                push(&mut out, &mut group, Tok::Set(negated, items));
                if group.is_none() {
                    anchored = true;
                }
            }
            '\\' => {
                if i == g.len() {
                    return None;
                }
                let escaped = g[i];
                i += 1;
                push(&mut out, &mut group, Tok::Lit(escaped));
                if group.is_none() {
                    anchored = true;
                }
            }
            '{' => {
                if group.is_some() {
                    return None;
                }
                let negative = next(i) == '!';
                if negative {
                    negative_anchored = anchored;
                    i += 1;
                }
                group = Some((vec![Vec::new()], negative));
            }
            '}' => match group.take() {
                Some((alternatives, negative)) => {
                    if !negative {
                        out.push(Tok::Group(alternatives));
                    } else if negative_anchored {
                        out.push(Tok::NotAhead(alternatives));
                        if anchored {
                            out.push(Tok::Star);
                            anchored = false;
                        }
                    } else {
                        out.push(Tok::NotBehind(alternatives));
                    }
                }
                None => {
                    out.push(Tok::Lit('}'));
                    anchored = true;
                }
            },
            c => {
                push(&mut out, &mut group, Tok::Lit(c));
                if group.is_none() {
                    anchored = true;
                }
            }
        }
    }
    if group.is_some() {
        return None;
    }
    Some(out)
}

/// Java's `.`: anything but a line terminator.
fn any(c: char) -> bool {
    !matches!(c, '\n' | '\r' | '\u{0085}' | '\u{2028}' | '\u{2029}')
}

/// Whether `toks` match `s` from `i`, ending where `k` accepts (a backtracking matcher).
fn go(toks: &[Tok], s: &[char], i: usize, k: &mut dyn FnMut(usize) -> bool) -> bool {
    let Some((t, rest)) = toks.split_first() else {
        return k(i);
    };
    match t {
        Tok::Lit(c) => s.get(i) == Some(c) && go(rest, s, i + 1, k),
        Tok::Any => s.get(i).is_some_and(|&c| any(c)) && go(rest, s, i + 1, k),
        Tok::Set(negated, items) => {
            s.get(i).is_some_and(|&c| items.iter().any(|&(a, b)| (a..=b).contains(&c)) != *negated)
                && go(rest, s, i + 1, k)
        }
        Tok::Star => {
            let mut end = i;
            while end < s.len() && any(s[end]) {
                end += 1;
            }
            (i..=end).rev().any(|j| go(rest, s, j, k))
        }
        Tok::Group(alternatives) => {
            alternatives.iter().any(|alternative| go(alternative, s, i, &mut |j| go(rest, s, j, k)))
        }
        Tok::NotAhead(alternatives) => {
            !alternatives.iter().any(|alternative| go(alternative, s, i, &mut |_| true)) && go(rest, s, i, k)
        }
        Tok::NotBehind(alternatives) => {
            if alternatives.iter().flatten().any(|t| *t == Tok::Star) {
                return false;
            }
            !alternatives.iter().any(|alternative| (0..=i).any(|j| go(alternative, s, j, &mut |end| end == i)))
                && go(rest, s, i, k)
        }
    }
}

// --- the order data ------------------------------------------------------------------------------

/// `BigItemStack` without the game: a stack's key and its count (which may pass a stack's size).
pub type Big<K> = (K, u32);

/// `PackageOrderWithCrafts.CraftingEntry`: a recipe's pattern and how many times it is crafted.
#[derive(Clone, Debug, PartialEq)]
pub struct Craft<K> {
    pub pattern: Vec<Big<K>>,
    pub count: u32,
}

/// `PackageOrderWithCrafts`: what was ordered, and the recipes it was ordered through.
#[derive(Clone, Debug, PartialEq)]
pub struct Context<K> {
    pub stacks: Vec<Big<K>>,
    pub crafts: Vec<Craft<K>>,
}

impl<K> Default for Context<K> {
    fn default() -> Self {
        Self { stacks: Vec::new(), crafts: Vec::new() }
    }
}

impl<K: Clone> Context<K> {
    /// `PackageOrderWithCrafts.isEmpty`: no ordered stacks.
    pub fn is_empty(&self) -> bool {
        self.stacks.is_empty()
    }

    /// `PackageOrderWithCrafts.singleRecipe`.
    pub fn single_recipe(pattern: Vec<Big<K>>) -> Self {
        Self { stacks: Vec::new(), crafts: vec![Craft { pattern, count: 1 }] }
    }
}

/// `PackageItem.PackageOrderData`: where a box sits in an order, so a Repackager can join the
/// pieces of one.
#[derive(Clone, Debug, PartialEq)]
pub struct OrderData<K> {
    pub order_id: i32,
    pub link_index: i32,
    pub is_final_link: bool,
    pub fragment_index: i32,
    pub is_final: bool,
    pub context: Option<Context<K>>,
}

// --- filling a box -------------------------------------------------------------------------------

/// A slot of an inventory as packing sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Slot<K> {
    pub key: K,
    pub count: u32,
    /// The item's stack size.
    pub max: u32,
    /// `!canFitInsideContainerItems`: packages and shulker boxes, which go in a box alone.
    pub bulky: bool,
    /// A package (`instanceof PackageItem`): sent as it is, not boxed again.
    pub package: bool,
}

/// The nine slots of a box being filled (an `ItemStackHandler(9)`).
pub type Contents<K> = [Option<Big<K>>; SLOTS];

pub fn empty_contents<K>() -> Contents<K> {
    std::array::from_fn(|_| None)
}

/// `ItemHandlerHelper.insertItemStacked` into a box of nine slots (each holds up to 64, and up
/// to the item's stack size): onto the same item first, then into empty slots. Returns what is
/// left of `count`.
pub fn insert_stacked<K: PartialEq + Clone>(contents: &mut Contents<K>, key: &K, count: u32, max: u32) -> u32 {
    let limit = max.clamp(1, 64);
    let mut left = count;
    if limit > 1 {
        for slot in contents.iter_mut() {
            if left == 0 {
                break;
            }
            if let Some((held, have)) = slot
                && held == key
            {
                let moved = limit.saturating_sub(*have).min(left);
                *have += moved;
                left -= moved;
            }
        }
    }
    for slot in contents.iter_mut() {
        if left == 0 {
            break;
        }
        if slot.is_none() {
            let moved = limit.min(left);
            *slot = Some((key.clone(), moved));
            left -= moved;
        }
    }
    left
}

/// `PackagingRequest`: the Stock Ticker's order for one item through one packager.
#[derive(Clone, Debug, PartialEq)]
pub struct Request<K> {
    pub item: K,
    pub count: u32,
    pub address: String,
    pub link_index: i32,
    pub final_link: bool,
    /// `packageCounter`: the index the next box of this link gets.
    pub package_counter: i32,
    pub order_id: i32,
    pub context: Option<Context<K>>,
}

/// What [`pack`] decided.
#[derive(Clone, Debug, PartialEq)]
pub struct Packed<K> {
    /// `(slot, count)` taken from the inventory, in order.
    pub taken: Vec<(usize, u32)>,
    pub contents: Contents<K>,
    /// The slot a package was taken from, when a package was packed as it is
    /// (`extractedPackageItem`: the box itself is sent, not a new one).
    pub package_slot: Option<usize>,
    /// For a request: the address and order the box gets.
    pub address: Option<String>,
    pub order: Option<OrderData<K>>,
}

/// `PackagerBlockEntity.attemptToSend`'s packing: nine passes over the inventory's slots, each
/// taking up to 64 from a slot (`extractItem(slot, 64)`) into the box. A bulky stack goes alone,
/// and ends the packing. With `requests` (the Stock Ticker's), only what the first request asks
/// for is taken, and finished requests are removed from the list. `None` when nothing was there
/// (the first request is dropped then, as Create does).
pub fn pack<K: PartialEq + Clone>(
    slots: &[Option<Slot<K>>],
    mut requests: Option<&mut Vec<Request<K>>>,
) -> Option<Packed<K>> {
    let mut left: Vec<u32> = slots.iter().map(|s| s.as_ref().map_or(0, |s| s.count)).collect();
    let mut taken: Vec<(usize, u32)> = Vec::new();
    let mut contents: Contents<K> = empty_contents();
    let mut any_item = false;
    let mut package_slot = None;

    let request_queue = requests.is_some();
    // The request being served, as an index into the list (always 0 in Create: it removes a
    // finished request from the front), and the data the box gets.
    let mut next_request: Option<Request<K>> = None;
    let mut fixed_address: Option<String> = None;
    let mut fixed_order_id = 0;
    let mut link_index = 0;
    let mut final_link = false;
    let mut package_index = 0;
    let mut final_package = false;
    let mut context: Option<Context<K>> = None;

    if let Some(queue) = requests.as_deref_mut()
        && let Some(first) = queue.first_mut()
    {
        fixed_address = Some(first.address.clone());
        fixed_order_id = first.order_id;
        link_index = first.link_index;
        final_link = first.final_link;
        package_index = first.package_counter;
        first.package_counter += 1;
        context = first.context.clone();
        next_request = Some(first.clone());
    }
    if request_queue && next_request.is_none() {
        return None;
    }

    'outer: for _ in 0..SLOTS {
        let mut continue_packing = true;
        while continue_packing {
            continue_packing = false;
            for (index, slot) in slots.iter().enumerate() {
                let Some(slot) = slot else { continue };
                let initial = match &next_request {
                    Some(request) if request_queue => request.count.min(64),
                    _ => 64,
                };
                // `extractItem(slot, initial, true)`.
                let extracted = left[index].min(initial).min(slot.max.max(1));
                if extracted == 0 {
                    continue;
                }
                if let Some(request) = &next_request
                    && request.item != slot.key
                {
                    continue;
                }
                if slot.bulky && any_item {
                    continue;
                }
                any_item = true;
                let leftovers = insert_stacked(&mut contents, &slot.key, extracted, slot.max);
                let transferred = extracted - leftovers;
                if transferred > 0 {
                    left[index] -= transferred;
                    match taken.iter_mut().find(|t| t.0 == index) {
                        Some(entry) => entry.1 += transferred,
                        None => taken.push((index, transferred)),
                    }
                }
                if slot.package {
                    package_slot = Some(index);
                }

                let Some(request) = next_request.as_mut() else {
                    if slot.bulky {
                        break 'outer;
                    }
                    continue;
                };
                request.count -= transferred.min(request.count);
                if let Some(queue) = requests.as_deref_mut()
                    && let Some(first) = queue.first_mut()
                {
                    first.count = request.count;
                }
                if request.count != 0 {
                    if slot.bulky {
                        break 'outer;
                    }
                    continue;
                }

                final_package = true;
                let queue = requests.as_deref_mut().expect("requests exist in request mode");
                let finished = queue.remove(0);
                let Some(following) = queue.first_mut() else {
                    next_request = None;
                    break 'outer;
                };
                if fixed_address.as_deref() != Some(following.address.as_str()) {
                    next_request = None;
                    break 'outer;
                }
                if fixed_order_id != following.order_id {
                    next_request = None;
                    break 'outer;
                }
                following.package_counter = finished.package_counter;
                final_package = false;
                continue_packing = true;
                if following.context.is_some() {
                    context = following.context.clone();
                }
                next_request = Some(following.clone());
                if slot.bulky {
                    break 'outer;
                }
                break;
            }
        }
    }

    if !any_item {
        if next_request.is_some()
            && let Some(queue) = requests.as_deref_mut()
            && !queue.is_empty()
        {
            queue.remove(0);
        }
        return None;
    }

    let order = request_queue.then(|| OrderData {
        order_id: fixed_order_id,
        link_index,
        is_final_link: final_link,
        fragment_index: package_index,
        is_final: final_package,
        context,
    });
    Some(Packed { taken, contents, package_slot, address: fixed_address, order })
}

// --- unpacking -----------------------------------------------------------------------------------

/// A slot of the inventory a box is unpacked into: its stack and the slot's limit
/// (`getSlotLimit`).
#[derive(Clone, Debug, PartialEq)]
pub struct Target<K> {
    pub stack: Option<Big<K>>,
    pub limit: u32,
}

/// What `insertItem(slot, stack, true)` would take of `count` of `key`: up to the slot's limit
/// and the item's stack size, on an empty slot or one holding the same item.
fn takes<K: PartialEq>(slot: &Target<K>, key: &K, count: u32, max: u32) -> u32 {
    let limit = slot.limit.min(max.max(1));
    match &slot.stack {
        None => count.min(limit),
        Some((held, have)) if held == key => count.min(limit.saturating_sub(*have)),
        Some(_) => 0,
    }
}

/// `DefaultUnpackingHandler.unpack` with `simulate`: whether every stack of the box finds room,
/// slot by slot as Create counts it. `items` are the box's stacks, `max` the item's stack size.
pub fn unpack_fits<K: PartialEq + Clone>(slots: &[Target<K>], items: &[Big<K>], max: &dyn Fn(&K) -> u32) -> bool {
    let mut items: Vec<Big<K>> = items.to_vec();
    for slot in slots {
        let mut in_slot: Option<Big<K>> = slot.stack.clone();
        let mut added = 0u32;
        for index in 0..items.len() {
            let (key, count) = items[index].clone();
            if count == 0 {
                continue;
            }
            // `targetInv.insertItem(slot, toInsert, true).getCount() == toInsert.getCount()`.
            let inserted = takes(slot, &key, count, max(&key));
            if inserted == 0 {
                continue;
            }
            match &in_slot {
                None => {
                    // An empty slot takes the stack, up to the slot's limit.
                    let put = count.min(slot.limit);
                    items[index].1 = count - put;
                    in_slot = Some((key, put));
                }
                Some((held, have)) => {
                    if *held != key {
                        continue;
                    }
                    let slot_limit = max(held).min(slot.limit);
                    let insertable = count.min(slot_limit.saturating_sub(*have).saturating_sub(added));
                    let put = inserted.min(insertable);
                    added += put;
                    items[index].1 = count - put;
                }
            }
        }
    }
    items.iter().all(|(_, count)| *count == 0)
}

// --- throwing --------------------------------------------------------------------------------------

/// `PackageItem.getUseDuration`.
pub const USE_DURATION: i32 = 72000;

/// `PackageItem.getPackageVelocity`: how hard a package is thrown after `ticks` of holding.
pub fn throw_velocity(ticks: i32) -> f32 {
    let f = ticks as f32 / 20.0;
    ((f * f + f * 2.0) / 3.0).min(1.0)
}

/// `PackageItem.releaseUsing`: a throw needs a velocity of at least 0.1, and goes along the look
/// direction at twice it.
pub fn throw_motion(look: [f64; 3], velocity: f32) -> Option<[f64; 3]> {
    (velocity >= 0.1).then(|| look.map(|v| v * f64::from(velocity) * 2.0))
}

// --- summaries -------------------------------------------------------------------------------------

/// `BigItemStack.INF`: a count this large is "infinite" (a Creative Crate, a bottomless handler).
pub const INF: u32 = 1_000_000_000;

/// `InventorySummary` without the game: stacks merged by key, in the order they came.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary<K> {
    pub stacks: Vec<Big<K>>,
    /// `InventorySummary.contributingLinks`: how many Stock Links contributed to a network's
    /// summary.
    pub contributing_links: u32,
    /// `InventorySummary.totalCount`: stops growing once it reaches [`INF`].
    pub total_count: i64,
}

impl<K> Default for Summary<K> {
    fn default() -> Self {
        Self { stacks: Vec::new(), contributing_links: 0, total_count: 0 }
    }
}

impl<K: PartialEq + Clone> Summary<K> {
    /// `InventorySummary.add(stack, count)`: an existing entry stops growing at [`INF`]. A
    /// count never goes below zero here (Create's int can: nothing reads a negative entry).
    pub fn add(&mut self, key: &K, count: i64) {
        if count == 0 {
            return;
        }
        if self.total_count < i64::from(INF) {
            self.total_count += count;
        }
        match self.stacks.iter_mut().find(|(k, _)| k == key) {
            Some((_, have)) => {
                if *have < INF {
                    *have = (i64::from(*have) + count).clamp(0, i64::from(u32::MAX)) as u32;
                }
            }
            None => self.stacks.push((key.clone(), count.clamp(0, i64::from(u32::MAX)) as u32)),
        }
    }

    /// `InventorySummary.add(InventorySummary)`: every stack, and the links that contributed.
    pub fn add_summary(&mut self, other: &Summary<K>) {
        for (key, count) in &other.stacks {
            self.add(key, i64::from(*count));
        }
        self.contributing_links += other.contributing_links;
    }

    /// `InventorySummary.copy`: the stacks only (`contributingLinks` isn't carried over).
    pub fn copy(&self) -> Summary<K> {
        let mut copy = Summary::default();
        for (key, count) in &self.stacks {
            copy.add(key, i64::from(*count));
        }
        copy
    }

    /// `InventorySummary.erase`.
    pub fn erase(&mut self, key: &K) -> bool {
        match self.stacks.iter().position(|(k, _)| k == key) {
            Some(index) => {
                let (_, count) = self.stacks.remove(index);
                self.total_count -= i64::from(count);
                true
            }
            None => false,
        }
    }

    /// `InventorySummary.getCountOf`.
    pub fn count_of(&self, key: &K) -> u32 {
        self.stacks.iter().find(|(k, _)| k == key).map_or(0, |(_, count)| *count)
    }

    /// `InventorySummary.getTotalOfMatching`.
    pub fn total_of_matching(&self, mut filter: impl FnMut(&K) -> bool) -> i64 {
        self.stacks.iter().filter(|(k, _)| filter(k)).map(|(_, count)| i64::from(*count)).sum()
    }

    /// `InventorySummary.getStacksByCount`: the most first (a stable sort, `BigItemStack.comparator`).
    pub fn stacks_by_count(&self) -> Vec<Big<K>> {
        let mut stacks = self.stacks.clone();
        stacks.sort_by(|a, b| b.1.cmp(&a.1));
        stacks
    }

    pub fn is_empty(&self) -> bool {
        self.stacks.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_and_items() {
        assert_eq!(STYLES.len(), 14);
        assert!(is_package("create:cardboard_package_10x8"));
        assert!(is_package("create:rare_vector_package"));
        assert!(!is_package("create:cardboard"));
        assert_eq!(default_box(), "create:cardboard_package_12x12");
        assert_eq!(style("create:rare_up_package").map(|s| s.rigging), Some(21.0));
        assert_eq!(width("create:cardboard_package_10x8"), 10.0 / 16.0);
        assert_eq!(height("create:cardboard_package_10x8"), 0.5);
        assert_eq!(hook_distance("create:cardboard_package_12x12"), 23.0 / 16.0);
        // One in 7,500 draws a rare box.
        let mut draws = [1u32, 2].into_iter();
        assert_eq!(random_box(|_| draws.next().unwrap()), STYLES[2].item);
        let mut draws = [0u32, 0].into_iter();
        assert_eq!(random_box(|n| if n == RARE_CHANCE { 0 } else { draws.next().unwrap() }), STYLES[4].item);
    }

    #[test]
    fn addresses_match_as_create_matches_them() {
        // Blank addresses take only blank boxes.
        assert!(match_address("", ""));
        assert!(match_address("  ", "\t"));
        assert!(!match_address("Home", ""));
        assert!(!match_address("", "Home"));
        // `*` on either side, equal strings.
        assert!(match_address("anything", "*"));
        assert!(match_address("*", "Base"));
        assert!(match_address("Base", "Base"));
        assert!(!match_address("Base", "base"), "case matters");
        // Either side as a glob.
        assert!(match_address("Farm*", "Farm North"));
        assert!(match_address("Farm North", "Farm*"));
        assert!(match_address("Farm ?", "Farm 1"));
        assert!(!match_address("Farm ?", "Farm 12"));
        assert!(match_address("{Farm,Mine} *", "Mine 3"));
        assert!(!match_address("{Farm,Mine} *", "Mill 3"));
        assert!(match_address("Storage[0-9]", "Storage7"));
        assert!(!match_address("Storage[!0-9]", "Storage7"));
        assert!(match_address("Storage[!0-9]", "StorageX"));
        // Regex characters are literal.
        assert!(match_address("a.b", "a.b"));
        assert!(!match_address("a.b", "axb"));
        assert!(match_address("(x)+", "(x)+"));
        // Escapes.
        assert!(Glob::new("a\\*").matches("a*"));
        assert!(!Glob::new("a\\*").matches("ab"));
        // Negative groups: anchored, a look-ahead; after a `*`, a look-behind.
        assert!(Glob::new("{!Farm}").matches("Mine"));
        assert!(!Glob::new("{!Farm}").matches("Farmhouse"));
        assert!(Glob::new("*{!.txt}").matches("notes.md"));
        assert!(!Glob::new("*{!.txt}").matches("notes.txt"));
        // A glob that can't be read is the empty pattern.
        assert!(!Glob::new("[]").matches("x"));
        assert!(Glob::new("{a").matches(""));
        assert!(!Glob::new("[b-a]").matches("a"));
        // `.` stops at line ends.
        assert!(!Glob::new("a*").matches("a\nb"));
        assert!(Glob::new("[a-]").matches("-"));
        assert!(Glob::new("x,y").matches("x,y"));
    }

    fn slot(key: &'static str, count: u32) -> Option<Slot<&'static str>> {
        let max = if key == "egg" { 16 } else if key == "package" || key == "shulker" { 1 } else { 64 };
        Some(Slot { key, count, max, bulky: key == "package" || key == "shulker", package: key == "package" })
    }

    #[test]
    fn insert_stacked_fills_the_same_item_first() {
        let mut box_: Contents<&str> = empty_contents();
        assert_eq!(insert_stacked(&mut box_, &"iron", 40, 64), 0);
        assert_eq!(insert_stacked(&mut box_, &"gold", 5, 64), 0);
        assert_eq!(insert_stacked(&mut box_, &"iron", 40, 64), 0);
        assert_eq!(box_[0], Some(("iron", 64)));
        assert_eq!(box_[1], Some(("gold", 5)));
        assert_eq!(box_[2], Some(("iron", 16)));
        // A full box leaves the rest.
        let mut full: Contents<&str> = empty_contents();
        for _ in 0..9 {
            assert_eq!(insert_stacked(&mut full, &"cobble", 64, 64), 0);
        }
        assert_eq!(insert_stacked(&mut full, &"cobble", 5, 64), 5);
        // Eggs stack to 16.
        let mut eggs: Contents<&str> = empty_contents();
        assert_eq!(insert_stacked(&mut eggs, &"egg", 40, 16), 0);
        assert_eq!(&eggs[..3], &[Some(("egg", 16)), Some(("egg", 16)), Some(("egg", 8))]);
    }

    #[test]
    fn packing_takes_up_to_nine_stacks_in_slot_order() {
        // Twelve slots of 64 cobblestone: the first nine fill the box.
        let inventory: Vec<_> = (0..12).map(|_| slot("cobble", 64)).collect();
        let packed = pack(&inventory, None).unwrap();
        assert_eq!(packed.taken, (0..9).map(|i| (i, 64)).collect::<Vec<_>>());
        assert!(packed.contents.iter().all(|c| *c == Some(("cobble", 64))));
        // Partial stacks merge: 10 + 10 iron become one stack of 20.
        let inventory = vec![slot("iron", 10), None, slot("gold", 5), slot("iron", 10)];
        let packed = pack(&inventory, None).unwrap();
        assert_eq!(packed.taken, vec![(0, 10), (2, 5), (3, 10)]);
        assert_eq!(packed.contents[0], Some(("iron", 20)));
        assert_eq!(packed.contents[1], Some(("gold", 5)));
        assert_eq!(packed.contents[2], None);
        // Eggs: 40 take three slots.
        let packed = pack(&[slot("egg", 16), slot("egg", 16), slot("egg", 8)], None).unwrap();
        assert_eq!(&packed.contents[..3], &[Some(("egg", 16)), Some(("egg", 16)), Some(("egg", 8))]);
        // A full box leaves the rest in the inventory.
        let mut inventory: Vec<_> = (0..9).map(|_| slot("cobble", 64)).collect();
        inventory.push(slot("dirt", 3));
        let packed = pack(&inventory, None).unwrap();
        assert!(packed.taken.iter().all(|t| t.0 < 9));
        // Nothing there.
        assert_eq!(pack::<&str>(&[None, None], None), None);
    }

    #[test]
    fn a_bulky_stack_goes_alone() {
        // A package first: it is the box.
        let packed = pack(&[slot("package", 1), slot("iron", 4)], None).unwrap();
        assert_eq!(packed.taken, vec![(0, 1)]);
        assert_eq!(packed.package_slot, Some(0));
        // Items first: the package is skipped and stays.
        let packed = pack(&[slot("iron", 4), slot("package", 1)], None).unwrap();
        assert_eq!(packed.taken, vec![(0, 4)]);
        // A shulker box goes alone as well.
        let packed = pack(&[slot("shulker", 1), slot("iron", 4)], None).unwrap();
        assert_eq!(packed.taken, vec![(0, 1)]);
    }

    fn request(item: &'static str, count: u32, address: &str, order_id: i32) -> Request<&'static str> {
        Request {
            item,
            count,
            address: address.to_owned(),
            link_index: 0,
            final_link: true,
            package_counter: 0,
            order_id,
            context: None,
        }
    }

    #[test]
    fn a_request_takes_only_what_it_asks_for() {
        let inventory = vec![slot("iron", 64), slot("gold", 64), slot("iron", 64)];
        let mut requests = vec![request("iron", 70, "Base", 7)];
        let packed = pack(&inventory, Some(&mut requests)).unwrap();
        assert_eq!(packed.taken, vec![(0, 64), (2, 6)]);
        assert_eq!(packed.address.as_deref(), Some("Base"));
        let order = packed.order.unwrap();
        assert_eq!((order.order_id, order.fragment_index, order.is_final, order.is_final_link), (7, 0, true, true));
        // The request is done and gone.
        assert!(requests.is_empty());
    }

    #[test]
    fn requests_of_one_order_share_a_box_and_others_break_it() {
        let inventory = vec![slot("iron", 10), slot("gold", 10), slot("coal", 10)];
        // Same address and order: one box holds both.
        let mut requests = vec![request("iron", 4, "Base", 1), request("gold", 3, "Base", 1)];
        let packed = pack(&inventory, Some(&mut requests)).unwrap();
        assert_eq!(packed.taken, vec![(0, 4), (1, 3)]);
        assert!(requests.is_empty());
        assert!(packed.order.unwrap().is_final);
        // Another order stops the box, and the request stays.
        let mut requests = vec![request("iron", 4, "Base", 1), request("gold", 3, "Base", 2)];
        let packed = pack(&inventory, Some(&mut requests)).unwrap();
        assert_eq!(packed.taken, vec![(0, 4)]);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].item, "gold");
        assert!(packed.order.unwrap().is_final);
        // A request that is only partly served stays, with its count and the next box's index.
        let mut requests = vec![request("iron", 25, "Base", 1)];
        let packed = pack(&[slot("iron", 10)], Some(&mut requests)).unwrap();
        assert_eq!(packed.taken, vec![(0, 10)]);
        assert_eq!(requests[0].count, 15);
        assert_eq!(requests[0].package_counter, 1);
        assert!(!packed.order.unwrap().is_final);
    }

    #[test]
    fn a_request_with_nothing_to_give_is_dropped() {
        let mut requests = vec![request("diamond", 4, "Base", 1)];
        assert_eq!(pack(&[slot("iron", 10)], Some(&mut requests)), None);
        assert!(requests.is_empty());
    }

    fn target(stack: Option<(&'static str, u32)>) -> Target<&'static str> {
        Target { stack, limit: 64 }
    }

    fn max(key: &&str) -> u32 {
        if *key == "egg" { 16 } else { 64 }
    }

    #[test]
    fn unpacking_fits_only_when_every_stack_has_room() {
        let items = [("iron", 64), ("iron", 20), ("gold", 5)];
        // Three empty slots: iron 64, iron 20, gold 5.
        assert!(unpack_fits(&[target(None), target(None), target(None)], &items, &max));
        // Two: the 5 gold has nowhere to go.
        assert!(!unpack_fits(&[target(None), target(None)], &items, &max));
        // A slot holding 44 iron takes the 20.
        assert!(unpack_fits(&[target(None), target(Some(("iron", 44))), target(None)], &items, &max));
        // ... but not when it holds 50.
        assert!(!unpack_fits(&[target(None), target(Some(("iron", 50)))], &items[..2], &max));
        // The rest of it goes on into the next free slot.
        assert!(unpack_fits(&[target(None), target(Some(("iron", 50))), target(None)], &items[..2], &max));
        // Another item in the way.
        assert!(!unpack_fits(&[target(Some(("dirt", 1)))], &[("gold", 1)], &max));
        // Nothing to put.
        assert!(unpack_fits::<&str>(&[], &[], &max));
    }

    #[test]
    fn throwing_gets_harder_the_longer_it_is_held() {
        assert_eq!(throw_velocity(0), 0.0);
        assert!(throw_velocity(2) < 0.1);
        assert!(throw_velocity(10) > 0.1 && throw_velocity(10) < 1.0);
        assert_eq!(throw_velocity(20), 1.0);
        assert_eq!(throw_velocity(200), 1.0);
        assert_eq!(throw_motion([0.0, 0.0, 1.0], 0.05), None);
        assert_eq!(throw_motion([0.0, 0.0, 1.0], 1.0), Some([0.0, 0.0, 2.0]));
    }

    #[test]
    fn a_summary_merges_equal_stacks() {
        let mut summary = Summary::default();
        summary.add(&"iron", 5);
        summary.add(&"gold", 2);
        summary.add(&"iron", 7);
        assert_eq!(summary.stacks, vec![("iron", 12), ("gold", 2)]);
        assert_eq!(summary.count_of(&"iron"), 12);
        summary.add(&"iron", -12);
        assert_eq!(summary.count_of(&"iron"), 0);
    }
}
