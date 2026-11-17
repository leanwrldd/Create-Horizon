//! The Mechanical Harvester and the placement the three attached actors share (`HarvesterBlock`,
//! `PloughBlock`, `RollerBlock`: `AttachedActorBlock`; `HarvesterBlockEntity`, `HarvesterRenderer`).
//! On their own they are blocks you place against a wall; what they do is done on a moving
//! structure (`actor.rs`, `actor_tools.rs`). The rules are `create_kinetics::rules::harvester`.
//!
//! Known differences from Create:
//! - `AttachedActorBlock.canSurvive` (the block breaks when the wall behind it goes) and its
//!   waterlogging are not ported; the placement of a block against a neighbouring actor takes the
//!   clicked face, not the neighbour's own facing.
//! - The Wrench only picks them up (`onWrenched` fails in Create, too).
//! - The stationary Harvester's blade stands still (`getAnimatedSpeed` is a Ponder detail).

use create_kinetics::Dir;
use horizon_client::mod_api::{
    BlockEntityEvent, BlockEntitySubmitEvent, BlockStateId, Compound, Flow, Hooks, Host, LevelHost,
    StandaloneModelsEvent, StateForPlacementEvent, UseItemOnEvent,
};

use crate::util;

pub const HARVESTER: &str = "create:mechanical_harvester";
pub const PLOUGH: &str = "create:mechanical_plough";
pub const ROLLER: &str = "create:mechanical_roller";
const BLADE: &str = "create:block/mechanical_harvester/blade";

struct Harvesters;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Harvesters)
}

fn is_attached_actor(id: &str) -> bool {
    matches!(id, HARVESTER | PLOUGH | ROLLER)
}

impl Hooks for Harvesters {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.push(BLADE.to_owned());
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        (event.block == HARVESTER).then(|| (HARVESTER.to_owned(), Compound::new()))
    }

    /// `HarvesterRenderer`: the blade, at rest.
    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        if event.block != HARVESTER {
            return;
        }
        let Some(facing) = host.block_state_property(event.state, "facing").and_then(Dir::from_name) else {
            return;
        };
        super::actor_draw::draw_blade(host, &mut *event.draw, facing, 0.0);
        event.draw.keep();
    }

    /// `AttachedActorBlock.getStateForPlacement`: against the wall clicked; on a floor or a ceiling,
    /// facing the way the player looks.
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if !is_attached_actor(&context.item) {
            return None;
        }
        let clicked = util::dir(context.face);
        let facing = if clicked.axis() == create_kinetics::Axis::Y {
            // `context.getHorizontalDirection().getOpposite()`.
            create_kinetics::looking(context.yaw, 0.0).opposite()
        } else {
            clicked
        };
        level.state_with_property(event.state, "facing", facing.name()).map(Some)
    }

    /// `IWrenchable.onWrenched` fails for these; sneaking still picks them up.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item != util::WRENCH || !is_attached_actor(&host.block(event.pos)) {
            return Flow::Continue;
        }
        util::wrench(host, event.pos, |_| {})
    }
}
