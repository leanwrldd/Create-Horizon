//! Filter items: the List Filter and the Attribute Filter (`FilterItem`, `ListFilterItem`,
//! `AttributeFilterItem`, `FilterItemStack`, `ItemAttribute`, `AllItemAttributeTypes`,
//! `AttributeFilterWhitelistMode`). The matching, the attributes and the wording of a filter's
//! summary are here with no game types: the game glue is `machines/filter.rs`, which reads a
//! stack's components into these types and answers the questions an attribute asks of an item
//! ([`Facts`]).
//!
//! Known differences from Create, for the README:
//! - Create edits a filter in a screen. There is none here: the filter is set with clicks
//!   (`machines/filter.rs` lists them), shown on the action bar, and the choice of attributes is
//!   "the next attribute of the reference item that isn't in the list yet".
//! - Attributes left out: `in_item_group` (what each creative tab holds can't be asked of the
//!   game); Create's mod-compat attributes (Astral Sorcery's) don't exist here either.
//! - The attributes about processing (`washable`, `hauntable`, `crushable`, `smeltable`,
//!   `smokable`, `blastable`) read this port's own recipes, so they match what its machines do.
//!   `fluid_container` and `has_fluid` know the four vanilla buckets only.
//! - The Package Filter (`PackageFilterItem`, `PackageFilterItemStack`) is [`Filter::Package`]: it
//!   passes a package whose address matches the filter's (`rules::package::match_address`). Its
//!   address is set with the `/address` command rather than in Create's text screen.

/// `AllDataComponents.FILTER_ITEMS` (a container's contents), `FILTER_ITEMS_BLACKLIST`,
/// `FILTER_ITEMS_RESPECT_NBT`, `ATTRIBUTE_FILTER_WHITELIST_MODE` and
/// `ATTRIBUTE_FILTER_MATCHED_ATTRIBUTES`.
pub const FILTER_ITEMS: &str = "create:filter_items";
pub const FILTER_ITEMS_BLACKLIST: &str = "create:filter_items_blacklist";
pub const FILTER_ITEMS_RESPECT_NBT: &str = "create:filter_items_respect_nbt";
pub const ATTRIBUTE_FILTER_WHITELIST_MODE: &str = "create:attribute_filter_whitelist_mode";
pub const ATTRIBUTE_FILTER_MATCHED_ATTRIBUTES: &str = "create:attribute_filter_matched_attributes";
/// Every component a filter item carries.
pub const COMPONENTS: [&str; 5] = [
    FILTER_ITEMS,
    FILTER_ITEMS_BLACKLIST,
    FILTER_ITEMS_RESPECT_NBT,
    ATTRIBUTE_FILTER_WHITELIST_MODE,
    ATTRIBUTE_FILTER_MATCHED_ATTRIBUTES,
];

/// `ListFilterItem.getFilterItemHandler`: `new ItemStackHandler(18)`.
pub const LIST_SLOTS: usize = 18;

/// `AttributeFilterWhitelistMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Any one of the attributes lets an item pass.
    WhitelistDisj,
    /// Every one of them has to hold.
    WhitelistConj,
    /// An item passes only if it has none.
    Blacklist,
}

impl Mode {
    /// `StringRepresentable.getSerializedName` (`Lang.asId(name())`).
    pub fn name(self) -> &'static str {
        match self {
            Self::WhitelistDisj => "whitelist_disj",
            Self::WhitelistConj => "whitelist_conj",
            Self::Blacklist => "blacklist",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [Self::WhitelistDisj, Self::WhitelistConj, Self::Blacklist]
            .into_iter()
            .find(|mode| mode.name() == name)
    }

    /// The stand-in for the mode buttons: each click moves on to the next.
    pub fn next(self) -> Self {
        match self {
            Self::WhitelistDisj => Self::WhitelistConj,
            Self::WhitelistConj => Self::Blacklist,
            Self::Blacklist => Self::WhitelistDisj,
        }
    }

    /// `AttributeFilterItem.makeSummary`'s first line (`gui.attribute_filter.*`).
    pub fn label(self) -> &'static str {
        match self {
            Self::WhitelistConj => "Allow-List (All)",
            Self::WhitelistDisj => "Allow-List (Any)",
            Self::Blacklist => "Deny-List",
        }
    }
}

