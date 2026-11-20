//! The Stock Link (`create:stock_link`) (`PackagerLinkBlock`, `PackagerLinkBlockEntity`, `LogisticallyLinkedBehaviour`,
//! `PackagerLinkGenerator`, `AllShapes.STOCK_LINK`): a small block that sits on a Packager and puts it on a logistics
//! network, a frequency shared by every Stock Link, Stock Ticker and Redstone Requester on it. The network's rules are
//! `create_kinetics::rules::logistics_network`; the shared run time is `logistics.rs`; this is the block.
//!
//! How it works here:
//! - It attaches to the face of the first block the player looks towards (floor, wall or ceiling, as Create), and the Packager
//!   it is attached to turns `linked`: it stops answering redstone and serves the network's orders instead
//!   (`PackagerBlockEntity.redstoneModeActive`).
//! - A placed link starts a new network; using a Stock Link item on a linked block takes that network to the next link, ticker or
//!   requester you place (see `logistics.rs`).
//! - A redstone signal on the link is its priority: lower first, and 15 takes it off the network (`redstonePowerChanged`).
//!
//! Known differences from Create:
//! - Its signal is the strongest neighbour signal (Create skips the attached side, which a Packager never powers anyway).
//! - No bulb light, no Wi-Fi particles and no waterlogging; the Wrench only picks it up.

use create_kinetics::rules::packager as packager_rules;
use create_kinetics::rules::stock_link as rules;
use create_kinetics::{Dir, Pos};
use crate::compat::{
    BlockEntityEvent, BlockEntityTickEvent, BlockEntityTicksEvent, BlockStateId, Compound, DestroyBlockEvent, Flow, Hooks, Host,
    LevelHost, PlaceBlockEvent, StateForPlacementEvent, UseItemOnEvent,
};

use super::logistics as network;
use super::packager::PACKAGER;
use crate::util;

struct StockLinks;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(StockLinks)
}

/// The Packager a Stock Link at `pos` is attached to.
fn attached_packager(host: &dyn Host, pos: Pos) -> Option<Pos> {
    let connected = network::link_connection(host, pos)?;
    let at = create_kinetics::rules::logistics_network::packager_pos(pos, connected);
    (host.block(at) == PACKAGER).then_some(at)
}

/// `PackagerBlockEntity.recheckIfLinksPresent`: the Packager is `linked` while a Stock Link is attached to one of its faces.
fn recheck(host: &mut dyn Host, packager: Pos, ignoring: Option<Pos>) {
    if host.block(packager) != PACKAGER {
        return;
    }
    let mut linked = false;
    for d in Dir::ALL {
        let at = util::add(packager, d.normal());
        if Some(at) == ignoring || host.block(at) != network::STOCK_LINK {
            continue;
        }
        if network::link_connection(&*host, at) == Some(d) {
            linked = true;
        }
    }
    let now = host.block_property(packager, "linked").as_deref() == Some("true");
    if linked != now {
        host.set_block_property(packager, "linked", if linked { "true" } else { "false" });
    }
}

impl Hooks for StockLinks {
    fn block_entity(&self, _: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        (event.block == network::STOCK_LINK).then(|| (event.block.to_owned(), Compound::new()))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if event.block == network::STOCK_LINK {
            event.ticks = true;
        }
    }

    /// `FaceAttachedHorizontalDirectionalBlock.getStateForPlacement` and `PackagerLinkBlock.getStateForPlacement`.
    fn state_for_placement(&self, level: &dyn LevelHost, event: &mut StateForPlacementEvent<'_>) -> Option<Option<BlockStateId>> {
        let context = event.context;
        if &*context.item != network::STOCK_LINK {
            return None;
        }
        let look = packager_rules::nearest_looking(context.yaw, context.pitch)[0];
        let (face, facing) = rules::placement(look, create_kinetics::rules::deployer::horizontal_direction(context.yaw));
        let state = level.state_with_property(event.state, "face", face.name())?;
        Some(level.state_with_property(state, "facing", facing.name()))
    }

    /// The network the player tuned, `POWERED` from the neighbour signal, and the Packager turning `linked`.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if &*event.stack.item != network::STOCK_LINK {
            return;
        }
        let pos = event.pos;
        network::note_placed(host, pos);
        let dimension = host.dimension();
        if host.neighbor_signal(dimension, pos) > 0 {
            host.set_block_property(pos, "powered", "true");
        }
        if let Some(packager) = attached_packager(&*host, pos) {
            recheck(host, packager, None);
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id != network::STOCK_LINK {
            return;
        }
        let pos = event.pos;
        let now = util::game_time(&*host);
        let dimension = host.dimension();
        let power = host.neighbor_signal(dimension, pos).clamp(0, 15) as u8;
        let changes = network::tick_link(host, pos, &event.entity.nbt, true, power, now);
        let mut changed = false;
        for (key, value) in changes {
            if util::differs(&event.entity.nbt, key, &value) {
                util::put(&mut event.entity.nbt, key, value);
                changed = true;
            }
        }
        // `PackagerLinkBlock.neighborChanged`: `POWERED` follows the signal.
        let powered = power > 0;
        if (host.block_property(pos, "powered").as_deref() == Some("true")) != powered {
            host.set_block_property(pos, "powered", if powered { "true" } else { "false" });
        }
        // `PackagerLinkBlockEntity.initialize`, and a Packager placed after the link.
        if (now + rules::lazy_salt(pos, 10)) % 10 == 0
            && let Some(packager) = attached_packager(&*host, pos)
        {
            recheck(host, packager, None);
        }
        let _ = changed;
    }

    /// A broken link leaves its network (`destroy`) and its Packager goes back to redstone.
    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        let pos = event.pos;
        if host.block(pos) != network::STOCK_LINK {
            return Flow::Continue;
        }
        let packager = attached_packager(&*host, pos);
        if let Some(entity) = host.mod_block_entity(pos) {
            network::link_removed(host, pos, &entity.nbt, true);
        }
        if let Some(packager) = packager {
            recheck(host, packager, Some(pos));
        }
        Flow::Continue
    }

    /// The Wrench picks the link up (sneaking); the tuning click is `logistics.rs`.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        if host.block(pos) != network::STOCK_LINK || &*event.stack.item != util::WRENCH {
            return Flow::Continue;
        }
        util::wrench(host, pos, |_| {})
    }
}
