//! Brewing in a Mechanical Mixer + Basin: Create's `PotionMixingRecipes` (`content/fluids/potion/`), which turns the
//! game's brewing mixes into `create:mixing` recipes, and `MechanicalMixerBlockEntity.getMatchingRecipes`, which adds
//! them to what the mixer looks at (config `allowBrewingInMixer`, default on). No game types: the glue reads the
//! brewing recipes through `Host::recipes_of_type("minecraft:brewing")` and asks these what they make.
//!
//! In Create 1.21.1 a mix comes from `PotionBrewing`'s `potionMixes` (a potion + an ingredient item gives another
//! potion) and `containerMixes` (a container item + an ingredient item gives another container item, keeping the
//! potion). Horizon's version holds the same data as `data/minecraft/recipe/brewing/*.json` (`input`, `reagent`,
//! `output`): a recipe with a potion filter on its input and a potion in its output is a potion mix, one with neither
//! (item to item) is a container mix. The recipes become:
//! - for every container (a regular, a splash and a lingering potion) and every potion mix, in the order of the
//!   mixes: a potion fluid of 1000 mB + the ingredient item, with heat (`HeatCondition.HEATED`), gives the other
//!   potion fluid of 1000 mB, in the same bottle type (`potion_mixing_vanilla_<n>`);
//! - then for every container mix and every potion in the potion registry: that potion's fluid in the mix's `from`
//!   bottle type + the ingredient gives it in the `to` bottle type (gunpowder makes it splash, dragon's breath
//!   lingering).
//!
//! A regular bottle of water is plain `minecraft:water`, not a potion fluid (`PotionFluidHandler.getFluidFromPotion`),
//! so the first brew (water + nether wart, awkward) takes a basin of water.
//!
//! Known differences from Create, for the README:
//! - Create's `potion_mixing_modded_<n>` pass (NeoForge's `BrewingRecipe`s that a mod registers, whose output is a
//!   potion item) is not ported: Horizon's brewing recipes are all data, and what that pass would add (recipes with a
//!   potion item for output and a plain input) is read as a potion or container mix above where it fits and left out
//!   where it doesn't;
//! - the potion registry (`Registries.POTION`, which the container mixes go over) is vanilla's list below, plus any
//!   potion the brewing recipes name.

use super::potion_fluid::{self, Bottle};

/// `AllConfigs.server().recipes.allowBrewingInMixer`.
pub const ALLOW_BREWING_IN_MIXER: bool = true;
/// The amount of every brewing mix (`PotionMixingRecipes.createRecipe` is given 1000 mB stacks).
pub const AMOUNT: i32 = 1000;
/// `PotionMixingRecipes.SUPPORTED_CONTAINERS`.
pub const CONTAINERS: [&str; 3] = ["minecraft:potion", "minecraft:splash_potion", "minecraft:lingering_potion"];

/// `Potions` of 1.21, in registration order (what `Registries.POTION.listElements()` gives).
pub const POTIONS: [&str; 46] = [
    "minecraft:water",
    "minecraft:mundane",
    "minecraft:thick",
    "minecraft:awkward",
    "minecraft:night_vision",
    "minecraft:long_night_vision",
    "minecraft:invisibility",
    "minecraft:long_invisibility",
    "minecraft:leaping",
    "minecraft:long_leaping",
    "minecraft:strong_leaping",
    "minecraft:fire_resistance",
    "minecraft:long_fire_resistance",
    "minecraft:swiftness",
    "minecraft:long_swiftness",
    "minecraft:strong_swiftness",
    "minecraft:slowness",
    "minecraft:long_slowness",
    "minecraft:strong_slowness",
    "minecraft:turtle_master",
    "minecraft:long_turtle_master",
    "minecraft:strong_turtle_master",
    "minecraft:water_breathing",
    "minecraft:long_water_breathing",
    "minecraft:healing",
    "minecraft:strong_healing",
    "minecraft:harming",
    "minecraft:strong_harming",
    "minecraft:poison",
    "minecraft:long_poison",
    "minecraft:strong_poison",
    "minecraft:regeneration",
    "minecraft:long_regeneration",
    "minecraft:strong_regeneration",
    "minecraft:strength",
    "minecraft:long_strength",
    "minecraft:strong_strength",
    "minecraft:weakness",
    "minecraft:long_weakness",
    "minecraft:luck",
    "minecraft:slow_falling",
    "minecraft:long_slow_falling",
    "minecraft:wind_charged",
    "minecraft:weaving",
    "minecraft:oozing",
    "minecraft:infested",
];