/// The attributes with no value (`AllItemAttributeTypes`' `singleton(..)` ones), in Create's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Singleton {
    Placeable,
    Consumable,
    FluidContainer,
    Enchanted,
    MaxEnchanted,
    Renamed,
    Damaged,
    BadlyDamaged,
    NotStackable,
    Equipable,
    FurnaceFuel,
    Washable,
    Hauntable,
    Crushable,
    Smeltable,
    Smokable,
    Blastable,
    Compostable,
}

impl Singleton {
    pub const ALL: [Singleton; 18] = [
        Self::Placeable,
        Self::Consumable,
        Self::FluidContainer,
        Self::Enchanted,
        Self::MaxEnchanted,
        Self::Renamed,
        Self::Damaged,
        Self::BadlyDamaged,
        Self::NotStackable,
        Self::Equipable,
        Self::FurnaceFuel,
        Self::Washable,
        Self::Hauntable,
        Self::Crushable,
        Self::Smeltable,
        Self::Smokable,
        Self::Blastable,
        Self::Compostable,
    ];

    /// The id Create registers it under (`create:<id>`), and its translation key.
    pub fn id(self) -> &'static str {
        match self {
            Self::Placeable => "placeable",
            Self::Consumable => "consumable",
            Self::FluidContainer => "fluid_container",
            Self::Enchanted => "enchanted",
            Self::MaxEnchanted => "max_enchanted",
            Self::Renamed => "renamed",
            Self::Damaged => "damaged",
            Self::BadlyDamaged => "badly_damaged",
            Self::NotStackable => "not_stackable",
            Self::Equipable => "equipable",
            Self::FurnaceFuel => "furnace_fuel",
            Self::Washable => "washable",
            Self::Hauntable => "hauntable",
            Self::Crushable => "crushable",
            Self::Smeltable => "smeltable",
            Self::Smokable => "smokable",
            Self::Blastable => "blastable",
            Self::Compostable => "compostable",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == id)
    }

    /// `create.item_attributes.<id>` and `.inverted` of the language file.
    fn phrase(self, inverted: bool) -> &'static str {
        match (self, inverted) {
            (Self::Placeable, false) => "is placeable",
            (Self::Placeable, true) => "is not placeable",
            (Self::Consumable, false) => "can be eaten",
            (Self::Consumable, true) => "cannot be eaten",
            (Self::FluidContainer, false) => "can store fluids",
            (Self::FluidContainer, true) => "cannot store fluids",
            (Self::Enchanted, false) => "is enchanted",
            (Self::Enchanted, true) => "is unenchanted",
            (Self::MaxEnchanted, false) => "is enchanted at max level",
            (Self::MaxEnchanted, true) => "is not enchanted at max level",
            (Self::Renamed, false) => "has a custom name",
            (Self::Renamed, true) => "does not have a custom name",
            (Self::Damaged, false) => "is damaged",
            (Self::Damaged, true) => "is not damaged",
            (Self::BadlyDamaged, false) => "is heavily damaged",
            (Self::BadlyDamaged, true) => "is not heavily damaged",
            (Self::NotStackable, false) => "cannot stack",
            (Self::NotStackable, true) => "can be stacked",
            (Self::Equipable, false) => "can be equipped",
            (Self::Equipable, true) => "cannot be equipped",
            (Self::FurnaceFuel, false) => "is furnace fuel",
            (Self::FurnaceFuel, true) => "is not furnace fuel",
            (Self::Washable, false) => "can be Washed",
            (Self::Washable, true) => "cannot be Washed",
            (Self::Hauntable, false) => "can be Haunted",
            (Self::Hauntable, true) => "cannot be Haunted",
            (Self::Crushable, false) => "can be Crushed",
            (Self::Crushable, true) => "cannot be Crushed",
            (Self::Smeltable, false) => "can be Smelted",
            (Self::Smeltable, true) => "cannot be Smelted",
            (Self::Smokable, false) => "can be Smoked",
            (Self::Smokable, true) => "cannot be Smoked",
            (Self::Blastable, false) => "can be Smelted in a Blast Furnace",
            (Self::Blastable, true) => "cannot be Smelted in a Blast Furnace",
            (Self::Compostable, false) => "can be composted",
            (Self::Compostable, true) => "cannot be composted",
        }
    }
}

/// `ShulkerFillLevelAttribute.ShulkerLevels`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fill {
    Empty,
    Partial,
    Full,
}

impl Fill {
    pub const ALL: [Fill; 3] = [Self::Empty, Self::Partial, Self::Full];

