//! Step 2, the rest: block breaking, entities, players, loot, damage, projectiles, menus, commands and network
//! messages, turned into the old hooks (`compat::events` does the block and block entity ones).
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use super::events::{items_new, items_old, stack_old};
use super::items::with_side;
use super::*;

fn create() -> &'static dyn Hooks {
    crate::create()
}

/// A player's entity id for their uuid: the damage hooks name the victim by entity id, the old event also named the
/// player. Filled by `player_tick_pre` (the new API gives no uuid for an entity id inside a damage hook).
static PLAYERS: Mutex<Option<HashMap<u32, u128>>> = Mutex::new(None);

fn remember_player(entity: u32, uuid: u128) {
    PLAYERS.lock().unwrap_or_else(PoisonError::into_inner).get_or_insert_with(HashMap::new).insert(entity, uuid);
}

fn player_uuid(entity: i32) -> Option<u128> {
    let entity = u32::try_from(entity).ok()?;
    PLAYERS.lock().unwrap_or_else(PoisonError::into_inner).as_ref()?.get(&entity).copied()
}

// --- breaking and placing -----------------------------------------------------------------------------------

/// `Hooks::break_block` (NeoForge's `BlockEvent.BreakEvent`, a player breaking a block): the old `destroy_block`.
pub fn break_block(host: &mut dyn ServerHost, event: &mut mod_api::BreakBlockEvent<'_>) -> mod_api::Flow {
    let mut old = DestroyBlockEvent { pos: event.pos, state: event.state.into() };
    create().destroy_block(&mut Adapter::Server(host), &mut old)
}

/// `ItemPart::on_block_placed_by`: the block is in the world (the old `place_block` ran after placing, and the
/// modules read and change the placed block), with the pre-shrink stack.
pub fn on_block_placed_by(host: &mut dyn ServerHost, event: &mut mod_api::OnBlockPlacedByEvent<'_>) {
    let Some(stack) = stack_old(&*host, event.stack) else { return };
    let mut old = PlaceBlockEvent { stack: &stack, pos: event.pos, state: event.state.into() };
    create().place_block(&mut Adapter::Server(host), &mut old);
}

/// `Hooks::entity_interact`: the old `interact_entity`.
pub fn entity_interact(
    host: HostSide<'_>,
    event: &mut mod_api::EntityInteractEvent<'_>,
) -> Option<mod_api::InteractionResult> {
    let stack = match event.stack {
        Some(stack) => match &host {
            HostSide::Server(host) => stack_old(&**host, stack),
            HostSide::Client(host) => stack_old(&**host, stack),
        },
        None => None,
    };
    let mut old = InteractEntityEvent { entity: event.entity, kind: event.kind, stack: stack.as_ref() };
    match with_side(host, |adapter| create().interact_entity(adapter, &mut old)) {
        Flow::Continue => None,
        _ => Some(mod_api::InteractionResult::Success),
    }
}

// --- players and entities --------------------------------------------------------------------------------------

/// `Hooks::player_tick_pre`: the old `player_tick`, once for each player each tick.
pub fn player_tick_pre(host: HostSide<'_>, event: &mut mod_api::PlayerTickPreEvent) {
    if matches!(host, HostSide::Server(_)) {
        remember_player(event.player.entity().id(), event.player.uuid());
    }
    let mut old = PlayerTickEvent {};
    with_side(host, |adapter| create().player_tick(adapter, &mut old));
}

pub fn entity_tick_listened(host: &mut dyn ServerHost, event: &mut mod_api::EntityTickListenedEvent) {
    let mut old = EntityTickListenedEvent { listened: event.listened };
    create().entity_tick_listened(&mut Adapter::Server(host), &mut old);
    event.listened = old.listened;
}

pub fn entity_tick_post(host: &mut dyn ServerHost, event: &mut mod_api::EntityTickPostEvent<'_>) {
    let mut old = EntityTickPostEvent { dimension: event.dimension, entity: event.entity, kind: event.kind };
    create().entity_tick_post(&mut Adapter::Server(host), &mut old);
}

