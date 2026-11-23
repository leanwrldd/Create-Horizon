//! `FactoryPanelSupportBehaviour`: what a Redstone Link or a Display Link keeps of the Factory Gauge panels attached to it
//! (`linkedPanels`, saved as `LinkedGauges`), and what the panel side asks of it. The rules are
//! `rules::factory_gauge` (`Support`, `LinkRead`, `transmitted_power`); this is the saved form and the reads of the world.
//!
//! - A Redstone Link listening (a receiver) is an output: it powers the panel while it receives a signal, which pauses the panel
//!   (`shouldPanelBePowered`: `isOutput() && receivedSignal > 0`). A transmitter sends 15 while any linked panel is satisfied with a
//!   target (`shouldBePoweredTristate`), on top of what its neighbours give (`RedstoneLinkBlock.updateTransmittedSignal`).
//! - A Display Link with panels linked reads them instead of the block it sits on: its source is the first panel's block, the
//!   source the gauge status (`DisplayLinkBlockEntity.getSourcePosition`, `FactoryGaugeDisplaySource`).
//!
//! Known differences from Create, for the README:
//! - A link that is broken is noticed by the panel (its next tick forgets the link), where Create's `destroy` tells each panel at once.
//! - `FactoryPanelSupportBehaviour.notifyLink` for a transmitter is the link's own tick reading the panels every tick.

use create_kinetics::Pos;
use create_kinetics::rules::factory_gauge::{LinkRead, PanelPosition, Support};

use crate::compat::{Compound, Hooks, Host, Tag};
use crate::util;

use super::{display_link, factory_gauge, redstone_link};

/// A module of shared code: no hooks of its own.
struct Shared;

impl Hooks for Shared {}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

const LINKED_GAUGES: &str = "LinkedGauges";

/// Whether the block is one a gauge panel can be connected to (a block with a `FactoryPanelSupportBehaviour`).
pub fn is_support(block: &str) -> bool {
    block == redstone_link::ID || block == display_link::ID
}

/// The saved list of panels of a link's block entity.
pub fn load(nbt: &Compound) -> Support {
    let linked_panels = match util::get(nbt, LINKED_GAUGES) {
        Some(Tag::List(list)) => list.iter().filter_map(factory_gauge::pos_of).collect(),
        _ => Vec::new(),
    };
    Support { linked_panels, changed: false }
}

pub fn store(nbt: &mut Compound, support: &Support) {
    nbt.insert(LINKED_GAUGES.into(), Tag::List(support.linked_panels.iter().map(factory_gauge::pos_tag).collect()));
}

/// The panels attached to the link at `pos`.
pub fn linked_panels(host: &mut dyn Host, pos: Pos) -> Vec<PanelPosition> {
    host.mod_block_entity(pos).map_or_else(Vec::new, |entity| load(&entity.nbt).linked_panels)
}

/// Changes the saved list of the link at `pos`; whether the link is there.
fn edit(host: &mut dyn Host, pos: Pos, change: impl FnOnce(&mut Support)) -> bool {
    if !is_support(&host.block(pos)) {
        return false;
    }
    let mut change = Some(change);
    let mut found = false;
    host.change_mod_block_entity(pos, &mut |entity| {
        let mut support = load(&entity.nbt);
        if let Some(change) = change.take() {
            change(&mut support);
        }
        store(&mut entity.nbt, &support);
        found = true;
    });
    found
}

/// `FactoryPanelSupportBehaviour.connect`.
pub fn connect(host: &mut dyn Host, link: Pos, panel: PanelPosition) -> bool {
    edit(host, link, |support| support.connect(panel))
}

/// `FactoryPanelSupportBehaviour.disconnect`.
pub fn disconnect(host: &mut dyn Host, link: Pos, panel: PanelPosition) -> bool {
    edit(host, link, |support| support.disconnect(panel))
}

/// `FactoryPanelBehaviour.checkForRedstoneInput`'s read of one attached link (`linkAt`).
pub fn read_link(host: &mut dyn Host, pos: Pos) -> LinkRead {
    if !host.chunk_is_loaded(host.dimension(), [pos[0] >> 4, pos[2] >> 4]) {
        return LinkRead::Unloaded;
    }
    let block = host.block(pos);
    if block == display_link::ID {
        return LinkRead::Link { is_output: false, powers_panel: false };
    }
    if block != redstone_link::ID {
        return LinkRead::Missing;
    }
    // `isOutput`: the link listens. `receivedSignal > 0`.
    let output = host.block_property(pos, "receiver").as_deref() == Some("true");
    let received = host.mod_block_entity(pos).map_or(0, |entity| util::int(&entity.nbt, "Receive"));
    LinkRead::Link { is_output: output, powers_panel: output && received > 0 }
}

/// `shouldBePoweredTristate` of the link whose saved form is `nbt` (the one being ticked): whether a linked panel is satisfied; `None`
/// while one is in an unloaded chunk. Panels that are gone are forgotten on the way.
pub fn powered_by_panels(host: &dyn Host, nbt: &mut Compound) -> Option<bool> {
    let mut support = load(nbt);
    if support.linked_panels.is_empty() {
        return Some(false);
    }
    let result = support.should_be_powered(&|at| factory_gauge::panel_read(host, at));
    if support.changed {
        store(nbt, &support);
    }
    result
}