    pub fn key(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Partial => "partial",
            Self::Full => "full",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|fill| fill.key() == key)
    }

    fn label(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Partial => "partially filled",
            Self::Full => "full",
        }
    }
}

/// One `ItemAttribute`: a question about an item that is true or false.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Attribute {
    Singleton(Singleton),
    /// `InTagAttribute`: the item tag, `minecraft:logs`.
    InTag(String),
    /// `AddedByAttribute`: the namespace of the item's id.
    AddedBy(String),
    /// `EnchantAttribute`: an enchantment's id.
    HasEnchant(String),
    ShulkerFill(Fill),
    /// `ColorAttribute`: a dye colour's name.
    HasColor(String),
    /// `ItemNameAttribute`: the custom name.
    HasName(String),
    BookAuthor(String),
    /// `BookCopyAttribute`: the generation, 0 an original.
    BookCopy(i32),
    /// `FluidContentsAttribute`: a fluid's id.
    HasFluid(String),
}

impl Attribute {
    /// The type id Create registers the attribute under.
    pub fn type_id(&self) -> String {
        match self {
            Self::Singleton(kind) => format!("create:{}", kind.id()),
            Self::InTag(_) => "create:in_tag".to_owned(),
            Self::AddedBy(_) => "create:added_by".to_owned(),
            Self::HasEnchant(_) => "create:has_enchant".to_owned(),
            Self::ShulkerFill(_) => "create:shulker_fill_level".to_owned(),
            Self::HasColor(_) => "create:has_color".to_owned(),
            Self::HasName(_) => "create:has_name".to_owned(),
            Self::BookAuthor(_) => "create:book_author".to_owned(),
            Self::BookCopy(_) => "create:book_copy".to_owned(),
            Self::HasFluid(_) => "create:has_fluid".to_owned(),
        }
    }

    /// The attribute's value as text (none for a singleton).
    pub fn value(&self) -> Option<String> {
        match self {
            Self::Singleton(_) => None,
            Self::InTag(v)
            | Self::AddedBy(v)
            | Self::HasEnchant(v)
            | Self::HasColor(v)
            | Self::HasName(v)
            | Self::BookAuthor(v)
            | Self::HasFluid(v) => Some(v.clone()),
            Self::ShulkerFill(fill) => Some(fill.key().to_owned()),
            Self::BookCopy(generation) => Some(generation.to_string()),
        }
    }

    /// The inverse of [`Attribute::type_id`] and [`Attribute::value`].
    pub fn parse(type_id: &str, value: Option<&str>) -> Option<Self> {
        let path = type_id.strip_prefix("create:")?;
        if let Some(kind) = Singleton::from_id(path) {
            return Some(Self::Singleton(kind));
        }
        let value = value?;
        Some(match path {
            "in_tag" => Self::InTag(value.to_owned()),
            "added_by" => Self::AddedBy(value.to_owned()),
            "has_enchant" => Self::HasEnchant(value.to_owned()),
            "shulker_fill_level" => Self::ShulkerFill(Fill::from_key(value)?),
            "has_color" => Self::HasColor(value.to_owned()),
            "has_name" => Self::HasName(value.to_owned()),
            "book_author" => Self::BookAuthor(value.to_owned()),
            "book_copy" => Self::BookCopy(value.parse().ok()?),
            "has_fluid" => Self::HasFluid(value.to_owned()),
            _ => return None,
        })
    }

