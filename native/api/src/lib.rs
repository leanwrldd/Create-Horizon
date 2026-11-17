//! Create for Horizon, rewritten in Rust: the hooks that put `create-kinetics` in the game.
//!
//! Each kinetic block has a block entity. On the server its tick reports the block to the
//! mod's table of networks and reads back its speed; the table is solved at the start of a
//! server tick whenever a block appeared, changed or went away. Clients get the speed in the
//! block entity's update tag and draw the block turning.

//!
//! The blocks of the first slices (the kinetics core, the depot, the belt, the press) are handled
//! by [`Core`] here. Every later machine is a module of its own under `machines/`, a `Hooks` of
//! its own. The Ponder viewer is under `ponder/`. `forward.rs` hands each hook to the modules.

mod belt;
mod depot;
mod draw;
mod forward;
pub(crate) mod kinetic;
mod machines;
mod ponder;
mod press;
pub(crate) mod recipes;
pub(crate) mod util;

use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex, MutexGuard};

use create_kinetics::generators::{
    CRANK_CAPACITY, CRANK_EXHAUSTION, CRANK_TICKS, WHEEL_CAPACITY, crank_speed, preferred_shaft,
    wheel_score, wheel_speed,
};
use create_kinetics::{Axis, Dir, Kind, Networks, Node, looking};
use horizon_client::mod_api::{
    BlockEntityContainerEvent, BlockEntityEvent, ContainerAcceptsEvent, BlockEntityRenderBoundingBoxEvent, DestroyBlockEvent, BlockEntitySubmitEvent, BlockEntityTickEvent,
    BlockEntityTicksEvent, BlockEntityUpdateTagEvent, BlockStateId, Compound, Direction,
    EquipmentSlot, Flow, Hooks, Host, LevelHost, ModContainer, PlaceBlockEvent,
    RightClickBlockEvent, ServerTickEvent, ServerTickPhase, StandaloneModelsEvent,
    StateForPlacementEvent, Tag, Text, UseItemOnEvent,
};

pub fn hooks() -> Box<dyn Hooks> {
    let mut modules: Vec<Box<dyn Hooks>> = vec![Box::new(Core)];
    modules.extend(machines::all());
    modules.push(Box::new(ponder::Ponder::default()));
    Box::new(forward::Create { modules })
}

/// Every kinetic block the server ticks, whichever module it belongs to.
static NETWORKS: LazyLock<Mutex<Networks>> = LazyLock::new(Mutex::default);

/// The table, never held across a call into the game: a block breaking runs hooks again.
pub(crate) fn networks() -> MutexGuard<'static, Networks> {
    NETWORKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The block entity's speed in RPM, saved and sent to clients.
const SPEED: &str = "Speed";
/// The speed a Creative Motor makes, saved with it.
const GENERATED: &str = "GeneratedSpeed";
/// Create's default for a Creative Motor.
const MOTOR_DEFAULT: f32 = 16.0;
/// Stress units a Creative Motor gives per RPM (Create's `creativeMotor` capacity).
const MOTOR_CAPACITY: f32 = 16384.0;
/// What a wrench steps a Creative Motor through, standing in for Create's scroll value.
const MOTOR_SPEEDS: [f32; 10] = [
    16.0, 32.0, 64.0, 128.0, 256.0, -16.0, -32.0, -64.0, -128.0, -256.0,
];
const WRENCH: &str = "create:wrench";
/// Ticks a Hand Crank keeps turning, counting down from each click.
const IN_USE: &str = "InUse";
/// Whether the last click on a Hand Crank was made sneaking: it turns backwards.
const BACKWARDS: &str = "Backwards";
const SURVIVAL: i32 = 0;
const CREATIVE: i32 = 1;
const SPECTATOR: i32 = 3;

/// Which of the mod's kinetic blocks `id` (`create:shaft`) is, without its state.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Block {
    Shaft,
    Cogwheel,
    LargeCogwheel,
    Gearbox,
    Motor,
    HandCrank,
    WaterWheel,
    Press,
}

impl Block {
    fn of(id: &str) -> Option<Self> {
        Some(match id.strip_prefix("create:")? {
            "shaft" => Block::Shaft,
            "cogwheel" => Block::Cogwheel,
            "large_cogwheel" => Block::LargeCogwheel,
            "gearbox" => Block::Gearbox,
            "creative_motor" => Block::Motor,
            "hand_crank" => Block::HandCrank,
            "water_wheel" => Block::WaterWheel,
            "mechanical_press" => Block::Press,
            _ => return None,
        })
    }

