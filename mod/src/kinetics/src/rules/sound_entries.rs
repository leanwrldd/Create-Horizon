//! Create's sound entries (`AllSoundEvents`) and what playing one means: which events play, at what volume and
//! pitch, on which mixer group. No game types: the glue is `util::create_sound`.
//!
//! A `SoundEntry` is one of two kinds:
//! - a **custom** entry (`CustomSoundEntry`) names one event with its own `.ogg` (and variants): playing it is one
//!   `playSound(event, category, volume, pitch)`;
//! - a **wrapped** entry (`WrappedSoundEntry`, built with `playExisting`) wraps vanilla events. Its `sounds.json`
//!   writes event `i` as `<name>` (i = 0) or `<name>_compounded_<i>`, each pointing at the vanilla event, and
//!   `play` plays EVERY layer together, each at its configured volume and pitch times the caller's. The
//!   configured numbers are not in `sounds.json`: the caller has to apply them, which [`plays`] does.
//!
//! Known difference from Create, for the README: Create's `playAt` (client-local, `fade`) and `play` (to every
//! player near) both end up as the same server broadcast here; the `fade` flag is not carried.

/// `SoundSource`, by the index the compat layer's `play_sound` takes (`host_services::sound_source`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Records,
    Blocks,
    Neutral,
    Players,
}

impl Category {
    /// The index of the source in the compat layer's list (Master 0, Music 1, Records 2, Weather 3, Blocks 4,
    /// Hostile 5, Neutral 6, Players 7, Ambient 8, Voice 9, Ui 10).
    pub fn source(self) -> usize {
        match self {
            Category::Records => 2,
            Category::Blocks => 4,
            Category::Neutral => 6,
            Category::Players => 7,
        }
    }
}

/// One entry: its path under `create:`, its category and, for a wrapped entry, each layer's configured
/// `(volume, pitch)` (empty for a custom entry, which has one event at 1 and 1).
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub name: &'static str,
    pub category: Category,
    pub layers: &'static [(f32, f32)],
}

const fn custom(name: &'static str, category: Category) -> Entry {
    Entry { name, category, layers: &[] }
}

const fn wrapped(name: &'static str, category: Category, layers: &'static [(f32, f32)]) -> Entry {
    Entry { name, category, layers }
}

use Category::{Blocks as B, Neutral as N, Players as P, Records as R};