    /// `ItemAttribute.format`: the attribute as a sentence (`create.item_attributes.*`).
    pub fn describe(&self, inverted: bool) -> String {
        let not = if inverted { "not " } else { "" };
        match self {
            Self::Singleton(kind) => kind.phrase(inverted).to_owned(),
            Self::InTag(tag) => format!("is {not}tagged #{tag}"),
            Self::AddedBy(id) => format!("was {not}added by {}", capitalize(id)),
            Self::HasEnchant(id) => format!("is {not}enchanted with {}", pretty(id)),
            Self::ShulkerFill(fill) => {
                if inverted {
                    format!("is shulker not {}", fill.label())
                } else {
                    format!("is shulker {}", fill.label())
                }
            }
            Self::HasColor(color) => format!("is {not}dyed {color}"),
            Self::HasName(name) => {
                if inverted {
                    format!("does not have the custom name {name}")
                } else {
                    format!("has the custom name {name}")
                }
            }
            Self::BookAuthor(author) => format!("was {not}authored by {author}"),
            Self::BookCopy(generation) => {
                let what = match generation {
                    0 => "an original",
                    1 => "a first-generation copy",
                    2 => "a second-generation copy",
                    _ => "a tattered mess",
                };
                format!("is {not}{what}")
            }
            Self::HasFluid(id) => {
                if inverted {
                    format!("does not contain {}", pretty(id))
                } else {
                    format!("contains {}", pretty(id))
                }
            }
        }
    }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

/// An item's or an enchantment's name for a line of text: `minecraft:fire_aspect` is `Fire Aspect`.
pub fn display_name(id: &str) -> String {
    pretty(id)
}

fn pretty(id: &str) -> String {
    let path = id.rsplit(':').next().unwrap_or(id);
    path.split('_').map(capitalize).collect::<Vec<_>>().join(" ")
}

/// What an attribute asks of an item. The glue answers from the stack and the game.
pub trait Facts {
    fn singleton(&self, kind: Singleton) -> bool;
    /// `ItemStack.is(TagKey)`.
    fn in_tag(&self, tag: &str) -> bool;
    /// The tags of the item among the ones Create-Horizon knows of ([`crate::rules::filter_items::ITEM_TAGS`]).
    fn tags(&self) -> Vec<String>;
    /// `Item.getCreatorModId`: the namespace of the item's id.
    fn namespace(&self) -> String;
    /// The ids of the enchantments on the item (`EnchantmentHelper.getEnchantmentsForCrafting`).
    fn enchantments(&self) -> Vec<String>;
    /// The shulker box fill level, or none for an item that isn't a shulker box.
    fn shulker_fill(&self) -> Option<Fill>;
    /// The dye colours the item has (`ColorAttribute.findMatchingDyeColors`).
    fn colours(&self) -> Vec<String>;
    fn custom_name(&self) -> Option<String>;
    fn book_author(&self) -> Option<String>;
    fn book_generation(&self) -> Option<i32>;
    fn fluids(&self) -> Vec<String>;
}

/// `ItemAttribute.appliesTo`.
pub fn applies(attribute: &Attribute, facts: &dyn Facts) -> bool {
    match attribute {
        Attribute::Singleton(kind) => facts.singleton(*kind),
        Attribute::InTag(tag) => facts.in_tag(tag),
        Attribute::AddedBy(id) => &facts.namespace() == id,
        Attribute::HasEnchant(id) => facts.enchantments().contains(id),
        Attribute::ShulkerFill(fill) => facts.shulker_fill() == Some(*fill),
        Attribute::HasColor(colour) => facts.colours().contains(colour),
        Attribute::HasName(name) => facts.custom_name().as_ref() == Some(name),
        Attribute::BookAuthor(author) => facts.book_author().as_ref() == Some(author),
        Attribute::BookCopy(generation) => facts.book_generation() == Some(*generation),
        Attribute::HasFluid(fluid) => facts.fluids().contains(fluid),
    }
}

/// `ItemAttribute.getAllAttributes`: every attribute the item has, in Create's order (the
/// singletons, then tag, group, creator, enchantments, shulker level, colours, fluid, name, book).
pub fn all_attributes(facts: &dyn Facts) -> Vec<Attribute> {
    let mut found: Vec<Attribute> = Singleton::ALL
        .into_iter()
        .filter(|kind| facts.singleton(*kind))
        .map(Attribute::Singleton)
        .collect();
    found.extend(facts.tags().into_iter().map(Attribute::InTag));
    let namespace = facts.namespace();
    if !namespace.is_empty() {
        found.push(Attribute::AddedBy(namespace));
    }
    found.extend(facts.enchantments().into_iter().map(Attribute::HasEnchant));
    if let Some(fill) = facts.shulker_fill() {
        found.push(Attribute::ShulkerFill(fill));
    }
    found.extend(facts.colours().into_iter().map(Attribute::HasColor));
    found.extend(facts.fluids().into_iter().map(Attribute::HasFluid));
    if let Some(name) = facts.custom_name().filter(|name| !name.is_empty()) {
        found.push(Attribute::HasName(name));
    }
    if let Some(author) = facts.book_author().filter(|author| !author.is_empty()) {
        found.push(Attribute::BookAuthor(author));
    }
    if let Some(generation) = facts.book_generation().filter(|g| *g >= 0) {
        found.push(Attribute::BookCopy(generation));
    }
    found
}

/// `ShulkerFillLevelAttribute.ShulkerLevels.canApply` for a box with `stacks` non-empty slots out
/// of 27, `full` when every slot holds a full stack.
pub fn shulker_fill(stacks: usize, full: bool) -> Fill {
    if full {
        Fill::Full
    } else if stacks == 0 {
        Fill::Empty
    } else {
        Fill::Partial
    }
}

/// A filter, as `FilterItemStack.of` makes it from a stack. `S` is the game's item stack.
#[derive(Clone, Debug, PartialEq)]
pub enum Filter<S> {
    /// An ordinary item (or a filter item with nothing set): passes the same item.
    Item(S),
    /// `ListFilterItemStack`.
    List(ListFilter<S>),
    /// `AttributeFilterItemStack`.
    Attribute(AttributeFilter),
    /// `PackageFilterItemStack`: the address a package has to match.
    Package(String),
}

/// `ListFilterItemStack`.
#[derive(Clone, Debug, PartialEq)]
pub struct ListFilter<S> {
    pub items: Vec<Filter<S>>,
    pub respect_nbt: bool,
    pub blacklist: bool,
}

/// `AttributeFilterItemStack`: the attributes with whether each is inverted.
#[derive(Clone, Debug, PartialEq)]
pub struct AttributeFilter {
    pub mode: Mode,
    pub tests: Vec<(Attribute, bool)>,
}

/// What matching needs of the game.
pub trait Env<S> {
    /// `ItemHelper.sameItem`: the same item, components ignored.
    fn same_item(&self, filter: &S, stack: &S) -> bool;
    /// `ItemStack.isSameItemSameComponents`.
    fn same_components(&self, filter: &S, stack: &S) -> bool;
    /// Whether the stack has the attribute (`ItemAttribute.appliesTo`).
    fn has(&self, attribute: &Attribute, stack: &S) -> bool;
    /// `PackageItem.getAddress` of a stack that is a package (`PackageItem.isPackage`), else none.
    fn package_address(&self, _stack: &S) -> Option<String> {
        None
    }
}

impl<S> Filter<S> {
    /// `FilterItemStack.test(world, stack, matchNBT)`. An empty filter is not represented here:
    /// the caller passes anything for "no filter".
    pub fn test(&self, env: &dyn Env<S>, stack: &S, match_nbt: bool) -> bool {
        match self {
            // `FilterItem.testDirect`.
            Self::Item(filter) => {
                if match_nbt {
                    env.same_components(filter, stack)
                } else {
                    env.same_item(filter, stack)
                }
            }
            Self::List(list) => {
                for item in &list.items {
                    if item.test(env, stack, list.respect_nbt) {
                        return !list.blacklist;
                    }
                }
                list.blacklist
            }
            Self::Attribute(filter) => filter.test(env, stack),
            // `PackageFilterItemStack.test`: `isPackage(stack) && matchAddress(stack, filterString)`.
            Self::Package(address) => env
                .package_address(stack)
                .is_some_and(|boxed| super::package::match_address(&boxed, address)),
        }
    }
}

impl AttributeFilter {
    /// `AttributeFilterItemStack.test`.
    pub fn test<S>(&self, env: &dyn Env<S>, stack: &S) -> bool {
        for (attribute, inverted) in &self.tests {
            let matches = env.has(attribute, stack) != *inverted;
            if matches {
                match self.mode {
                    Mode::Blacklist => return false,
                    Mode::WhitelistConj => continue,
                    Mode::WhitelistDisj => return true,
                }
            } else if self.mode == Mode::WhitelistConj {
                return false;
            }
        }
        match self.mode {
            Mode::Blacklist | Mode::WhitelistConj => true,
            Mode::WhitelistDisj => false,
        }
    }
}

// --- the stand-in for the screens ---------------------------------------------------------------

/// What a click with a reference item does to a List Filter's items.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    Added,
    Removed,
    /// All 18 slots are in use.
    Full,
}

