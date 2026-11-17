//! Create's potion fluid (`AllFluids.POTION`, `PotionFluid`, `PotionFluidHandler`): potions that a bottle gives an Item
//! Drain and a Spout gives a bottle, and the `create:filling` recipes that take a potion (`filling/glowstone`,
//! `filling/gunpowder`, `filling/redstone`).
//!
//! In Create the potion fluid is one fluid whose stack carries the data components `minecraft:potion_contents` and
//! `create:potion_fluid_bottle_type`. A tank of Horizon's holds a fluid id and an amount, no components, so the
//! potion and the bottle type go in the fluid id: `create:potion[bottle=regular,potion=minecraft:night_vision]`
//! (`potion=` left out for `PotionContents.EMPTY`). Water in a regular bottle is plain `minecraft:water`, as in
//! `PotionFluidHandler.getFluidFromPotion`.
//!
//! Known differences from Create, for the README:
//! - only a potion's id is carried (`PotionContents.potion`): a potion with custom effects, a custom colour or a custom
//!   name (`custom_effects`, `custom_color`, `custom_name`) can't be drained, as a bottle that holds nothing else is;
//! - the fluid is drawn as Horizon's water tinted with the potion's colour, and named by the potion
//!   (`item.minecraft.potion.effect.<name>`), not Create's own fluid texture and tooltip with the effects listed.

/// `AllFluids.POTION`.
pub const POTION_FLUID: &str = "create:potion";
/// `AllDataComponents.POTION_FLUID_BOTTLE_TYPE`.
pub const BOTTLE_TYPE_COMPONENT: &str = "create:potion_fluid_bottle_type";
pub const POTION_CONTENTS: &str = "minecraft:potion_contents";
/// `PotionFluidHandler.getFluidFromPotionItem`'s amount and `getRequiredAmountForFilledBottle`: a bottle's worth.
pub const BOTTLE_AMOUNT: i32 = 250;

const WATER_FLUID: &str = "minecraft:water";
const WATER_POTION: &str = "minecraft:water";
const GLASS_BOTTLE: &str = "minecraft:glass_bottle";

/// `PotionFluid.BottleType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bottle {
    Regular,
    Splash,
    Lingering,
}

impl Bottle {
    /// `getSerializedName` (`Lang.asId(name())`).
    pub fn name(self) -> &'static str {
        match self {
            Bottle::Regular => "regular",
            Bottle::Splash => "splash",
            Bottle::Lingering => "lingering",
        }
    }

    pub fn from_name(name: &str) -> Option<Bottle> {
        match name {
            "regular" => Some(Bottle::Regular),
            "splash" => Some(Bottle::Splash),
            "lingering" => Some(Bottle::Lingering),
            _ => None,
        }
    }

    /// `PotionFluidHandler.itemFromBottleType`.
    pub fn item(self) -> &'static str {
        match self {
            Bottle::Regular => "minecraft:potion",
            Bottle::Splash => "minecraft:splash_potion",
            Bottle::Lingering => "minecraft:lingering_potion",
        }
    }

    /// `PotionFluidHandler.bottleTypeFromItem` for the potion items; none for any other item.
    pub fn of_item(item: &str) -> Option<Bottle> {
        match item {
            "minecraft:potion" => Some(Bottle::Regular),
            "minecraft:splash_potion" => Some(Bottle::Splash),
            "minecraft:lingering_potion" => Some(Bottle::Lingering),
            _ => None,
        }
    }
}

/// A potion fluid: the bottle it comes from and the potion in it (`None` for `PotionContents.EMPTY`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Potion {
    pub bottle: Bottle,
    pub potion: Option<String>,
}

/// The fluid id of a potion fluid.
pub fn fluid_id(bottle: Bottle, potion: Option<&str>) -> String {
    match potion {
        Some(potion) => format!("{POTION_FLUID}[bottle={},potion={potion}]", bottle.name()),
        None => format!("{POTION_FLUID}[bottle={}]", bottle.name()),
    }
}