/// `AllSoundEvents`, in declaration order.
pub const ENTRIES: &[Entry] = &[
    wrapped("schematicannon_launch_block", B, &[(0.1, 1.1)]),
    wrapped("schematicannon_finish", B, &[(1.0, 0.7)]),
    wrapped("depot_slide", B, &[(0.125, 1.5)]),
    wrapped("depot_plop", B, &[(0.25, 1.25)]),
    wrapped("funnel_flap", B, &[(0.125, 1.5), (0.0425, 0.75)]),
    wrapped("packager", B, &[(0.5, 0.75)]),
    wrapped("slime_added", B, &[(1.0, 1.0)]),
    wrapped("mechanical_press_activation", B, &[(0.125, 1.0), (0.5, 1.0)]),
    wrapped("mechanical_press_activation_belt", B, &[(0.75, 1.0), (0.15, 0.75)]),
    wrapped("mixing", B, &[(0.125, 0.5), (0.125, 0.5)]),
    custom("spout", B),
    wrapped("cranking", B, &[(0.075, 0.5), (0.025, 0.5)]),
    wrapped("worldshaper_place", P, &[(1.0, 1.0)]),
    wrapped("scroll_value", P, &[(0.124, 1.0)]),
    wrapped("confirm", P, &[(0.5, 0.8)]),
    custom("confirm_2", P),
    wrapped("deny", P, &[(1.0, 0.5)]),
    custom("cogs", B),
    custom("fwoomp", P),
    custom("cardboard_bonk", P),
    wrapped("frogport_open", B, &[(1.0, 2.0)]),
    custom("frogport_close", B),
    custom("frogport_catch", B),
    custom("stock_link", B),
    wrapped("frogport_deposit", B, &[(1.0, 1.0)]),
    wrapped("potato_hit", P, &[(0.75, 0.75), (0.75, 1.25)]),
    wrapped("contraption_assemble", B, &[(0.5, 0.5), (0.045, 0.74)]),
    wrapped("contraption_disassemble", B, &[(0.35, 0.75)]),
    wrapped("wrench_rotate", B, &[(0.25, 1.25)]),
    wrapped("wrench_remove", B, &[(0.25, 0.75), (0.25, 0.75)]),
    wrapped("package_pop", B, &[(0.75, 1.0), (0.25, 1.15)]),
    wrapped("crafter_click", B, &[(0.25, 1.0), (0.125, 1.0)]),
    wrapped("crafter_craft", B, &[(0.125, 0.75)]),
    wrapped("copper_armor_equip", P, &[(1.0, 1.0)]),
    custom("sanding_short", B),
    custom("sanding_long", B),
    wrapped("controller_click", B, &[(0.35, 1.0)]),
    wrapped("controller_put", B, &[(1.0, 1.0)]),
    wrapped("controller_take", B, &[(1.0, 1.0)]),
    wrapped("saw_activate_wood", B, &[(0.75, 1.5)]),
    wrapped("saw_activate_stone", B, &[(0.125, 1.25)]),
    wrapped("blaze_munch", B, &[(0.5, 1.0)]),
    wrapped("item_hatch", B, &[(0.25, 1.4), (0.75, 1.15)]),
    wrapped("crushing_1", B, &[(1.0, 1.0)]),
    wrapped("crushing_2", B, &[(1.0, 1.0)]),
    wrapped("crushing_3", B, &[(1.0, 1.0)]),
    wrapped("peculiar_bell_use", B, &[(1.0, 1.0)]),
    custom("desk_bell", B),
    custom("whistle_high", R),
    custom("whistle", R),
    custom("whistle_low", R),
    custom("steam", N),
    custom("train", N),
    custom("train2", N),
    custom("train3", N),
    custom("whistle_train", R),
    custom("whistle_train_low", R),
    custom("whistle_train_manual", N),
    custom("whistle_train_manual_low", N),
    custom("whistle_train_manual_end", N),
    custom("whistle_train_manual_low_end", N),
    custom("chiff", R),
    custom("haunted_bell_convert", B),
    custom("haunted_bell_use", B),
    custom("stock_ticker_request", B),
    custom("stock_ticker_trade", B),
    custom("clipboard_check", B),
    custom("clipboard_erase", B),
];

/// The entry named `name` (`"mixing"` or `"create:mixing"`).
pub fn entry(name: &str) -> Option<&'static Entry> {
    let name = name.strip_prefix("create:").unwrap_or(name);
    ENTRIES.iter().find(|entry| entry.name == name)
}

/// One sound to play: the event (`create:mixing`, `create:mixing_compounded_1`), its mixer group, and the volume
/// and pitch to play it at.
#[derive(Clone, Debug, PartialEq)]
pub struct Play {
    pub event: String,
    pub source: usize,
    pub volume: f32,
    pub pitch: f32,
}

/// `SoundEntry.play`/`playAt` for entry `name` at the caller's `volume` and `pitch`: a custom entry is its one
/// event; a wrapped entry is the main event and each `_compounded_<i>` layer at its configured volume and pitch
/// times the caller's (`event.volume() * volume`, `event.pitch() * pitch`). Empty for a name that isn't an entry.
pub fn plays(name: &str, volume: f32, pitch: f32) -> Vec<Play> {
    let Some(entry) = entry(name) else { return Vec::new() };
    let source = entry.category.source();
    if entry.layers.is_empty() {
        return vec![Play { event: format!("create:{}", entry.name), source, volume, pitch }];
    }
    entry
        .layers
        .iter()
        .enumerate()
        .map(|(i, &(layer_volume, layer_pitch))| Play {
            event: if i == 0 {
                format!("create:{}", entry.name)
            } else {
                format!("create:{}_compounded_{i}", entry.name)
            },
            source,
            volume: layer_volume * volume,
            pitch: layer_pitch * pitch,
        })
        .collect()
}

