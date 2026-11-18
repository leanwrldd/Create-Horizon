//! What a Spout does to the block two below it (`BlockSpoutingBehaviour`, `StateChangingBehavior`,
//! `CauldronSpoutingBehavior`, `AllBlockSpoutingBehaviours.registerDefaults`): while it is idle with fluid in its tank,
//! it asks the block below for a behaviour, and if that would take some fluid it runs its 20 ticks and, at tick 5,
//! makes the change.
//!
//! The behaviours Create registers by default (a plain `AllBlockSpoutingBehaviours`):
//! - dirt, coarse dirt and rooted dirt: 250 mB of water makes mud;
//! - farmland: 100 mB of water raises `moisture` by one, up to 7;
//! - a water cauldron: 250 mB of water raises `level` by one, up to 3;
//! - an empty cauldron: 250 mB of water makes a water cauldron, 1000 mB of lava a lava cauldron.
//!
//! Create registers a `SpoutCasting` behaviour for Tinkers' Construct's tables and basins only when that mod is
//! loaded (`compat/tconstruct`); it is not ported. Nor is a Spout over a basin or a campfire: Create 6.0.10 has no
//! behaviour for either (a Basin takes fluid from pipes and pumps; the Spout's tank gives only to what asks of it).
//!
//! Known differences from Create, for the README:
//! - the behaviour is asked again when the fill happens, as Create asks it (`fillBlock(.., false)` at tick 5), but
//!   the memory of which behaviour started the fill isn't saved: a Spout reloaded between the start and tick 5 fills
//!   nothing (Create's `customProcess` isn't saved either);
//! - no splash particles (`sendSplash`).

use super::symmetry_wand_state as state;

/// `Fluids.WATER`'s and `Fluids.LAVA`'s ids as a Spout's tank holds them.
const WATER: &str = "minecraft:water";
const LAVA: &str = "minecraft:lava";

/// What a behaviour does: the fluid it takes from the tank and the state the block becomes (as `/setblock` takes it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fill {
    pub amount: i32,
    pub state: String,
}

/// `BlockSpoutingBehaviour.fillBlock`, for the state of the block below and the fluid the tank holds (`available` mB).
/// `None` when no behaviour takes it (Create's 0).
pub fn fill(block_state: &str, fluid: &str, available: i32) -> Option<Fill> {
    let (name, mut props) = state::parse(block_state);
    match name.as_str() {
        // `StateChangingBehavior.setTo(250, isWater, Blocks.MUD)`.
        "minecraft:dirt" | "minecraft:coarse_dirt" | "minecraft:rooted_dirt" => {
            (fluid == WATER && available >= 250).then(|| Fill { amount: 250, state: "minecraft:mud".to_owned() })
        }
        // `StateChangingBehavior.incrementingState(100, isWater, FarmBlock.MOISTURE)`.
        "minecraft:farmland" => {
            let moisture: i32 = props.get("moisture")?.parse().ok()?;
            if fluid != WATER || available < 100 || moisture >= 7 {
                return None;
            }
            props.insert("moisture".into(), (moisture + 1).to_string());
            Some(Fill { amount: 100, state: state::format(&name, &props) })
        }
        // `StateChangingBehavior.incrementingState(250, isWater, LayeredCauldronBlock.LEVEL)`.
        "minecraft:water_cauldron" => {
            let level: i32 = props.get("level")?.parse().ok()?;
            if fluid != WATER || available < 250 || level >= 3 {
                return None;
            }
            props.insert("level".into(), (level + 1).to_string());
            Some(Fill { amount: 250, state: state::format(&name, &props) })
        }
        // `CauldronSpoutingBehavior`: `CAULDRON_INFO` has water (250, a water cauldron) and lava (1000, a lava cauldron).
        "minecraft:cauldron" => match fluid {
            WATER if available >= 250 => {
                Some(Fill { amount: 250, state: "minecraft:water_cauldron[level=1]".to_owned() })
            }
            LAVA if available >= 1000 => Some(Fill { amount: 1000, state: "minecraft:lava_cauldron".to_owned() }),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirt_becomes_mud_for_a_quarter_bucket_of_water() {
        for dirt in ["minecraft:dirt", "minecraft:coarse_dirt", "minecraft:rooted_dirt"] {
            assert_eq!(fill(dirt, WATER, 250), Some(Fill { amount: 250, state: "minecraft:mud".into() }), "{dirt}");
            assert_eq!(fill(dirt, WATER, 249), None);
            assert_eq!(fill(dirt, LAVA, 1000), None);
        }
        assert_eq!(fill("minecraft:grass_block[snowy=false]", WATER, 1000), None);
    }

    #[test]
    fn farmland_takes_a_tenth_of_a_bucket_a_step_up_to_seven() {
        assert_eq!(
            fill("minecraft:farmland[moisture=3]", WATER, 100),
            Some(Fill { amount: 100, state: "minecraft:farmland[moisture=4]".into() })
        );
        assert_eq!(
            fill("minecraft:farmland[moisture=6]", WATER, 1000).map(|f| f.state),
            Some("minecraft:farmland[moisture=7]".to_owned())
        );
        assert_eq!(fill("minecraft:farmland[moisture=7]", WATER, 1000), None);
        assert_eq!(fill("minecraft:farmland[moisture=0]", WATER, 99), None);
        assert_eq!(fill("minecraft:farmland[moisture=0]", LAVA, 1000), None);
    }

    #[test]
    fn a_water_cauldron_takes_a_quarter_bucket_a_level_up_to_three() {
        assert_eq!(
            fill("minecraft:water_cauldron[level=1]", WATER, 250),
            Some(Fill { amount: 250, state: "minecraft:water_cauldron[level=2]".into() })
        );
        assert_eq!(fill("minecraft:water_cauldron[level=3]", WATER, 1000), None);
        assert_eq!(fill("minecraft:water_cauldron[level=1]", WATER, 100), None);
    }

    #[test]
    fn an_empty_cauldron_takes_water_by_the_quarter_and_lava_by_the_bucket() {
        assert_eq!(
            fill("minecraft:cauldron", WATER, 250),
            Some(Fill { amount: 250, state: "minecraft:water_cauldron[level=1]".into() })
        );
        assert_eq!(fill("minecraft:cauldron", WATER, 249), None);
        assert_eq!(
            fill("minecraft:cauldron", LAVA, 1000),
            Some(Fill { amount: 1000, state: "minecraft:lava_cauldron".into() })
        );
        assert_eq!(fill("minecraft:cauldron", LAVA, 999), None);
        assert_eq!(fill("minecraft:cauldron", "create:honey", 1000), None);
        // A filled cauldron of lava takes nothing.
        assert_eq!(fill("minecraft:lava_cauldron", LAVA, 1000), None);
    }
}