/// The potion in a fluid id; `None` for any fluid that isn't a potion fluid.
pub fn parse(fluid: &str) -> Option<Potion> {
    let rest = fluid.strip_prefix(POTION_FLUID)?;
    if rest.is_empty() {
        // The bare fluid: a regular bottle of nothing.
        return Some(Potion { bottle: Bottle::Regular, potion: None });
    }
    let inner = rest.strip_prefix('[')?.strip_suffix(']')?;
    let mut bottle = Bottle::Regular;
    let mut potion = None;
    for part in inner.split(',') {
        match part.split_once('=')? {
            ("bottle", name) => bottle = Bottle::from_name(name)?,
            ("potion", id) => potion = Some(id.to_owned()),
            _ => return None,
        }
    }
    Some(Potion { bottle, potion })
}

pub fn is_potion_fluid(fluid: &str) -> bool {
    parse(fluid).is_some()
}

/// `PotionFluidHandler.getFluidFromPotion`: the fluid a potion makes (water for a regular bottle of water).
pub fn from_potion(potion: Option<&str>, bottle: Bottle) -> String {
    if potion == Some(WATER_POTION) && bottle == Bottle::Regular {
        return WATER_FLUID.to_owned();
    }
    fluid_id(bottle, potion)
}

/// `PotionFluidHandler.getFluidFromPotionItem` and `emptyPotion`: what a potion item holds, as a fluid and its amount
/// (250), and what is left of it (a glass bottle). `None` for an item that isn't a potion.
pub fn from_item(item: &str, potion: Option<&str>) -> Option<(String, i32)> {
    let bottle = Bottle::of_item(item)?;
    Some((from_potion(potion, bottle), BOTTLE_AMOUNT))
}

/// `PotionFluidHandler.emptyPotion`'s leftover.
pub const EMPTIED_ITEM: &str = GLASS_BOTTLE;

/// `GenericItemFilling.canFillGlassBottleInternally`: the fluids an empty bottle takes in itself.
pub fn fills_a_glass_bottle(fluid: &str) -> bool {
    fluid == WATER_FLUID || fluid == "create:tea" || is_potion_fluid(fluid)
}

/// `PotionFluidHandler.fillBottle`: the item a bottle filled from a potion fluid becomes and its
/// `minecraft:potion_contents` (`None` for empty contents).
pub fn filled_bottle(fluid: &str) -> Option<(&'static str, Option<String>)> {
    let potion = parse(fluid)?;
    Some((potion.bottle.item(), potion.potion))
}

/// `FluidIngredient.test` of a recipe's `neoforge:components` fluid ingredient (`DataComponentFluidIngredient`, not
/// strict): the fluid must be the potion fluid and carry the components the recipe names. `bottle` is its
/// `create:potion_fluid_bottle_type` and `potion` the `potion` of its `minecraft:potion_contents`; one the recipe
/// doesn't name is not asked.
pub fn ingredient_matches(fluid: &str, ingredient_fluid: &str, bottle: Option<&str>, potion: Option<&str>) -> bool {
    if ingredient_fluid != POTION_FLUID {
        return false;
    }
    let Some(have) = parse(fluid) else {
        return false;
    };
    if let Some(bottle) = bottle
        && have.bottle.name() != bottle
    {
        return false;
    }
    if let Some(potion) = potion
        && have.potion.as_deref() != Some(potion)
    {
        return false;
    }
    true
}