// --- when and how loud (the numbers a block entity or item passes to `play`) ---------------------------------------

/// `HandCrankBlockEntity.tickAudio`: while the crank is in use (`inUse` counts 10 down to 0) it clicks on every
/// 10th tick with `CRANKING.playAt(.., inUse / 2.5, .65 + (10 - inUse) / 10)`. `ticks` is the game time.
/// Answers `(volume, pitch)`.
pub fn crank_sound(in_use: i32, ticks: i64) -> Option<(f32, f32)> {
    if in_use <= 0 || ticks.rem_euclid(10) != 0 {
        return None;
    }
    Some((in_use as f32 / 2.5, 0.65 + (10 - in_use) as f32 / 10.0))
}

/// `SawBlockEntity.tickAudio`: a shown item that plays (`playEvent`) rings `SAW_ACTIVATE_WOOD` when the item is
/// a block of the wood sound type and `SAW_ACTIVATE_STONE` otherwise, both at volume 3 and pitch 1.
pub fn saw_activation(is_wood: bool) -> (&'static str, f32, f32) {
    (if is_wood { "saw_activate_wood" } else { "saw_activate_stone" }, 3.0, 1.0)
}

/// Whether a block item's block has `SoundType.WOOD` (`SawBlockEntity.tickAudio`: `block.getSoundType(..) ==
/// SoundType.WOOD`). Not a runtime question here: the wooden families of the overworld (a cherry, bamboo, crimson
/// or warped block has a sound type of its own, so it is stone to the saw) and the plain wooden furniture.
pub fn is_wood_item(item: &str) -> bool {
    let name = item.strip_prefix("minecraft:").unwrap_or(item);
    if name.contains(':') {
        return false;
    }
    let name = name.strip_prefix("stripped_").unwrap_or(name);
    const WOODS: [&str; 8] = ["oak", "spruce", "birch", "jungle", "acacia", "dark_oak", "mangrove", "pale_oak"];
    const PARTS: [&str; 12] = [
        "planks", "log", "wood", "slab", "stairs", "fence", "fence_gate", "door", "trapdoor", "button",
        "pressure_plate", "sign",
    ];
    for wood in WOODS {
        let Some(rest) = name.strip_prefix(wood).and_then(|rest| rest.strip_prefix('_')) else { continue };
        let rest = rest.strip_prefix("hanging_").unwrap_or(rest);
        if PARTS.contains(&rest) {
            return true;
        }
    }
    matches!(
        name,
        "crafting_table" | "bookshelf" | "chest" | "trapped_chest" | "barrel" | "lectern" | "jukebox" | "note_block" | "loom"
            | "fletching_table" | "cartography_table" | "smithing_table"
    )
}

/// `SteamEngineBlockEntity.tick`'s stroke, which queues `BoilerData`'s hiss: once each time the crank passes a point,
/// that is once for each turn of the shaft (`speed * 0.3` degrees a tick). True on the tick the turn count rises.
/// Create finds the point from the piston's drawn angle on the client; here it is the shaft's own turn count.
pub fn engine_stroke(speed: f32, ticks: i64) -> bool {
    let turns = |t: i64| (t as f64 * f64::from(speed.abs()) / 1200.0).floor();
    speed != 0.0 && turns(ticks) > turns(ticks - 1)
}

/// `BoilerData.sound`: `CANDLE_EXTINGUISH` at `3 / max(2, engines / 6)` and pitch `1.18 - random * .25`, then
/// `STEAM` at a sixteenth of that volume and pitch 0.8. Answers `(volume, pitch)` of the first; the second is
/// `volume / 16` at 0.8. `dice` is in `[0, 1)`.
pub fn boiler_hiss(engines: i32, dice: f32) -> (f32, f32) {
    (3.0 / (engines / 6).max(2) as f32, 1.18 - dice * 0.25)
}

