//! What the redstone modules share (the Redstone Link, the Analog Lever, the diodes and the Rose
//! Quartz Lamp): reading a state's properties from a level hook, telling the level of a hook from
//! the level of a `Host`, the sounds, and reading the signal that reaches one side of a block.
//!
//! Horizon's `Host` gives only `neighbor_signal`, the strongest signal of all six neighbours
//! (`getBestNeighborSignal`), while a repeater reads one side (`DiodeBlock.getInputSignal`) and
//! Create's Powered Latch two more (`isPoweredOnSides`). [`read_sides`] gets one side out of
//! it: while it asks, every module's `block_signal` hook zeroes what the other sides give
//! ([`mask`]), so what is left is the signal of the sides asked for. The mask is only on while
//! [`read_sides`] runs, on the game's own thread.

use std::sync::Mutex;

use create_kinetics::{Dir, Pos};
use horizon_client::mod_api::{
    BlockSignalEvent, BlockStateId, Hooks, Host, LevelDimension, LevelHost, Text,
};

use crate::util;

/// A module of shared code: no hooks of its own.
struct Shared;

impl Hooks for Shared {}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

/// `SoundSource.BLOCKS`' ordinal.
pub const BLOCKS: usize = 4;

/// The level of a hook's event as the small number the modules key their tables with. Levels
/// Horizon doesn't name (a mod's own) count as the overworld, as `Dimension::of_level` does.
pub fn level_index(dimension: LevelDimension) -> u8 {
    match dimension {
        LevelDimension::Nether => 1,
        LevelDimension::End => 2,
        _ => 0,
    }
}

/// The same for the level a `Host` is in.
pub fn host_level(host: &dyn Host) -> u8 {
    match host.dimension() {
        "minecraft:the_nether" => 1,
        "minecraft:the_end" => 2,
        _ => 0,
    }
}

/// The value of a boolean property of `state`, or `None` when its block has no such property.
pub fn boolean(level: &dyn LevelHost, state: BlockStateId, key: &str) -> Option<bool> {
    let flipped = level.state_with_property(state, key, "true")?;
    Some(flipped == state)
}

/// Which of `values` a property of `state` is, or `None` when its block has no such property.
pub fn choice(
    level: &dyn LevelHost,
    state: BlockStateId,
    key: &str,
    values: &[&'static str],
) -> Option<&'static str> {
    values
        .iter()
        .copied()
        .find(|&value| level.state_with_property(state, key, value) == Some(state))
}

/// The horizontal `facing` of `state`, if it has one.
pub fn horizontal_facing(level: &dyn LevelHost, state: BlockStateId) -> Option<Dir> {
    choice(level, state, "facing", &["north", "south", "west", "east"]).and_then(Dir::from_name)
}

/// Whether `state` has a property called `key` that takes `value`.
pub fn takes(level: &dyn LevelHost, state: BlockStateId, key: &str, value: &str) -> bool {
    level.state_with_property(state, key, value).is_some()
}

/// `BlockBehaviour.canBeReplaced` for the blocks a Create block is likely to stand next to:
/// what a block that needs a support (`canSurvive`) is torn off by when the support goes.
pub fn replaceable(name: &str) -> bool {
    matches!(
        name,
        "minecraft:air"
            | "minecraft:cave_air"
            | "minecraft:void_air"
            | "minecraft:water"
            | "minecraft:lava"
            | "minecraft:fire"
            | "minecraft:soul_fire"
            | "minecraft:short_grass"
            | "minecraft:tall_grass"
            | "minecraft:fern"
            | "minecraft:large_fern"
            | "minecraft:seagrass"
            | "minecraft:tall_seagrass"
            | "minecraft:vine"
            | "minecraft:glow_lichen"
            | "minecraft:dead_bush"
            | "minecraft:bubble_column"
            | "minecraft:structure_void"
            | "minecraft:light"
    ) || name.is_empty()
}

/// Plays a block sound at the middle of `pos`.
pub fn sound(host: &mut dyn Host, name: &str, pos: Pos, volume: f32, pitch: f32) {
    host.play_sound(name, util::centre(pos), BLOCKS, volume, pitch);
}

/// `SoundEvents.LEVER_CLICK` as Create's blocks play it.
pub fn lever_click(host: &mut dyn Host, pos: Pos, volume: f32, pitch: f32) {
    sound(host, "minecraft:block.lever.click", pos, volume, pitch);
}

/// A line over the acting player's hotbar.
pub fn tell(host: &mut dyn Host, line: &str) {
    if let Some(player) = host.acting_player() {
        host.action_bar(player.uuid, Text::literal(line.to_owned()));
    }
}

/// Whether the acting player holds nothing in either hand.
pub fn hands_empty(host: &dyn Host) -> bool {
    let Some(player) = host.acting_player() else {
        return false;
    };
    host.held_stack(player.uuid).is_none()
        && host
            .player_equipment(player.uuid, horizon_client::mod_api::EquipmentSlot::OffHand)
            .is_none()
}