    /// The block with its state: `property` reads one of the state's properties.
    fn kind<'a>(self, property: impl Fn(&str) -> Option<&'a str>) -> Option<Kind> {
        let axis = || property("axis").and_then(Axis::from_name);
        Some(match self {
            Block::Shaft => Kind::Shaft(axis()?),
            Block::Cogwheel => Kind::Cogwheel(axis()?),
            Block::LargeCogwheel => Kind::LargeCogwheel(axis()?),
            Block::Gearbox => Kind::Gearbox(axis()?),
            Block::Motor => Kind::Motor(property("facing").and_then(Dir::from_name)?),
            Block::HandCrank => Kind::Crank(property("facing").and_then(Dir::from_name)?),
            Block::WaterWheel => Kind::WaterWheel(axis()?),
            Block::Press => press::kind(property("facing").and_then(Dir::from_name)?),
        })
    }
}

fn int<K: Ord + for<'a> From<&'a str>>(nbt: &BTreeMap<K, Tag>, key: &str) -> i32 {
    match nbt.get(&K::from(key)) {
        Some(Tag::Int(value)) => *value,
        Some(Tag::Byte(value)) => i32::from(*value),
        _ => 0,
    }
}

fn add(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// A compound's float, whatever its key type is called.
fn float<K: Ord + for<'a> From<&'a str>>(nbt: &BTreeMap<K, Tag>, key: &str) -> Option<f32> {
    match nbt.get(&K::from(key))? {
        Tag::Float(bits) => Some(f32::from_bits(*bits)),
        _ => None,
    }
}

fn set_float(nbt: &mut Compound, key: &str, value: f32) {
    nbt.insert(key.into(), Tag::Float(value.to_bits()));
}

fn dir(direction: Direction) -> Dir {
    match direction {
        Direction::Down => Dir::Down,
        Direction::Up => Dir::Up,
        Direction::North => Dir::North,
        Direction::South => Dir::South,
        Direction::West => Dir::West,
        Direction::East => Dir::East,
    }
}

/// The first slices: shafts, cogwheels, the gearbox, the generators, the depot, the belt and the
/// press.
struct Core;

impl Core {
    fn networks(&self) -> MutexGuard<'static, Networks> {
        networks()
    }

    fn draw_entity(
        host: &dyn Host,
        draw: &mut horizon_client::mod_api::BlockEntityDraw,
        name: &str,
        pos: [i32; 3],
        state: BlockStateId,
        entity: &horizon_client::mod_api::ModBlockEntity,
    ) {
        if name == depot::ID {
            depot::draw(host, draw, entity);
            return;
        }
        if name == belt::ID {
            let facing = host.block_state_property(state, "facing").and_then(Dir::from_name);
            let part = host
                .block_state_property(state, "part")
                .and_then(create_kinetics::belt::Part::from_name);
            let slope = host
                .block_state_property(state, "slope")
                .and_then(create_kinetics::belt::Slope::from_name);
            if let (Some(facing), Some(slope), Some(part)) = (facing, slope, part) {
                belt::draw(host, draw, facing, slope, part, entity);
            }
            return;
        }
        let Some(block) = Block::of(name) else {
            return;
        };
        if block == Block::Press {
            if let Some(facing) = host
                .block_state_property(state, "facing")
                .and_then(Dir::from_name)
            {
                press::draw(host, draw, facing, &entity.nbt);
            }
            return;
        }
        let Some(kind) = block.kind(|key| host.block_state_property(state, key)) else {
            return;
        };
        let speed = float(&entity.nbt, SPEED).unwrap_or(0.0);
        draw::block(host, draw, kind, pos, speed, float(&entity.nbt, ponder::ANGLE));
    }
}

impl Hooks for Core {
    fn standalone_models(&self, event: &mut StandaloneModelsEvent<'_>) {
        event.models.extend(draw::MODELS.map(str::to_owned));
        event.models.extend(press::MODELS.map(str::to_owned));
        event.models.extend(belt::models());
    }