/// `AllArmorMaterials.COPPER`'s equip sound (`COPPER_ARMOR_EQUIP.getMainEventHolder()`): what the copper diving
/// helmet, boots and Copper Backtank play when worn. `None` for any other item.
pub fn equip_sound(item: &str) -> Option<&'static str> {
    match item.strip_prefix("create:").unwrap_or(item) {
        "copper_diving_helmet" | "copper_diving_boots" | "copper_backtank" => Some("create:copper_armor_equip"),
        _ => None,
    }
}

/// `IWrenchable.playRemoveSound` (`random * .5 + .5`) from a random `r` in `[0, 1)`.
pub fn wrench_remove_pitch(r: f32) -> f32 {
    r * 0.5 + 0.5
}

/// `IWrenchable.playRotateSound` (`random + .5`) from a random `r` in `[0, 1)`.
pub fn wrench_rotate_pitch(r: f32) -> f32 {
    r + 0.5
}

#[cfg(test)]
mod cue_tests {
    use super::*;

    #[test]
    fn the_crank_clicks_every_tenth_tick_while_in_use() {
        assert_eq!(crank_sound(0, 10), None);
        assert_eq!(crank_sound(5, 11), None);
        let (volume, pitch) = crank_sound(10, 20).unwrap();
        assert!((volume - 4.0).abs() < 1e-6 && (pitch - 0.65).abs() < 1e-6);
        let (volume, pitch) = crank_sound(1, 30).unwrap();
        assert!((volume - 0.4).abs() < 1e-6 && (pitch - 1.55).abs() < 1e-6);
    }

    #[test]
    fn the_engine_strokes_once_a_turn_of_the_shaft() {
        // 60 RPM is a turn in 20 ticks.
        let strokes = (1..=100).filter(|&t| engine_stroke(60.0, t)).count();
        assert_eq!(strokes, 5);
        assert!(!(1..100).any(|t| engine_stroke(0.0, t)));
        let fast = (1..=100).filter(|&t| engine_stroke(-120.0, t)).count();
        assert_eq!(fast, 10);
    }

    #[test]
    fn the_boiler_hisses_quieter_with_more_engines() {
        assert_eq!(boiler_hiss(1, 0.0), (1.5, 1.18));
        assert_eq!(boiler_hiss(12, 0.0).0, 1.5);
        assert_eq!(boiler_hiss(24, 0.0).0, 0.75);
        assert!((boiler_hiss(1, 1.0).1 - 0.93).abs() < 1e-6);
    }

    #[test]
    fn copper_gear_clinks_when_worn() {
        assert_eq!(equip_sound("create:copper_diving_helmet"), Some("create:copper_armor_equip"));
        assert_eq!(equip_sound("create:copper_backtank"), Some("create:copper_armor_equip"));
        assert_eq!(equip_sound("create:netherite_diving_boots"), None);
        assert!(entry("copper_armor_equip").is_some());
    }

    #[test]
    fn the_saw_picks_wood_or_stone() {
        assert_eq!(saw_activation(true), ("saw_activate_wood", 3.0, 1.0));
        assert_eq!(saw_activation(false), ("saw_activate_stone", 3.0, 1.0));
    }

    #[test]
    fn wooden_items_are_wood_and_the_rest_is_stone() {
        assert!(is_wood_item("minecraft:oak_log"));
        assert!(is_wood_item("minecraft:stripped_dark_oak_log"));
        assert!(is_wood_item("minecraft:spruce_planks"));
        assert!(is_wood_item("minecraft:mangrove_fence_gate"));
        assert!(is_wood_item("minecraft:acacia_hanging_sign"));
        assert!(is_wood_item("minecraft:crafting_table"));
        assert!(!is_wood_item("minecraft:cherry_planks"));
        assert!(!is_wood_item("minecraft:bamboo_planks"));
        assert!(!is_wood_item("minecraft:crimson_planks"));
        assert!(!is_wood_item("minecraft:stone"));
        assert!(!is_wood_item("create:oak_window"));
        assert!(!is_wood_item("minecraft:oak_sapling"));
    }

