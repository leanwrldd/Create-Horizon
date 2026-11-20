//! Redstone input heard from `neighborChanged` instead of read every tick.
//!
//! Create's redstone-driven blocks react in `Block.neighborChanged` (`GearshiftBlock`, `ClutchBlock`,
//! `StickerBlock`, `DeployerBlock`, `AbstractFunnelBlock`, `PortableStorageInterfaceBlock`, ...), which the
//! game fires on a block whose neighbour changed. The old port had no such event and read
//! `Host::neighbor_signal` (six queries through the signal hooks) in every block entity tick.
//!
//! Now `parts.rs` hands the event here: [`heard`] marks the position dirty, and a block entity's tick calls
//! [`read`], which asks the world only when the position is dirty, has never been read, or on a slow
//! heartbeat ([`HEARTBEAT`]) that stands in for the updates Horizon may not send (Create gets a load-time read
//! from `onPlace`/`firstTick`; a power change two blocks away through a conductor depends on the game firing
//! the neighbour update of the neighbour). The last answer is kept in the block entity's own NBT, so a new
//! block at the same position starts with a fresh read.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};

use crate::compat::{Compound, Host, Tag};
use crate::util;

/// The block entity key of the last signal read.
const HEARD: &str = "HeardSignal";

/// Ticks between reads made whatever the events said; a position is spread over them by its coordinates.
pub const HEARTBEAT: i64 = 20;

/// Every position a block entity has read, by dimension: whether `neighborChanged` fired there since.
static POSITIONS: LazyLock<Mutex<HashMap<(&'static str, [i32; 3]), bool>>> = LazyLock::new(Mutex::default);

/// `neighborChanged` on the block at `pos`: its next [`read`] asks the world.
pub fn heard(dimension: &'static str, pos: [i32; 3]) {
    let mut positions = POSITIONS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(dirty) = positions.get_mut(&(dimension, pos)) {
        *dirty = true;
    }
}

/// `getBestNeighborSignal` at `pos` for the block entity `nbt` belongs to, read from the world only when
/// something told of a change (see the module header); otherwise the last answer.
pub fn read(host: &mut dyn Host, nbt: &mut Compound, pos: [i32; 3]) -> i32 {
    let dimension = host.dimension();
    let dirty = {
        let mut positions = POSITIONS.lock().unwrap_or_else(PoisonError::into_inner);
        std::mem::take(positions.entry((dimension, pos)).or_insert(true))
    };
    let known = util::has(nbt, HEARD);
    if !dirty && known && !beat(host, pos) {
        return util::int(nbt, HEARD);
    }
    let signal = host.neighbor_signal(dimension, pos);
    if !known || util::int(nbt, HEARD) != signal {
        nbt.insert(HEARD.into(), Tag::Int(signal));
    }
    signal
}

fn beat(host: &dyn Host, pos: [i32; 3]) -> bool {
    let spread = i64::from(pos[0]) + 3 * i64::from(pos[1]) + 7 * i64::from(pos[2]);
    (util::game_time(host) + spread).rem_euclid(HEARTBEAT) == 0
}