    fn block_entity(
        &self,
        _: &mut dyn Host,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<(String, Compound)> {
        if event.block == depot::ID {
            return Some((depot::ID.to_owned(), Compound::new()));
        }
        if event.block == belt::ID {
            return Some(belt::new_entity());
        }
        let block = Block::of(event.block)?;
        let mut nbt = Compound::new();
        set_float(&mut nbt, SPEED, 0.0);
        match block {
            Block::Motor => set_float(&mut nbt, GENERATED, MOTOR_DEFAULT),
            Block::HandCrank => {
                nbt.insert(IN_USE.into(), Tag::Int(0));
                nbt.insert(BACKWARDS.into(), Tag::Byte(0));
            }
            Block::Press => press::new_entity(&mut nbt),
            _ => {}
        }
        Some((event.block.to_owned(), nbt))
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if Block::of(event.block).is_some() || event.block == depot::ID || event.block == belt::ID {
            event.ticks = true;
        }
    }

    fn block_entity_render_bounding_box(
        &self,
        host: &dyn Host,
        event: &mut BlockEntityRenderBoundingBoxEvent<'_>,
    ) {
        if event.block == belt::ID
            && let Some(facing) = host
                .block_state_property(event.state, "facing")
                .and_then(Dir::from_name)
            && let Some(slope) = host
                .block_state_property(event.state, "slope")
                .and_then(create_kinetics::belt::Slope::from_name)
        {
            belt::bounds(facing, slope, event.entity, event.bounds);
        }
    }

    /// A player breaking a belt block: its items drop first (`BeltBlock.spawnAfterBreak`).
    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        belt::destroyed(host, event.pos);
        Flow::Continue
    }

    fn block_entity_container(
        &self,
        _: &dyn Host,
        event: &mut BlockEntityContainerEvent<'_>,
    ) -> Option<ModContainer> {
        depot::container(event.block)
    }

    /// `DepotItemHandler.insertItem`: only into the empty depot's first slot.
    fn container_accepts(&self, _: &dyn Host, event: &mut ContainerAcceptsEvent<'_>) -> Flow {
        depot::container_accepts(event)
    }