/// Whether the acting player holds the Wrench in their main hand.
pub fn holds_wrench(host: &dyn Host) -> bool {
    let Some(player) = host.acting_player() else {
        return false;
    };
    host.held_stack(player.uuid)
        .is_some_and(|stack| &*stack.item == util::WRENCH)
}

/// `Player.mayBuild`: not in adventure or spectator mode.
pub fn may_build(game_mode: i32) -> bool {
    game_mode == util::SURVIVAL || game_mode == util::CREATIVE
}

// --- reading one side ---------------------------------------------------------------------------

struct Probe {
    target: Pos,
    /// Which neighbours (in `Dir::ALL` order) are left alone.
    open: [bool; 6],
}

static PROBE: Mutex<Option<Probe>> = Mutex::new(None);

fn index(dir: Dir) -> usize {
    Dir::ALL.iter().position(|&d| d == dir).unwrap_or(0)
}

/// The signal that reaches the block at `pos` through the sides `open` only, as
/// `SignalGetter.getSignal(pos.relative(side), side)` gives it for each of them, and the power of
/// a redstone wire standing there (`DiodeBlock.getInputSignal` counts it even where the wire
/// doesn't point at the diode).
pub fn read_sides(host: &mut dyn Host, pos: Pos, open: &[Dir]) -> i32 {
    let mut flags = [false; 6];
    for &side in open {
        flags[index(side)] = true;
    }
    if let Ok(mut probe) = PROBE.lock() {
        *probe = Some(Probe { target: pos, open: flags });
    }
    let signal = host.neighbor_signal(host.dimension(), pos);
    if let Ok(mut probe) = PROBE.lock() {
        *probe = None;
    }
    let mut wire = 0;
    for &side in open {
        let at = util::add(pos, side.normal());
        if host.block(at) == "minecraft:redstone_wire" {
            let power = host
                .block_property(at, "power")
                .and_then(|p| p.parse::<i32>().ok())
                .unwrap_or(0);
            wire = wire.max(power);
        }
    }
    create_kinetics::rules::diodes::input_with_wire(signal, wire)
}