/// Adds `item` to the list, or takes it out if the list holds it already (`same` says whether two
/// entries are the same).
pub fn toggle_item<S>(items: &mut Vec<S>, item: S, same: impl Fn(&S, &S) -> bool) -> Toggle {
    if let Some(at) = items.iter().position(|have| same(have, &item)) {
        items.remove(at);
        return Toggle::Removed;
    }
    if items.len() >= LIST_SLOTS {
        return Toggle::Full;
    }
    items.push(item);
    Toggle::Added
}

/// The List Filter's mode buttons as one cycle: allow, allow and respect data, deny, deny and
/// respect data. Answers the new `(blacklist, respect_nbt)`.
pub fn next_list_mode(blacklist: bool, respect_nbt: bool) -> (bool, bool) {
    match (blacklist, respect_nbt) {
        (false, false) => (false, true),
        (false, true) => (true, false),
        (true, false) => (true, true),
        (true, true) => (false, false),
    }
}

/// The attribute to add for a reference item: the first of its attributes that the filter doesn't
/// hold yet with the same `inverted` (the screen shows them all and lets a player pick).
pub fn next_attribute(
    candidates: &[Attribute],
    selected: &[(Attribute, bool)],
    inverted: bool,
) -> Option<Attribute> {
    candidates
        .iter()
        .find(|candidate| !selected.iter().any(|(have, flag)| have == *candidate && *flag == inverted))
        .cloned()
}