    fn server_tick(&self, _: &mut dyn Host, event: &mut ServerTickEvent<'_>) {
        if let ServerTickPhase::ServerTickStart {
            world_running: true,
        } = event.phase
        {
            self.networks().advance();
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id == depot::ID {
            depot::tick(host, event.pos, event.entity);
            return;
        }
        if event.entity.id == belt::ID {
            belt::tick(host, event.pos, event.entity, &NETWORKS);
            return;
        }
        let Some(block) = Block::of(&event.entity.id) else {
            return;
        };
        let pos = event.pos;
        let (axis, facing) = (
            host.block_property(pos, "axis"),
            host.block_property(pos, "facing"),
        );
        let property = |key: &str| match key {
            "axis" => axis.as_deref(),
            "facing" => facing.as_deref(),
            _ => None,
        };
        let Some(kind) = block.kind(property) else {
            return;
        };
        let mut node = match (block, facing.as_deref().and_then(Dir::from_name)) {
            (Block::Press, Some(facing)) => press::node(facing),
            _ => Node::passive(kind),
        };
        match kind {
            Kind::Motor(_) => {
                node.generated = float(&event.entity.nbt, GENERATED).unwrap_or(MOTOR_DEFAULT);
                node.capacity = MOTOR_CAPACITY;
            }
            Kind::Crank(facing) => {
                let left = int(&event.entity.nbt, IN_USE);
                let backwards = int(&event.entity.nbt, BACKWARDS) != 0;
                node.generated = crank_speed(left, backwards, facing);
                node.capacity = CRANK_CAPACITY;
                // Only written while it counts down: any write marks the entity changed.
                if left > 0 {
                    event.entity.nbt.insert(IN_USE.into(), Tag::Int(left - 1));
                }
            }
            Kind::WaterWheel(axis) => {
                let dimension = host.dimension();
                let score = wheel_score(axis, |offset| {
                    let at = add(pos, offset);
                    // A bubble column pushes straight up, or down over magma
                    // (`WaterWheelBlockEntity.getFlowVectorAtPosition`).
                    if host.block(at) == "minecraft:bubble_column" {
                        let down = host.block_property(at, "drag").as_deref() == Some("true");
                        return [0.0, if down { -1.0 } else { 1.0 }, 0.0];
                    }
                    host.fluid_flow(dimension, at).unwrap_or([0.0; 3])
                });
                node.generated = wheel_speed(score);
                node.capacity = WHEEL_CAPACITY;
            }
            _ => {}
        }
        let (speed, breaks) = self.networks().report(host.dimension(), pos, node);
        if breaks {
            // As Create's blocks do when their network asks them for two speeds at once.
            host.destroy_block(pos, None, false);
            return;
        }
        // Compared first: writing to a block entity, even the same value, counts as a change.
        let speed_changed = float(&event.entity.nbt, SPEED) != Some(speed);
        if speed_changed {
            set_float(&mut event.entity.nbt, SPEED, speed);
        }
        let (over, theory) = kinetic::stress_at(host, pos);
        let mut sync = speed_changed | kinetic::note_stress(&mut event.entity.nbt, over, theory);
        if block == Block::Press {
            sync |= press::tick(host, pos, event.entity, speed, speed_changed);
        }
        if sync {
            host.block_entity_sync(pos);
        }
    }

    fn block_entity_update_tag(
        &self,
        event: &mut BlockEntityUpdateTagEvent<'_>,
    ) -> Option<Compound> {
        if event.entity.id == depot::ID {
            return Some(depot::update_tag(event.entity));
        }
        if event.entity.id == belt::ID {
            return Some(belt::update_tag(event.entity));
        }
        let block = Block::of(&event.entity.id)?;
        let mut tag = Compound::new();
        set_float(
            &mut tag,
            SPEED,
            float(&event.entity.nbt, SPEED).unwrap_or(0.0),
        );
        kinetic::copy_stress(&event.entity.nbt, &mut tag);
        if block == Block::Press {
            press::update_tag(&event.entity.nbt, &mut tag);
        }
        // A bracket on a shaft or a cogwheel (`BracketedBlockEntityBehaviour`).
        if matches!(block, Block::Shaft | Block::Cogwheel | Block::LargeCogwheel) {
            util::copy_bracket(&mut tag, &event.entity.nbt);
        }
        Some(tag)
    }

    fn block_entity_submit(&self, host: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
        Self::draw_entity(host, event.draw, event.block, event.pos, event.state, event.entity);
    }

    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockStateId>> {
        let context = event.context;
        let (key, value) = match Block::of(&context.item)? {
            // The shaft points out of the face clicked, as a log does.
            Block::Shaft | Block::Cogwheel | Block::LargeCogwheel | Block::Gearbox => {
                ("axis", dir(context.face).axis().name())
            }
            // The handle points out of the face clicked; its shaft goes into that block.
            Block::HandCrank => ("facing", dir(context.face).name()),
            // Facing the player (`HorizontalKineticBlock.getStateForPlacement`). Create first
            // lines it up with a shaft beside it, which a placement hook can't read here.
            Block::Press => ("facing", looking(context.yaw, 0.0).opposite().name()),
            // WaterWheelBlock uses DirectionalKineticBlock's look direction as the fallback.
            Block::WaterWheel => ("axis", looking(context.yaw, context.pitch).axis().name()),
            // The shaft points at the player; away from them when sneaking.
            Block::Motor => {
                let look = looking(context.yaw, context.pitch);
                let facing = if context.sneaking {
                    look
                } else {
                    look.opposite()
                };
                ("facing", facing.name())
            }
        };
        level.state_with_property(event.state, key, value).map(Some)
    }

    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        let (key, crank) = match Block::of(&event.stack.item) {
            Some(Block::HandCrank) => ("facing", true),
            Some(Block::WaterWheel) => ("axis", false),
            _ => return,
        };
        if host.acting_player().is_none_or(|player| player.sneaking)
            || matches!(event.stack.data().and_then(|data| data.other("minecraft:block_state")),
                Some(Tag::Compound(properties)) if matches!(properties.get(&key.into()), Some(Tag::String(_))))
        {
            return;
        }

        // The placement context's LevelHost can't read block names. Align once
        // after player placement, where Host can inspect each neighbor.
        let preferred = preferred_shaft(|side| {
            let pos = add(event.pos, side.normal());
            let block = Block::of(&host.block(pos))?;
            let axis = host.block_property(pos, "axis");
            let facing = host.block_property(pos, "facing");
            block.kind(|key| match key {
                "axis" => axis.as_deref(),
                "facing" => facing.as_deref(),
                _ => None,
            })
        });
        if let Some(side) = preferred {
            let value = if crank {
                side.opposite().name()
            } else {
                side.axis().name()
            };
            host.set_block_property(event.pos, key, value);
        }
    }