/// Called by each redstone module's `block_signal` after it has answered for its own blocks:
/// while [`read_sides`] is asking, a signal that reaches a side it didn't ask for is zeroed, so
/// that `neighbor_signal` adds up only the sides it did.
pub fn mask(event: &mut BlockSignalEvent) {
    let Ok(guard) = PROBE.try_lock() else {
        return;
    };
    let Some(probe) = guard.as_ref() else {
        return;
    };
    let source = [event.pos.x, event.pos.y, event.pos.z];
    // The block that reads the signal: the one beside the source's face for a direct signal (one
    // that passes through a conductor), and the asking block itself beside the source otherwise.
    let adjacent = if event.direct {
        let normal = event.face.normal();
        [source[0] + normal[0], source[1] + normal[1], source[2] + normal[2]]
    } else {
        source
    };
    let offset = [
        adjacent[0] - probe.target[0],
        adjacent[1] - probe.target[1],
        adjacent[2] - probe.target[2],
    ];
    if let Some(side) = Dir::from_normal(offset)
        && !probe.open[index(side)]
    {
        event.signal = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::mod_api::{BlockPos, Direction, dispatch_block_signal};

    /// The probe is global: the tests that set it take turns.
    static SERIAL: Mutex<()> = Mutex::new(());

    /// What every redstone module does at the end of its `block_signal`.
    struct Masker;

    impl Hooks for Masker {
        fn block_signal(&self, _: &dyn LevelHost, event: &mut BlockSignalEvent) {
            mask(event);
        }
    }

    struct Level;

    impl LevelHost for Level {
        fn block_state(&self, _: [i32; 3]) -> BlockStateId {
            BlockStateId(0)
        }
        fn is_client_side(&self) -> bool {
            false
        }
        fn state_with_property(&self, _: BlockStateId, _: &str, _: &str) -> Option<BlockStateId> {
            None
        }
    }

    /// A world where some blocks give a weak signal on a face: `gives` is (position, face,
    /// strength).
    struct World {
        gives: Vec<(Pos, Direction, i32)>,
        mods: Vec<Box<dyn Hooks>>,
    }

    impl Host for World {
        fn dimension(&self) -> &'static str {
            "minecraft:overworld"
        }
        fn block(&self, _: Pos) -> String {
            "minecraft:air".to_owned()
        }
        fn block_property(&self, _: Pos, _: &str) -> Option<String> {
            None
        }
        /// `SignalGetter.getBestNeighborSignal`: each neighbour is asked what it gives towards
        /// `pos`, through the hook, as Horizon asks (`face` is the side touching the reader).
        fn neighbor_signal(&mut self, _: &str, pos: Pos) -> i32 {
            let mut best = 0;
            for d in Direction::ALL {
                let n = d.normal();
                let source = [pos[0] + n[0], pos[1] + n[1], pos[2] + n[2]];
                let mut signal = self
                    .gives
                    .iter()
                    .find(|(at, face, _)| *at == source && *face == d.opposite())
                    .map_or(0, |gives| gives.2);
                dispatch_block_signal(
                    &Level,
                    &self.mods,
                    LevelDimension::Overworld,
                    BlockPos::new(source[0], source[1], source[2]),
                    BlockStateId(0),
                    d.opposite(),
                    false,
                    &mut signal,
                );
                best = best.max(signal);
            }
            best
        }
    }

    fn world(gives: Vec<(Pos, Direction, i32)>) -> World {
        World { gives, mods: vec![Box::new(Masker)] }
    }

    #[test]
    fn only_the_sides_asked_for_are_read() {
        let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // The block at [0, 0, 0] has a source behind it (north, giving on its south face), one
        // to its east giving 9, and one above giving 6.
        let mut host = world(vec![
            ([0, 0, -1], Direction::South, 15),
            ([1, 0, 0], Direction::West, 9),
            ([0, 1, 0], Direction::Down, 6),
        ]);
        // As the game answers when nothing is masked: the strongest of all.
        assert_eq!(host.neighbor_signal("minecraft:overworld", [0, 0, 0]), 15);
        assert_eq!(read_sides(&mut host, [0, 0, 0], &[Dir::North]), 15);
        assert_eq!(read_sides(&mut host, [0, 0, 0], &[Dir::East]), 9);
        assert_eq!(read_sides(&mut host, [0, 0, 0], &[Dir::East, Dir::West]), 9);
        assert_eq!(read_sides(&mut host, [0, 0, 0], &[Dir::Up]), 6);
        assert_eq!(read_sides(&mut host, [0, 0, 0], &[Dir::South, Dir::West, Dir::Down]), 0);
        // The mask is off again once the read is over.
        assert_eq!(host.neighbor_signal("minecraft:overworld", [0, 0, 0]), 15);
    }

    #[test]
    fn a_neighbour_that_is_not_beside_the_block_is_not_masked() {
        let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // A probe on [0, 0, 0] leaves signals for another block alone.
        let mut host = world(vec![([5, 0, 4], Direction::South, 12)]);
        if let Ok(mut probe) = PROBE.lock() {
            *probe = Some(Probe { target: [0, 0, 0], open: [false; 6] });
        }
        let seen = host.neighbor_signal("minecraft:overworld", [5, 0, 5]);
        if let Ok(mut probe) = PROBE.lock() {
            *probe = None;
        }
        assert_eq!(seen, 12);
    }
}

/// A level whose block states are tables of property values, for the modules' tests of how a
/// state is read (`decode` and the like).
#[cfg(test)]
pub(crate) mod fake {
    use std::collections::BTreeMap;

    use horizon_client::mod_api::{BlockStateId, LevelHost};

    #[derive(Default)]
    pub struct Registry {
        states: Vec<BTreeMap<&'static str, &'static str>>,
    }

    impl Registry {
        /// Adds a block with every combination of the values its properties take.
        pub fn block(&mut self, props: &[(&'static str, &[&'static str])]) {
            let mut combos: Vec<BTreeMap<&'static str, &'static str>> = vec![BTreeMap::new()];
            for (name, values) in props {
                combos = combos
                    .into_iter()
                    .flat_map(|combo| {
                        values.iter().map(move |value| {
                            let mut next = combo.clone();
                            next.insert(name, value);
                            next
                        })
                    })
                    .collect();
            }
            self.states.extend(combos);
        }

        /// The state with exactly these property values.
        pub fn find(&self, want: &[(&str, &str)]) -> BlockStateId {
            let index = self
                .states
                .iter()
                .position(|state| {
                    state.len() == want.len()
                        && want.iter().all(|(key, value)| state.get(key) == Some(value))
                })
                .expect("no such state");
            BlockStateId(index as u32)
        }
    }

    impl LevelHost for Registry {
        fn block_state(&self, _: [i32; 3]) -> BlockStateId {
            BlockStateId(0)
        }
        fn is_client_side(&self) -> bool {
            false
        }
        /// `BlockState.setValue`: `None` when the block has no such property or value.
        fn state_with_property(
            &self,
            state: BlockStateId,
            key: &str,
            value: &str,
        ) -> Option<BlockStateId> {
            let mut props = self.states.get(state.0 as usize)?.clone();
            let slot = props.get_mut(key)?;
            *slot = self
                .states
                .iter()
                .find_map(|s| s.get(key).copied().filter(|v| *v == value))?;
            self.states
                .iter()
                .position(|s| *s == props)
                .map(|i| BlockStateId(i as u32))
        }
    }
}
