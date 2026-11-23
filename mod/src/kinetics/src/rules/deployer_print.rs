//! A Deployer on a moving structure with a Schematic in its filter slot prints it
//! (`DeployerMovementBehaviour.activateAsSchematicPrinter`): at each cell it reaches it places the
//! block the deployed Schematic has there, if the cell can be built over, taking what the block
//! costs (`ItemRequirement`) out of the structure's storage. This decides what is placed; the
//! glue reads the world and the storage. No game types.
//!
//! Known differences from Create, for the README:
//! - The same as the Schematicannon's (`schematic_printer`): a block costs what `requirement` says,
//!   and the block entity's data is what `safe_block_entity_data` allows.
//! - No `EventHooks.onBlockPlace` (no other mod can veto the placement), no track advancement.
//! - `ItemRequirement.isEmpty` (a block that costs nothing: air) and `isInvalid` both end the stroke,
//!   as in Create; a belt is left out here as in `schematic_printer` (the check Create makes too).
//! - No Universal Creative Crate on a structure (`hasUniversalCreativeCrate`): the Creative Crate
//!   can't be carried here, so every printed block takes its items.

use crate::rules::contraption::{State, replaceable};
use crate::rules::schematic_cannon::safe_block_entity_data;
use crate::rules::schematic_nbt::Nbt;
use crate::rules::schematic_printer::{Layout, Requirement, requirement};
use crate::rules::schematic_structure::Pos;

/// What one stroke places.
#[derive(Clone, Debug, PartialEq)]
pub struct Print {
    /// The state the Schematic has at the cell (the glue filters it as `placeSchematicBlock` does).
    pub state: State,
    /// `BlockHelper.prepareBlockEntityData`.
    pub data: Option<Nbt>,
    /// The items taken from the structure: every one must be there (`ExtractionCountMode.EXACTLY`).
    pub items: Vec<(String, u32)>,
}

/// `activateAsSchematicPrinter` up to the storage: `None` where nothing is to be placed.
/// `deployed` is the Schematic's `SCHEMATIC_DEPLOYED`, `anchor` its `SCHEMATIC_ANCHOR`, `existing` the
/// state now at `pos`.
pub fn plan(
    layout: &Layout,
    deployed: bool,
    anchor: Pos,
    pos: Pos,
    existing: &State,
    item_exists: &dyn Fn(&str) -> bool,
) -> Option<Print> {
    // `level.getBlockState(pos).canBeReplaced()`.
    if !replaceable(existing) {
        return None;
    }
    if !deployed {
        return None;
    }
    let rel = [pos[0] - anchor[0], pos[1] - anchor[1], pos[2] - anchor[2]];
    // `schematicWorld.getBounds().isInside(pos - anchor)`.
    if !layout.bounds.contains(rel) {
        return None;
    }
    let state = layout.state(rel);
    let items = match requirement(&state, item_exists) {
        // `requirement.isInvalid() || requirement.isEmpty()`.
        Requirement::None | Requirement::Invalid => return None,
        Requirement::Items(items) if items.is_empty() => return None,
        Requirement::Items(items) => items,
    };
    // `AllBlocks.BELT.has(blockState)`.
    if state.name == "create:belt" {
        return None;
    }
    let data = layout.nbt(rel).and_then(|nbt| safe_block_entity_data(&state, nbt));
    Some(Print { state, data, items })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::schematic_structure::{Block, Template};
    use crate::rules::schematic_transform::{Mirror, Rotation};

    fn layout(blocks: &[(Pos, &str)]) -> Layout {
        let mut template = Template { size: [3, 1, 1], ..Template::default() };
        for (pos, name) in blocks {
            template.palette.push(State::parse(name));
            template.blocks.push(Block { pos: *pos, state: template.palette.len() - 1, nbt: None });
        }
        Layout::new(&template, Rotation::None, Mirror::None)
    }

    fn exists(_: &str) -> bool {
        true
    }

    fn air() -> State {
        State::new("minecraft:air")
    }

    #[test]
    fn places_the_block_the_schematic_has_at_the_cell_and_asks_for_its_item() {
        let layout = layout(&[([0, 0, 0], "minecraft:stone"), ([1, 0, 0], "create:shaft[axis=x]")]);
        let print = plan(&layout, true, [10, 64, 10], [11, 64, 10], &air(), &exists).unwrap();
        assert_eq!(print.state.name, "create:shaft");
        assert_eq!(print.items, vec![("create:shaft".to_owned(), 1)]);
    }

    #[test]
    fn nothing_is_placed_unless_deployed_or_over_something_replaceable() {
        let layout = layout(&[([0, 0, 0], "minecraft:stone")]);
        assert!(plan(&layout, false, [0, 0, 0], [0, 0, 0], &air(), &exists).is_none());
        let stone = State::new("minecraft:stone");
        assert!(plan(&layout, true, [0, 0, 0], [0, 0, 0], &stone, &exists).is_none());
        assert!(plan(&layout, true, [0, 0, 0], [0, 0, 0], &air(), &exists).is_some());
    }

    #[test]
    fn nothing_outside_the_schematic_or_where_it_has_air_or_an_item_less_block() {
        let layout = layout(&[([0, 0, 0], "minecraft:stone"), ([2, 0, 0], "minecraft:nether_portal")]);
        // Outside the bounds.
        assert!(plan(&layout, true, [0, 0, 0], [0, 5, 0], &air(), &exists).is_none());
        assert!(plan(&layout, true, [0, 0, 0], [-1, 0, 0], &air(), &exists).is_none());
        // Inside the bounds, where the file has no block: air costs nothing.
        assert!(plan(&layout, true, [0, 0, 0], [1, 0, 0], &air(), &exists).is_none());
        // A block the game has no item for.
        assert!(plan(&layout, true, [0, 0, 0], [0, 0, 0], &air(), &|_| false).is_none());
    }

    #[test]
    fn a_belt_is_not_printed() {
        let layout = layout(&[([0, 0, 0], "create:belt[part=start]")]);
        assert!(plan(&layout, true, [0, 0, 0], [0, 0, 0], &air(), &exists).is_none());
    }

    #[test]
    fn only_safe_block_entity_data_travels() {
        let mut template = Template { size: [1, 1, 1], ..Template::default() };
        template.palette.push(State::parse("minecraft:chest"));
        let mut data = Nbt::compound();
        data.put("Items", Nbt::List(Vec::new()));
        template.blocks.push(Block { pos: [0, 0, 0], state: 0, nbt: Some(data) });
        let layout = Layout::new(&template, Rotation::None, Mirror::None);
        let print = plan(&layout, true, [0, 0, 0], [0, 0, 0], &air(), &exists).unwrap();
        assert_eq!(print.data, None);
    }
}