pub fn living_breathe(event: &mut mod_api::LivingBreatheEvent<'_>) {
    let mut old = LivingBreatheEvent {
        entity: event.entity,
        kind: event.kind,
        can_breathe: event.can_breathe,
        consume: event.consume,
        refill: event.refill,
    };
    create().living_breathe(&mut old);
    event.can_breathe = old.can_breathe;
    event.consume = old.consume;
    event.refill = old.refill;
}

/// `Hooks::living_incoming_damage`: the old `living_incoming_damage_v3`. The players' uuids come from
/// `player_tick_pre` (the damage hook has no host to read them from).
pub fn living_incoming_damage(event: &mut mod_api::LivingIncomingDamageEvent<'_>) -> mod_api::Flow {
    let id = |entity: i32| u32::try_from(entity).ok().filter(|id| *id != 0);
    let mut old = LivingIncomingDamageV3Event {
        victim: u32::try_from(event.entity).unwrap_or(0),
        victim_uuid: player_uuid(event.entity),
        kind: event.kind,
        attacker: id(event.source.causing),
        attacker_uuid: player_uuid(event.source.causing),
        direct: id(event.source.direct),
        damage_type: event.source.damage_type,
        amount: event.container.new_damage(),
    };
    create().living_incoming_damage_v3(&mut old)
}

pub fn living_knockback(event: &mut mod_api::LivingKnockbackEvent<'_>) -> mod_api::Flow {
    let mut old = LivingKnockbackEvent {
        entity: event.entity,
        kind: event.kind,
        damage_type: event.source.damage_type,
        causing: event.source.causing,
        direct: event.source.direct,
        strength: event.strength,
        x: event.x,
        z: event.z,
    };
    let flow = create().living_knockback(&mut old);
    event.strength = old.strength;
    event.x = old.x;
    event.z = old.z;
    flow
}

pub fn hears_projectile_impacts(host: &dyn mod_api::Host, event: &mut mod_api::HearsProjectileImpactsEvent) {
    let mut old = HearsProjectileImpactsEvent { hears: event.hears };
    create().hears_projectile_impacts(&Adapter::Base(host), &mut old);
    event.hears = old.hears;
}

pub fn projectile_impact(host: &mut dyn ServerHost, event: &mut mod_api::ProjectileImpactEvent<'_>) -> mod_api::Flow {
    let mut old = ProjectileImpactEvent {
        dimension: event.dimension,
        projectile: event.projectile,
        kind: event.kind,
        owner: event.owner,
        entity: event.entity,
        entity_kind: event.entity_kind,
        block: event.block,
        face: event.face,
        at: event.at,
    };
    create().projectile_impact(&mut Adapter::Server(host), &mut old)
}

// --- loot ------------------------------------------------------------------------------------------------------

/// `Hooks::loot_drops`: the modules add to or change the drops of a table.
pub fn loot_drops(host: &mut dyn ServerHost, event: &mut mod_api::LootDropsEvent<'_>) {
    let tool = event.context.tool.and_then(|stack| stack_old(&*host, stack));
    let context = NativeLootDrops {
        table: event.context.table,
        dimension: event.context.dimension,
        origin: event.context.origin,
        block: event.context.block.map(|(pos, state)| (pos, state.into())),
        entity: event.context.entity,
        player: event.context.player,
        tool: tool.as_ref(),
        explosion: event.context.explosion,
        used: event.context.used,
    };
    let slots: Vec<Option<mod_api::ItemStack>> = event.drops.iter().cloned().map(Some).collect();
    let mut drops: Vec<ItemStack> = items_old(&*host, &slots).into_iter().flatten().collect();
    let before = drops.clone();
    let mut old = LootDropsEvent { context: &context, drops: &mut drops };
    create().loot_drops(&mut Adapter::Server(&mut *host), &mut old);
    if drops != before {
        let slots: Vec<Option<ItemStack>> = drops.iter().cloned().map(Some).collect();
        *event.drops = items_new(&*host, &slots).into_iter().flatten().collect();
    }
}