/// One `minecraft:brewing` recipe file, read: item lists are already resolved from tags, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Brew {
    /// `input.item`: the bottle items it takes.
    pub input_items: Vec<String>,
    /// `input.potion_contents.potions`: the potions it takes; none for any.
    pub input_potions: Option<Vec<String>>,
    /// `reagent.item`: the ingredient items.
    pub reagent: Vec<String>,
    /// `output.id`.
    pub output_item: String,
    /// `output.components["minecraft:potion_contents"].potion`; none where the output keeps the potion.
    pub output_potion: Option<String>,
}

/// A mixing recipe made from a mix: `from` + one of `reagent` (with heat) gives `to`, both fluid ids of `AMOUNT`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mix {
    /// `create:potion_mixing_vanilla_<n>`.
    pub id: String,
    pub reagent: Vec<String>,
    pub from: String,
    pub to: String,
}

fn is_container(item: &str) -> bool {
    CONTAINERS.contains(&item)
}

fn bottle_of(item: &str) -> Option<Bottle> {
    Bottle::of_item(item)
}

/// `PotionMixingRecipes.createRecipesImpl`'s vanilla part, from the brewing recipes in their order.
pub fn mixes(brews: &[Brew]) -> Vec<Mix> {
    if !ALLOW_BREWING_IN_MIXER {
        return Vec::new();
    }
    // `potionMixes`: (from potion, reagent, to potion), once each.
    let mut potion_mixes: Vec<(String, Vec<String>, String)> = Vec::new();
    // `containerMixes`: (from container, reagent, to container).
    let mut container_mixes: Vec<(String, Vec<String>, String)> = Vec::new();
    // The containers the brewing knows (`isContainer`): the ones its recipes take or make.
    let mut containers: Vec<&'static str> = Vec::new();
    let mut potions: Vec<String> = POTIONS.iter().map(|p| (*p).to_owned()).collect();
    let note_potion = |potions: &mut Vec<String>, potion: &str| {
        if !potions.iter().any(|p| p == potion) {
            potions.push(potion.to_owned());
        }
    };
    for brew in brews {
        if brew.reagent.is_empty() {
            continue;
        }
        let from_containers: Vec<&String> = brew.input_items.iter().filter(|i| is_container(i)).collect();
        if !is_container(&brew.output_item) && from_containers.is_empty() {
            continue;
        }
        for item in from_containers.iter().map(|s| s.as_str()).chain(std::iter::once(brew.output_item.as_str())) {
            if let Some(known) = CONTAINERS.iter().find(|c| **c == item)
                && !containers.contains(known)
            {
                containers.push(known);
            }
        }
        match (&brew.input_potions, &brew.output_potion) {
            (Some(from), Some(to)) => {
                note_potion(&mut potions, to);
                for p in from {
                    note_potion(&mut potions, p);
                    let mix = (p.clone(), brew.reagent.clone(), to.clone());
                    if !potion_mixes.contains(&mix) {
                        potion_mixes.push(mix);
                    }
                }
            }
            (None, None) if is_container(&brew.output_item) => {
                for from in &from_containers {
                    if **from != brew.output_item {
                        let mix = ((*from).clone(), brew.reagent.clone(), brew.output_item.clone());
                        if !container_mixes.contains(&mix) {
                            container_mixes.push(mix);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    // `allowedSupportedContainers` keeps `SUPPORTED_CONTAINERS`' order.
    let allowed: Vec<&str> = CONTAINERS.iter().copied().filter(|c| containers.contains(c)).collect();
    let mut out = Vec::new();
    let mut index = 0;
    let mut push = |out: &mut Vec<Mix>, reagent: &[String], from: String, to: String| {
        out.push(Mix { id: format!("create:potion_mixing_vanilla_{index}"), reagent: reagent.to_vec(), from, to });
        index += 1;
    };
    for container in &allowed {
        let Some(bottle) = bottle_of(container) else { continue };
        for (from, reagent, to) in &potion_mixes {
            push(
                &mut out,
                reagent,
                potion_fluid::from_potion(Some(from), bottle),
                potion_fluid::from_potion(Some(to), bottle),
            );
        }
    }
    for (from, reagent, to) in &container_mixes {
        if !allowed.contains(&from.as_str()) || !allowed.contains(&to.as_str()) {
            continue;
        }
        let (Some(from_bottle), Some(to_bottle)) = (bottle_of(from), bottle_of(to)) else { continue };
        for potion in &potions {
            push(
                &mut out,
                reagent,
                potion_fluid::from_potion(Some(potion), from_bottle),
                potion_fluid::from_potion(Some(potion), to_bottle),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|i| (*i).to_owned()).collect()
    }

    fn awkward() -> Brew {
        Brew {
            input_items: s(&["minecraft:potion"]),
            input_potions: Some(s(&["minecraft:water"])),
            reagent: s(&["minecraft:nether_wart"]),
            output_item: "minecraft:potion".into(),
            output_potion: Some("minecraft:awkward".into()),
        }
    }

    fn gunpowder() -> Brew {
        Brew {
            input_items: s(&["minecraft:potion"]),
            input_potions: None,
            reagent: s(&["minecraft:gunpowder"]),
            output_item: "minecraft:splash_potion".into(),
            output_potion: None,
        }
    }

    fn dragons_breath() -> Brew {
        Brew {
            input_items: s(&["minecraft:splash_potion"]),
            input_potions: None,
            reagent: s(&["minecraft:dragon_breath"]),
            output_item: "minecraft:lingering_potion".into(),
            output_potion: None,
        }
    }

    #[test]
    fn a_potion_mix_runs_for_every_container_in_order() {
        let found = mixes(&[awkward(), gunpowder(), dragons_breath()]);
        assert_eq!(found[0].id, "create:potion_mixing_vanilla_0");
        assert_eq!(found[0].from, "minecraft:water", "a regular bottle of water is plain water");
        assert_eq!(found[0].to, "create:potion[bottle=regular,potion=minecraft:awkward]");
        assert_eq!(found[0].reagent, s(&["minecraft:nether_wart"]));
        assert_eq!(found[1].from, "create:potion[bottle=splash,potion=minecraft:water]");
        assert_eq!(found[1].to, "create:potion[bottle=splash,potion=minecraft:awkward]");
        assert_eq!(found[2].from, "create:potion[bottle=lingering,potion=minecraft:water]");
        assert_eq!(found[2].id, "create:potion_mixing_vanilla_2");
    }

    #[test]
    fn container_mixes_follow_the_potion_mixes_for_every_potion() {
        let found = mixes(&[awkward(), gunpowder(), dragons_breath()]);
        // Three potion mixes, then gunpowder over every potion, then dragon's breath.
        let first = &found[3];
        assert_eq!(first.id, "create:potion_mixing_vanilla_3");
        assert_eq!(first.reagent, s(&["minecraft:gunpowder"]));
        assert_eq!(first.from, "minecraft:water");
        assert_eq!(first.to, "create:potion[bottle=splash,potion=minecraft:water]");
        assert_eq!(found.len(), 3 + 2 * POTIONS.len());
        let last = found.last().unwrap();
        assert_eq!(last.reagent, s(&["minecraft:dragon_breath"]));
        assert_eq!(last.from, "create:potion[bottle=splash,potion=minecraft:infested]");
        assert_eq!(last.to, "create:potion[bottle=lingering,potion=minecraft:infested]");
    }

    #[test]
    fn a_split_potion_mix_is_one_mix_and_unreadable_recipes_make_none() {
        let again = awkward();
        let found = mixes(&[awkward(), again]);
        assert_eq!(found.len(), 1, "the same mix twice is one, over the one container the data names");
        let not_a_brew = Brew {
            input_items: s(&["minecraft:stick"]),
            input_potions: None,
            reagent: s(&["minecraft:stone"]),
            output_item: "minecraft:diamond".into(),
            output_potion: None,
        };
        assert!(mixes(&[not_a_brew]).is_empty());
        assert!(mixes(&[]).is_empty());
    }

    #[test]
    fn a_potion_the_data_names_joins_the_registry() {
        let modded = Brew { output_potion: Some("example:glow".into()), ..awkward() };
        let found = mixes(&[modded, gunpowder()]);
        assert_eq!(found.len(), 2 + POTIONS.len() + 1);
        assert_eq!(found.last().unwrap().to, "create:potion[bottle=splash,potion=example:glow]");
    }
}