    #[test]
    fn wrench_pitches_follow_the_dice() {
        assert_eq!(wrench_remove_pitch(0.0), 0.5);
        assert_eq!(wrench_remove_pitch(1.0), 1.0);
        assert_eq!(wrench_rotate_pitch(0.0), 0.5);
        assert_eq!(wrench_rotate_pitch(1.0), 1.5);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_custom_entry_is_one_event_at_the_callers_numbers() {
        let p = plays("create:deny", 1.0, 1.0);
        // `deny` wraps the note block bass: it is a wrapped entry.
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].event, "create:deny");
        assert_eq!((p[0].volume, p[0].pitch), (1.0, 0.5));
        let p = plays("desk_bell", 0.7, 1.2);
        assert_eq!(p, vec![Play { event: "create:desk_bell".into(), source: 4, volume: 0.7, pitch: 1.2 }]);
    }

    #[test]
    fn a_compound_entry_plays_its_main_event_and_every_layer() {
        let p = plays("mixing", 0.75, 1.0);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].event, "create:mixing");
        assert_eq!(p[1].event, "create:mixing_compounded_1");
        assert!((p[0].volume - 0.09375).abs() < 1e-6);
        assert!((p[1].pitch - 0.5).abs() < 1e-6);
        let p = plays("funnel_flap", 1.0, 2.0);
        assert_eq!(p.len(), 2);
        assert!((p[0].pitch - 3.0).abs() < 1e-6);
        assert!((p[1].volume - 0.0425).abs() < 1e-6);
        assert!((p[1].pitch - 1.5).abs() < 1e-6);
    }

    #[test]
    fn the_configured_numbers_come_from_all_sound_events() {
        let p = plays("item_hatch", 1.0, 1.0);
        assert_eq!((p[0].volume, p[0].pitch), (0.25, 1.4));
        assert_eq!((p[1].volume, p[1].pitch), (0.75, 1.15));
        let p = plays("crafter_click", 1.0, 2.0);
        assert_eq!((p[0].volume, p[0].pitch), (0.25, 2.0));
        assert_eq!((p[1].volume, p[1].pitch), (0.125, 2.0));
        let p = plays("contraption_assemble", 0.75, 0.8);
        assert!((p[0].volume - 0.375).abs() < 1e-6 && (p[0].pitch - 0.4).abs() < 1e-6);
        assert!((p[1].volume - 0.03375).abs() < 1e-6 && (p[1].pitch - 0.592).abs() < 1e-6);
    }

    #[test]
    fn categories_map_to_the_mixer_groups() {
        assert_eq!(plays("whistle", 1.0, 1.0)[0].source, 2);
        assert_eq!(plays("steam", 1.0, 1.0)[0].source, 6);
        assert_eq!(plays("potato_hit", 1.0, 1.0)[0].source, 7);
        assert_eq!(plays("stock_link", 1.0, 1.0)[0].source, 4);
    }

    #[test]
    fn an_unknown_name_plays_nothing() {
        assert!(plays("create:nonsense", 1.0, 1.0).is_empty());
        assert!(entry("minecraft:stone").is_none());
    }

    #[test]
    fn every_entry_is_listed_once_and_the_compound_ones_have_layers() {
        assert_eq!(ENTRIES.len(), 68);
        for (i, a) in ENTRIES.iter().enumerate() {
            assert!(ENTRIES[i + 1..].iter().all(|b| b.name != a.name), "{} twice", a.name);
        }
        let compound: Vec<_> = ENTRIES.iter().filter(|e| e.layers.len() > 1).map(|e| e.name).collect();
        assert_eq!(
            compound,
            [
                "funnel_flap",
                "mechanical_press_activation",
                "mechanical_press_activation_belt",
                "mixing",
                "cranking",
                "potato_hit",
                "contraption_assemble",
                "wrench_remove",
                "package_pop",
                "crafter_click",
                "item_hatch"
            ]
        );
    }
}