// --- menus -----------------------------------------------------------------------------------------------------

pub fn menu_clicked(host: &mut dyn ServerHost, event: &mut mod_api::MenuClickedEvent) {
    let mut old = MenuClickedEvent {
        uuid: event.uuid,
        window: event.window,
        slot: event.slot,
        button: event.button,
        mode: event.mode,
    };
    // A view `Host::open_menu_with_input` opened takes its clicks here.
    if menu_input::clicked(host, event) {
        return;
    }
    create().menu_clicked(&mut Adapter::Server(host), &mut old);
}

pub fn menu_closed(host: &mut dyn ServerHost, event: &mut mod_api::MenuClosedEvent) {
    // The view of `Host::open_menu_with_input` closes: `menu_input_closed` first, then the plain close.
    if let Some(view) = menu_input::closed(event.window) {
        menu_input::closed_to_modules(host, event, view);
    }
    let mut old = MenuClosedEvent { uuid: event.uuid, window: event.window };
    create().menu_closed(&mut Adapter::Server(host), &mut old);
}

// --- commands --------------------------------------------------------------------------------------------------

/// One word of a registered usage: a literal, or an argument's name.
#[derive(Clone, Debug)]
enum Word {
    Literal(String),
    Argument(String),
}

/// The usages the modules registered, as words: `run_mod_command` rebuilds the old raw line from them.
static USAGES: Mutex<Vec<Vec<Word>>> = Mutex::new(Vec::new());

fn words_of(usage: &str) -> Vec<Word> {
    usage
        .split_whitespace()
        .map(|word| match word.strip_prefix('<').or_else(|| word.strip_prefix('[')) {
            Some(argument) => {
                Word::Argument(argument.split(':').next().unwrap_or("").trim_end_matches([']', '>']).to_owned())
            }
            None => Word::Literal(word.to_owned()),
        })
        .collect()
}

/// `Hooks::register_commands`: the old modules' usages, typed commands now.
pub fn register_commands(host: &mut dyn ServerHost, event: &mut mod_api::RegisterCommandsEvent<'_>) {
    let mut commands: Vec<NativeCommand> = Vec::new();
    let mut old = RegisterCommandsEvent { commands: &mut commands };
    create().register_commands(&mut Adapter::Server(host), &mut old);
    let mut usages = USAGES.lock().unwrap_or_else(PoisonError::into_inner);
    usages.clear();
    for command in &commands {
        usages.push(words_of(&command.usage));
        event.commands.push(mod_api::Command::op(&command.usage, command.level));
    }
}

fn value_text(value: &mod_api::CommandArgumentValue) -> Option<String> {
    use mod_api::CommandArgumentValue as Value;
    Some(match value {
        Value::String(text) => text.clone(),
        Value::Integer(number) => number.to_string(),
        Value::Number(number) => number.to_string(),
        Value::Boolean(flag) => flag.to_string(),
        _ => return None,
    })
}

/// The raw line the old `perform_command` took: the usage whose literals and arguments the invocation matches, with
/// the argument values put back in their places.
fn line_of(command: &mod_api::CommandInvocation) -> Option<String> {
    let usages = USAGES.lock().unwrap_or_else(PoisonError::into_inner);
    let usage = usages.iter().find(|words| {
        let literals: Vec<&str> =
            words.iter().filter_map(|w| if let Word::Literal(l) = w { Some(l.as_str()) } else { None }).collect();
        let arguments: Vec<&str> =
            words.iter().filter_map(|w| if let Word::Argument(a) = w { Some(a.as_str()) } else { None }).collect();
        literals == command.literals.iter().map(String::as_str).collect::<Vec<_>>()
            && arguments.len() == command.arguments.len()
            && arguments.iter().all(|name| command.argument(name).is_some())
    })?;
    let mut line = Vec::new();
    for word in usage {
        match word {
            Word::Literal(text) => line.push(text.clone()),
            Word::Argument(name) => line.push(value_text(command.argument(name)?)?),
        }
    }
    Some(line.join(" "))
}

