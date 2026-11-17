//! The Mechanical Plough on a structure: what it breaks and what it tills
//! (`PloughMovementBehaviour.canBreak`, `HoeItem.TILLABLES` through `useOn`). Plain values: the glue
//! reads the world, asks this and applies the answer.

/// The cell a plough visits, as `canBreak` sees it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    pub air: bool,
    /// The block below is a `FarmBlock`.
    pub farmland_below: bool,
    /// `LiquidBlock`.
    pub liquid: bool,
    /// `BubbleColumnBlock`.
    pub bubble_column: bool,
    /// `AllBlockTags.PLOUGH_BLACKLIST` (`#minecraft:portals`).
    pub blacklisted: bool,
    /// `AllBlockTags.PLOUGH_WHITELIST` (`minecraft:snow`).
    pub whitelisted: bool,
    /// `getCollisionShape().isEmpty()`.
    pub collision_empty: bool,
}

/// `PloughMovementBehaviour.canBreak`: plants and the like, never anything on farmland, a liquid,
/// a bubble column or a portal; snow always. (Create's tracks are not in this port.)
pub fn can_break(cell: &Cell) -> bool {
    if cell.air || cell.farmland_below || cell.liquid || cell.bubble_column {
        return false;
    }
    if cell.blacklisted {
        return false;
    }
    if cell.whitelisted {
        return true;
    }
    cell.collision_empty
}

/// What the plough's diamond hoe does to the block it meets (`HoeItem.TILLABLES`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tilled {
    /// The block put there.
    pub block: &'static str,
    /// An item that pops out (`changeIntoStateAndDropItem`): the rooted dirt's hanging roots.
    pub drops: Option<&'static str>,
}

/// `HoeItem.useOn` of the block below the visited cell: only with air above it (the visited
/// cell), `TILLABLES.get(block)`.
pub fn till(block: &str, air_above: bool) -> Option<Tilled> {
    if !air_above {
        return None;
    }
    Some(match block {
        "minecraft:grass_block" | "minecraft:dirt_path" | "minecraft:dirt" => {
            Tilled { block: "minecraft:farmland", drops: None }
        }
        "minecraft:coarse_dirt" => Tilled { block: "minecraft:dirt", drops: None },
        "minecraft:rooted_dirt" => Tilled { block: "minecraft:dirt", drops: Some("minecraft:hanging_roots") },
        _ => return None,
    })
}

/// `PloughMovementBehaviour.onBlockBroken`: snow drops as if broken by an iron shovel.
pub fn drops_with_shovel(block: &str) -> bool {
    block == "minecraft:snow"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_breaks_plants_but_not_crops_on_farmland() {
        let grass = Cell { collision_empty: true, ..Cell::default() };
        assert!(can_break(&grass));
        assert!(!can_break(&Cell { farmland_below: true, ..grass }));
        assert!(!can_break(&Cell { air: true, ..grass }));
        assert!(!can_break(&Cell { liquid: true, ..grass }));
        assert!(!can_break(&Cell { bubble_column: true, ..grass }));
        assert!(!can_break(&Cell { blacklisted: true, ..grass }));
        assert!(!can_break(&Cell::default()));
    }

    #[test]
    fn snow_is_broken_even_though_it_has_a_collision() {
        let snow = Cell { whitelisted: true, ..Cell::default() };
        assert!(can_break(&snow));
        assert!(drops_with_shovel("minecraft:snow"));
    }

    #[test]
    fn the_hoe_tills_what_it_does_by_hand() {
        assert_eq!(till("minecraft:grass_block", true).unwrap().block, "minecraft:farmland");
        assert_eq!(till("minecraft:dirt_path", true).unwrap().block, "minecraft:farmland");
        assert_eq!(till("minecraft:coarse_dirt", true).unwrap().block, "minecraft:dirt");
        assert_eq!(till("minecraft:rooted_dirt", true).unwrap().drops, Some("minecraft:hanging_roots"));
        assert_eq!(till("minecraft:grass_block", false), None);
        assert_eq!(till("minecraft:stone", true), None);
    }
}