/// `ListFilterItem.makeSummary`: the lines under the item's name. `names` are the display names of
/// the items in the list; empty when the list is.
pub fn list_summary(blacklist: bool, names: &[String]) -> Vec<String> {
    if names.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![(if blacklist { "Deny-List" } else { "Allow-List" }).to_owned()];
    for (count, name) in names.iter().enumerate() {
        if count > 3 {
            lines.push("- ...".to_owned());
            break;
        }
        lines.push(format!("- {name}"));
    }
    lines
}

/// `AttributeFilterItem.makeSummary`.
pub fn attribute_summary(filter: &AttributeFilter) -> Vec<String> {
    if filter.tests.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![filter.mode.label().to_owned()];
    for (count, (attribute, inverted)) in filter.tests.iter().enumerate() {
        if count > 3 {
            lines.push("- ...".to_owned());
            break;
        }
        lines.push(format!("- {}", attribute.describe(*inverted)));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stack in a test: an item, a component value and some facts.
    #[derive(Clone, Debug, PartialEq)]
    struct S {
        item: &'static str,
        data: u32,
        attributes: Vec<Attribute>,
    }

    fn stack(item: &'static str, data: u32) -> S {
        S { item, data, attributes: Vec::new() }
    }

    struct World;

    impl Env<S> for World {
        fn same_item(&self, filter: &S, stack: &S) -> bool {
            filter.item == stack.item
        }
        fn same_components(&self, filter: &S, stack: &S) -> bool {
            filter.item == stack.item && filter.data == stack.data
        }
        fn has(&self, attribute: &Attribute, stack: &S) -> bool {
            stack.attributes.contains(attribute)
        }
    }

    fn list(items: Vec<Filter<S>>, respect_nbt: bool, blacklist: bool) -> Filter<S> {
        Filter::List(ListFilter { items, respect_nbt, blacklist })
    }

    #[test]
    fn an_allow_list_passes_what_it_holds_and_an_empty_one_nothing() {
        let filter = list(vec![Filter::Item(stack("a", 0)), Filter::Item(stack("b", 0))], false, false);
        assert!(filter.test(&World, &stack("a", 5), false));
        assert!(filter.test(&World, &stack("b", 0), false));
        assert!(!filter.test(&World, &stack("c", 0), false));
        // `ListFilterItem`'s description: an empty Allow-List rejects everything.
        assert!(!list(vec![], false, false).test(&World, &stack("a", 0), false));
    }

    #[test]
    fn a_deny_list_passes_what_it_does_not_hold_and_an_empty_one_everything() {
        let filter = list(vec![Filter::Item(stack("a", 0))], false, true);
        assert!(!filter.test(&World, &stack("a", 0), false));
        assert!(filter.test(&World, &stack("c", 0), false));
        assert!(list(vec![], false, true).test(&World, &stack("a", 0), false));
    }

    #[test]
    fn respect_data_compares_components_and_the_callers_flag_is_ignored() {
        let respect = list(vec![Filter::Item(stack("a", 1))], true, false);
        assert!(respect.test(&World, &stack("a", 1), false));
        assert!(!respect.test(&World, &stack("a", 2), false));
        // A list uses its own setting, whatever the caller asks.
        let ignore = list(vec![Filter::Item(stack("a", 1))], false, false);
        assert!(ignore.test(&World, &stack("a", 2), true));
    }

    #[test]
    fn a_filter_inside_a_list_filters_by_its_own_rules() {
        let inner = Filter::Attribute(AttributeFilter {
            mode: Mode::WhitelistDisj,
            tests: vec![(Attribute::Singleton(Singleton::Placeable), false)],
        });
        let outer = list(vec![inner], false, false);
        let mut block = stack("stone", 0);
        block.attributes.push(Attribute::Singleton(Singleton::Placeable));
        assert!(outer.test(&World, &block, false));
        assert!(!outer.test(&World, &stack("stick", 0), false));
    }

    fn with(attributes: &[Attribute]) -> S {
        S { item: "x", data: 0, attributes: attributes.to_vec() }
    }

    fn test(mode: Mode, tests: &[(Attribute, bool)], stack: &S) -> bool {
        AttributeFilter { mode, tests: tests.to_vec() }.test(&World, stack)
    }

    #[test]
    fn attribute_modes_follow_create() {
        let food = Attribute::Singleton(Singleton::Consumable);
        let block = Attribute::Singleton(Singleton::Placeable);
        let tests = [(food.clone(), false), (block.clone(), false)];
        let both = with(&[food.clone(), block.clone()]);
        let one = with(&[food.clone()]);
        let none = with(&[]);
        // Any.
        assert!(test(Mode::WhitelistDisj, &tests, &both));
        assert!(test(Mode::WhitelistDisj, &tests, &one));
        assert!(!test(Mode::WhitelistDisj, &tests, &none));
        // All.
        assert!(test(Mode::WhitelistConj, &tests, &both));
        assert!(!test(Mode::WhitelistConj, &tests, &one));
        // Deny.
        assert!(!test(Mode::Blacklist, &tests, &one));
        assert!(test(Mode::Blacklist, &tests, &none));
    }

    #[test]
    fn empty_attribute_filters_behave_as_create_s() {
        // The loop never runs: a deny list and an all-list pass, an any-list rejects.
        assert!(test(Mode::Blacklist, &[], &with(&[])));
        assert!(test(Mode::WhitelistConj, &[], &with(&[])));
        assert!(!test(Mode::WhitelistDisj, &[], &with(&[])));
    }

    #[test]
    fn an_inverted_attribute_matches_items_without_it() {
        let food = Attribute::Singleton(Singleton::Consumable);
        let tests = [(food.clone(), true)];
        assert!(test(Mode::WhitelistDisj, &tests, &with(&[])));
        assert!(!test(Mode::WhitelistDisj, &tests, &with(&[food])));
    }

    #[test]
    fn attributes_round_trip_through_their_ids() {
        let all = [
            Attribute::Singleton(Singleton::Washable),
            Attribute::InTag("minecraft:logs".into()),
            Attribute::AddedBy("create".into()),
            Attribute::HasEnchant("minecraft:sharpness".into()),
            Attribute::ShulkerFill(Fill::Partial),
            Attribute::HasColor("red".into()),
            Attribute::HasName("Bob".into()),
            Attribute::BookAuthor("Alice".into()),
            Attribute::BookCopy(2),
            Attribute::HasFluid("minecraft:water".into()),
        ];
        for attribute in all {
            let again = Attribute::parse(&attribute.type_id(), attribute.value().as_deref());
            assert_eq!(again, Some(attribute));
        }
        assert_eq!(Attribute::parse("create:nonsense", None), None);
        assert_eq!(Attribute::parse("create:in_tag", None), None);
        for kind in Singleton::ALL {
            assert_eq!(Singleton::from_id(kind.id()), Some(kind));
        }
        for mode in [Mode::WhitelistDisj, Mode::WhitelistConj, Mode::Blacklist] {
            assert_eq!(Mode::from_name(mode.name()), Some(mode));
        }
    }

    #[test]
    fn attributes_read_as_create_s_sentences() {
        assert_eq!(Attribute::Singleton(Singleton::Consumable).describe(false), "can be eaten");
        assert_eq!(Attribute::Singleton(Singleton::Consumable).describe(true), "cannot be eaten");
        assert_eq!(Attribute::InTag("minecraft:logs".into()).describe(false), "is tagged #minecraft:logs");
        assert_eq!(Attribute::InTag("minecraft:logs".into()).describe(true), "is not tagged #minecraft:logs");
        assert_eq!(Attribute::HasEnchant("minecraft:fire_aspect".into()).describe(false), "is enchanted with Fire Aspect");
        assert_eq!(Attribute::ShulkerFill(Fill::Partial).describe(false), "is shulker partially filled");
        assert_eq!(Attribute::BookCopy(3).describe(false), "is a tattered mess");
        assert_eq!(Attribute::AddedBy("create".into()).describe(true), "was not added by Create");
    }

    #[test]
    fn the_list_toggles_an_item_and_holds_eighteen() {
        let mut items = Vec::new();
        let same = |a: &u32, b: &u32| a == b;
        for n in 0..18 {
            assert_eq!(toggle_item(&mut items, n, same), Toggle::Added);
        }
        assert_eq!(toggle_item(&mut items, 99, same), Toggle::Full);
        assert_eq!(toggle_item(&mut items, 3, same), Toggle::Removed);
        assert_eq!(items.len(), 17);
        assert_eq!(toggle_item(&mut items, 99, same), Toggle::Added);
    }

    #[test]
    fn the_modes_cycle() {
        let mut state = (false, false);
        let mut seen = Vec::new();
        for _ in 0..4 {
            state = next_list_mode(state.0, state.1);
            seen.push(state);
        }
        assert_eq!(seen, [(false, true), (true, false), (true, true), (false, false)]);
        assert_eq!(Mode::WhitelistDisj.next().next().next(), Mode::WhitelistDisj);
    }

    #[test]
    fn the_next_attribute_skips_the_ones_the_filter_holds() {
        let a = Attribute::Singleton(Singleton::Placeable);
        let b = Attribute::Singleton(Singleton::Damaged);
        let candidates = [a.clone(), b.clone()];
        assert_eq!(next_attribute(&candidates, &[], false), Some(a.clone()));
        assert_eq!(next_attribute(&candidates, &[(a.clone(), false)], false), Some(b.clone()));
        // The same attribute may be added once inverted.
        assert_eq!(next_attribute(&candidates, &[(a.clone(), false)], true), Some(a.clone()));
        assert_eq!(next_attribute(&candidates, &[(a, false), (b, false)], false), None);
    }

    #[test]
    fn the_summaries_list_four_lines_and_then_dots() {
        let names: Vec<String> = (1..=6).map(|n| format!("Item {n}")).collect();
        let lines = list_summary(false, &names);
        assert_eq!(lines[0], "Allow-List");
        assert_eq!(lines.len(), 6);
        assert_eq!(lines[5], "- ...");
        assert!(list_summary(true, &[]).is_empty());
        assert_eq!(list_summary(true, &names[..1]), ["Deny-List", "- Item 1"]);
        let filter = AttributeFilter {
            mode: Mode::WhitelistConj,
            tests: vec![(Attribute::Singleton(Singleton::Placeable), false)],
        };
        assert_eq!(attribute_summary(&filter), ["Allow-List (All)", "- is placeable"]);
    }

    #[test]
    fn the_shulker_level_and_the_attribute_order() {
        assert_eq!(shulker_fill(0, false), Fill::Empty);
        assert_eq!(shulker_fill(5, false), Fill::Partial);
        assert_eq!(shulker_fill(27, true), Fill::Full);

        struct Apple;
        impl Facts for Apple {
            fn singleton(&self, kind: Singleton) -> bool {
                matches!(kind, Singleton::Consumable | Singleton::Compostable)
            }
            fn in_tag(&self, _: &str) -> bool {
                false
            }
            fn tags(&self) -> Vec<String> {
                vec!["minecraft:fruits".to_owned()]
            }
            fn namespace(&self) -> String {
                "minecraft".to_owned()
            }
            fn enchantments(&self) -> Vec<String> {
                Vec::new()
            }
            fn shulker_fill(&self) -> Option<Fill> {
                None
            }
            fn colours(&self) -> Vec<String> {
                Vec::new()
            }
            fn custom_name(&self) -> Option<String> {
                None
            }
            fn book_author(&self) -> Option<String> {
                None
            }
            fn book_generation(&self) -> Option<i32> {
                None
            }
            fn fluids(&self) -> Vec<String> {
                Vec::new()
            }
        }
        assert_eq!(
            all_attributes(&Apple),
            [
                Attribute::Singleton(Singleton::Consumable),
                Attribute::Singleton(Singleton::Compostable),
                Attribute::InTag("minecraft:fruits".into()),
                Attribute::AddedBy("minecraft".into()),
            ]
        );
        assert!(applies(&Attribute::AddedBy("minecraft".into()), &Apple));
        assert!(!applies(&Attribute::AddedBy("create".into()), &Apple));
    }
}