/// `Hooks::run_mod_command`: the old `perform_command`.
pub fn run_mod_command(host: &mut dyn ServerHost, event: &mut mod_api::RunModCommandEvent<'_>) -> mod_api::Flow {
    let Some(line) = line_of(event.command) else { return mod_api::Flow::Continue };
    let mut adapter = Adapter::Server(host);
    let Some(player) = adapter.acting_player() else { return mod_api::Flow::Continue };
    let mut old = PerformCommandEvent { player: &player, command: &line };
    create().perform_command(&mut adapter, &mut old)
}

// --- network messages ------------------------------------------------------------------------------------------

/// The one channel every old message travels on: `[u16 channel length][channel][data]`.
const NET_ID: &str = "create:net";

fn frame(channel: &str, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + channel.len() + data.len());
    out.extend_from_slice(&(channel.len() as u16).to_be_bytes());
    out.extend_from_slice(channel.as_bytes());
    out.extend_from_slice(data);
    out
}

fn unframe(bytes: &[u8]) -> Option<(&str, &[u8])> {
    let len = usize::from(u16::from_be_bytes([*bytes.first()?, *bytes.get(1)?]));
    let channel = std::str::from_utf8(bytes.get(2..2 + len)?).ok()?;
    Some((channel, &bytes[2 + len..]))
}

/// Receives the old messages: on the server they are `net_from_client`, on a client `net_from_server`.
struct Net;

impl IPayloadHandler for Net {
    fn handle(&self, payload: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), PayloadHandleError> {
        let Some((channel, data)) = unframe(&payload.0) else {
            return Err(PayloadHandleError::Failed("malformed create:net message".to_owned()));
        };
        let failed = |e: PayloadContextError| PayloadHandleError::Failed(format!("{e:?}"));
        match context.flow() {
            PayloadDirection::ToServer => {
                let player = context.player().map_err(failed)?.uuid();
                let host = context.server_host().map_err(failed)?;
                let mut old = NetFromClientEvent { player, channel, data };
                create().net_from_client(&mut Adapter::Server(host), &mut old);
            }
            _ => {
                let host = context.client_host().map_err(failed)?;
                let mut old = NetFromServerEvent { channel, data };
                create().net_from_server(&mut Adapter::ClientMut(host), &mut old);
            }
        }
        Ok(())
    }
}

/// Declares the channel, in `Hooks::register`.
pub fn register_net(r: &mut dyn Registrar) {
    let mut handlers = r.payload_handlers();
    let mut view = handlers.registrar("1");
    view.play_bidirectional(PayloadId(NET_ID.to_owned()), Arc::new(Net), Some(Arc::new(Net)));
}

impl Adapter<'_> {
    /// `Host::net_send_to_player`.
    pub(crate) fn net_to_player(&mut self, player: u128, channel: &str, data: &[u8]) -> Result<(), String> {
        let Adapter::Server(host) = self else { return Err("not on the server".to_owned()) };
        let Some(player) = host.player(player) else { return Err("no such player".to_owned()) };
        player
            .send_payload(&mut **host, &PayloadId(NET_ID.to_owned()), PayloadBytes(frame(channel, data)))
            .map(drop)
            .map_err(|error| format!("{error:?}"))
    }

    /// `Host::net_send_to_server`.
    pub(crate) fn net_to_server(&mut self, channel: &str, data: &[u8]) -> Result<(), String> {
        let Adapter::ClientMut(host) = self else { return Err("not on a client that can send".to_owned()) };
        host.send_payload_to_server(&PayloadId(NET_ID.to_owned()), PayloadBytes(frame(channel, data)))
            .map(drop)
            .map_err(|error| format!("{error:?}"))
    }
}