/// The language keys of a potion fluid's name (`Potion.getName(potion, item.getDescriptionId() + ".effect.")`):
/// `item.minecraft.potion.effect.night_vision`. The potion's effect name drops its `long_` and `strong_` prefix, as
/// vanilla's `Potion.name` does (`long_night_vision` is named `night_vision`).
pub fn name_key(potion: &Potion) -> String {
    let item = potion.bottle.item();
    let (namespace, path) = item.split_once(':').unwrap_or(("minecraft", item));
    let id = potion.potion.as_deref().unwrap_or("empty");
    let id = id.split_once(':').map_or(id, |(_, path)| path);
    let name = id.strip_prefix("long_").or_else(|| id.strip_prefix("strong_")).unwrap_or(id);
    format!("item.{namespace}.{path}.effect.{name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_potion_fluid_id_round_trips() {
        let id = fluid_id(Bottle::Splash, Some("minecraft:night_vision"));
        assert_eq!(id, "create:potion[bottle=splash,potion=minecraft:night_vision]");
        assert_eq!(parse(&id), Some(Potion { bottle: Bottle::Splash, potion: Some("minecraft:night_vision".into()) }));
        let empty = fluid_id(Bottle::Lingering, None);
        assert_eq!(parse(&empty), Some(Potion { bottle: Bottle::Lingering, potion: None }));
        assert_eq!(parse("create:potion"), Some(Potion { bottle: Bottle::Regular, potion: None }));
    }

    #[test]
    fn other_fluids_are_no_potions() {
        for fluid in ["minecraft:water", "create:honey", "create:potions", "create:potion[bottle=huge]", "create:potion[x=1]"] {
            assert_eq!(parse(fluid), None, "{fluid}");
            assert!(!is_potion_fluid(fluid));
        }
    }

    #[test]
    fn a_regular_bottle_of_water_is_plain_water() {
        assert_eq!(from_potion(Some("minecraft:water"), Bottle::Regular), "minecraft:water");
        // A splash bottle of water stays a potion fluid.
        assert_eq!(
            from_potion(Some("minecraft:water"), Bottle::Splash),
            "create:potion[bottle=splash,potion=minecraft:water]"
        );
    }

    #[test]
    fn a_potion_item_holds_a_bottles_worth() {
        assert_eq!(
            from_item("minecraft:potion", Some("minecraft:harming")),
            Some(("create:potion[bottle=regular,potion=minecraft:harming]".to_owned(), 250))
        );
        assert_eq!(
            from_item("minecraft:lingering_potion", Some("minecraft:strength")),
            Some(("create:potion[bottle=lingering,potion=minecraft:strength]".to_owned(), 250))
        );
        assert_eq!(from_item("minecraft:potion", Some("minecraft:water")), Some(("minecraft:water".to_owned(), 250)));
        assert_eq!(from_item("minecraft:stick", None), None);
    }

    #[test]
    fn an_empty_bottle_takes_water_tea_and_potions() {
        assert!(fills_a_glass_bottle("minecraft:water"));
        assert!(fills_a_glass_bottle("create:tea"));
        assert!(fills_a_glass_bottle("create:potion[bottle=regular,potion=minecraft:luck]"));
        assert!(!fills_a_glass_bottle("create:honey"));
        assert!(!fills_a_glass_bottle("minecraft:lava"));
    }

    #[test]
    fn a_filled_bottle_is_the_item_of_its_type() {
        assert_eq!(
            filled_bottle("create:potion[bottle=splash,potion=minecraft:poison]"),
            Some(("minecraft:splash_potion", Some("minecraft:poison".to_owned())))
        );
        assert_eq!(filled_bottle("minecraft:water"), None);
    }

    #[test]
    fn a_components_ingredient_asks_the_bottle_and_the_potion() {
        let fluid = "create:potion[bottle=regular,potion=minecraft:night_vision]";
        assert!(ingredient_matches(fluid, "create:potion", Some("regular"), Some("minecraft:night_vision")));
        assert!(!ingredient_matches(fluid, "create:potion", Some("regular"), Some("minecraft:strength")));
        assert!(!ingredient_matches(fluid, "create:potion", Some("splash"), Some("minecraft:night_vision")));
        assert!(ingredient_matches(fluid, "create:potion", None, None));
        assert!(!ingredient_matches(fluid, "minecraft:water", None, None));
        assert!(!ingredient_matches("minecraft:water", "create:potion", Some("regular"), None));
        // A long potion is a different potion.
        assert!(!ingredient_matches(
            "create:potion[bottle=regular,potion=minecraft:long_night_vision]",
            "create:potion",
            Some("regular"),
            Some("minecraft:night_vision")
        ));
    }

    #[test]
    fn the_name_follows_the_bottle_and_drops_the_length() {
        let regular = parse("create:potion[bottle=regular,potion=minecraft:long_night_vision]").unwrap();
        assert_eq!(name_key(&regular), "item.minecraft.potion.effect.night_vision");
        let splash = parse("create:potion[bottle=splash,potion=minecraft:strong_healing]").unwrap();
        assert_eq!(name_key(&splash), "item.minecraft.splash_potion.effect.healing");
        assert_eq!(name_key(&parse("create:potion").unwrap()), "item.minecraft.potion.effect.empty");
    }
}
