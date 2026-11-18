//! The Encased Fluid Pipe (`create:encased_fluid_pipe`; `EncasedPipeBlock`, `EncasableBlock`): a
//! Fluid Pipe with a Copper Casing right-clicked on it, keeping its six open sides; a Wrench turns it
//! back into a pipe. It carries fluid as the pipe it was (`fluids::classify_for`), and a pipe beside it
//! joins it only on a side it has open (`fluids::transport_faces`).
//!
//! Known differences from Create:
//! - The casing isn't used up (`EncasedPipeBlock.handleEncasing` doesn't shrink it, as Create's).
//! - The encased pipe's sides are kept as they were when it was encased and are never refreshed from
//!   its neighbours (`EncasedPipeBlock` has no `updateShape`), as Create's.
//! - The casing's connected textures (`EncasedCTBehaviour`, `casingConnectivity`) are not applied:
//!   a plain Copper Casing face, as a Fluid Tank's.
//! - Structure rotation and mirroring (`FluidPipeBlockRotation`) are not ported (nothing places
//!   structures), and no waterlogging.

use create_kinetics::Dir;
use crate::compat::{Flow, Hooks, Host, UseItemOnEvent};

use super::fluids::{ENCASED_FLUID_PIPE, FLUID_PIPE};
use crate::util;

/// `AllBlocks.COPPER_CASING`.
const COPPER_CASING: &str = "create:copper_casing";

struct Encased;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Encased)
}

/// The six sides the block at `pos` has open, in `Dir::ALL` order.
fn sides(host: &dyn Host, pos: [i32; 3]) -> [bool; 6] {
    Dir::ALL.map(|d| host.block_property(pos, d.name()).as_deref() == Some("true"))
}

/// `EncasedPipeBlock.transferSixWayProperties` into a fresh block `to`.
fn become_block(host: &mut dyn Host, pos: [i32; 3], to: &str) -> bool {
    let open = sides(&*host, pos);
    let Some(state) = host.default_state(to) else {
        return false;
    };
    let dimension = host.dimension().to_owned();
    host.set_block(&dimension, pos, state, util::UPDATE_ALL);
    for d in Dir::ALL {
        if open[d as usize] {
            host.set_block_property(pos, d.name(), "true");
        }
    }
    true
}

impl Hooks for Encased {
    /// `FluidPipeBlock.useItemOn` (`tryEncase`) with the Copper Casing, and the Wrench on the
    /// encased pipe (`EncasedPipeBlock.onWrenched`).
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        let pos = event.pos;
        let block = host.block(pos);
        let item: &str = &event.stack.item;
        if block == FLUID_PIPE && item == COPPER_CASING {
            if become_block(host, pos, ENCASED_FLUID_PIPE) {
                // `playEncaseSound`: the new block's place sound at `(volume + 1) / 2`, pitch * 0.8.
                host.play_sound("minecraft:block.copper.place", util::centre(pos), 4, 1.0, 0.8);
            }
            return Flow::Cancel;
        }
        if block == ENCASED_FLUID_PIPE && item == util::WRENCH {
            return util::wrench(host, pos, |host| {
                become_block(host, pos, FLUID_PIPE);
            });
        }
        Flow::Continue
    }
}