    /// Turning a Hand Crank, with anything in hand or nothing, as Create's `HandCrankBlock.use`.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        if let Flow::Cancel = depot::right_click(host, event.pos, event.face) {
            return Flow::Cancel;
        }
        if let Flow::Cancel = belt::right_click(host, event.pos) {
            return Flow::Cancel;
        }
        let pos = event.pos;
        if Block::of(&host.block(pos)) != Some(Block::HandCrank) {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        if player.game_mode == SPECTATOR {
            return Flow::Continue;
        }
        // Let the held wrench handle sneak-use before the crank can reverse.
        if player.sneaking
            && [EquipmentSlot::MainHand, EquipmentSlot::OffHand]
                .into_iter()
                .any(|slot| {
                    host.player_equipment(player.uuid, slot)
                        .is_some_and(|stack| &*stack.item == WRENCH)
                })
        {
            return Flow::Continue;
        }
        // Once a click: the off hand's turn would charge the exhaustion again.
        if !util::first_click(host, player.uuid, pos) {
            return Flow::Cancel;
        }
        let backwards = i8::from(player.sneaking);
        host.change_mod_block_entity(pos, &mut |entity| {
            entity.nbt.insert(IN_USE.into(), Tag::Int(CRANK_TICKS));
            if int(&entity.nbt, BACKWARDS) != i32::from(backwards) {
                entity.nbt.insert(BACKWARDS.into(), Tag::Byte(backwards));
            }
        });
        // Turning it is work: Player.causeFoodExhaustion, which creative players skip.
        if player.game_mode != CREATIVE
            && let Some((level, saturation, exhaustion)) = host.player_food(player.uuid)
        {
            let exhaustion = (exhaustion + CRANK_EXHAUSTION).min(40.0);
            host.set_player_status(
                player.uuid,
                None,
                Some((level, saturation, exhaustion)),
                None,
            );
        }
        Flow::Cancel
    }

    /// The wrench picks blocks up while sneaking and adjusts them otherwise.
    fn use_item_on(&self, host: &mut dyn Host, event: &mut UseItemOnEvent<'_>) -> Flow {
        if &*event.stack.item == belt::CONNECTOR {
            return belt::use_connector(host, event.pos);
        }
        if &*event.stack.item != WRENCH {
            return Flow::Continue;
        }
        let pos = event.pos;
        let id = host.block(pos);
        let block = Block::of(&id);
        // The Depot can be wrenched too (`DepotBlock implements IWrenchable`), with nothing to turn.
        if block.is_none() && id != depot::ID {
            return Flow::Continue;
        }
        if let Some(player) = host.acting_player()
            && player.sneaking
        {
            // Create's `IWrenchable.onSneakWrenched`: return the drops in survival.
            if matches!(player.game_mode, SURVIVAL | CREATIVE) {
                host.player_break_block(player.uuid, pos, true);
            }
            return Flow::Cancel;
        }
        let Some(block) = block else {
            return Flow::Continue;
        };
        if block == Block::Motor {
            let mut speed = MOTOR_DEFAULT;
            host.change_mod_block_entity(pos, &mut |entity| {
                let now = float(&entity.nbt, GENERATED).unwrap_or(MOTOR_DEFAULT);
                let index = MOTOR_SPEEDS.iter().position(|s| *s == now);
                speed = MOTOR_SPEEDS[index.map_or(0, |i| (i + 1) % MOTOR_SPEEDS.len())];
                set_float(&mut entity.nbt, GENERATED, speed);
            });
            if let Some(player) = host.acting_player() {
                host.tell(
                    player.uuid,
                    Text::literal(format!("Creative Motor: {speed} RPM")),
                );
            }
        } else if block == Block::Press {
            // A quarter turn clockwise seen from above, as Create's wrench turns a horizontal
            // block clicked on its top (`IWrenchable.getRotatedBlockState`); here on any face.
            if let Some(facing) = host
                .block_property(pos, "facing")
                .as_deref()
                .and_then(Dir::from_name)
            {
                let next = match facing {
                    Dir::North => Dir::East,
                    Dir::East => Dir::South,
                    Dir::South => Dir::West,
                    _ => Dir::North,
                };
                host.set_block_property(pos, "facing", next.name());
            }
        } else if let Some(axis) = host
            .block_property(pos, "axis")
            .as_deref()
            .and_then(Axis::from_name)
        {
            let next = Axis::ALL[(axis.index() + 1) % 3];
            host.set_block_property(pos, "axis", next.name());
        }
        Flow::Cancel
    }
}
