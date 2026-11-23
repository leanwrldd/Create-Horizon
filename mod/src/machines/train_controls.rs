//! Driving a train with the movement keys: `ControlsHandler` (the client half, content/contraptions/actors/trainControls/
//! ControlsHandler.java) and `ControlsServerHandler`'s receiving end. The decisions are
//! `create_kinetics::rules::trains::train_controls`; what the server does with the keys is `trains.rs` (`drive_players`).
//!
//! A player starts with `/train drive` near a carriage that has Train Controls (Create: a click on the Controls
//! block, `ControlsInteractionBehaviour`; clicks on a carriage's blocks aren't handled here, the carriage has no
//! entity to click). The server then says so (`create:train_controls`, `1`), and while it holds the client reads the six
//! keys ([`rules::KEYS`]: forward, back, left, right, jump, sneak), takes the movement keys away from the player
//! (`kb.setDown(false)`: a player at the controls doesn't walk) and sends what changed, and a keepalive every
//! [`rules::PACKET_RATE`] ticks. Sneaking stops it (the server answers `0`).
//!
//! Differences from Create: the keys are the default bindings (Horizon gives a mod no way to read a rebound
//! key, as for the Linked Controller), no Escape handling (the sneak key and distance end it), no HUD (`TrainHUD`)
//! and no lever animation (`ControlsRenderer`'s lever and cover are drawn still).

use std::sync::Mutex;

use create_kinetics::rules::trains::train_controls as rules;
use crate::compat::{
    ClientTickPostEvent, Hooks, Host, KeyEvent, MovementInputEvent, NativeScreenKind, NetFromClientEvent, NetFromServerEvent,
};

use super::trains::{CONTROLS_CHANNEL, INBOX};

struct Controls;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Controls)
}

#[derive(Default)]
struct Client {
    controlling: bool,
    /// What each key is now, and what the server was last told.
    down: [bool; 6],
    sent: [bool; 6],
    cooldown: i32,
}

static CLIENT: Mutex<Client> = Mutex::new(Client { controlling: false, down: [false; 6], sent: [false; 6], cooldown: 0 });

fn lock() -> std::sync::MutexGuard<'static, Client> {
    CLIENT.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn playing(host: &dyn Host) -> bool {
    host.client_state().is_some_and(|client| client.screen == NativeScreenKind::Playing)
}

/// `ControlsInputPacket`: the keys and whether they are pressed.
fn packet(keys: &[i32], pressed: bool) -> Vec<u8> {
    let mut out = vec![u8::from(pressed)];
    out.extend(keys.iter().map(|k| *k as u8));
    out
}

fn send(host: &mut dyn Host, keys: &[i32], pressed: bool) {
    if !keys.is_empty() || pressed {
        let _ = host.net_send_to_server(CONTROLS_CHANNEL, &packet(keys, pressed));
    }
}

impl Hooks for Controls {
    // ---- the client ----------------------------------------------------------------------------------------

    fn key(&self, host: &mut dyn Host, event: &mut KeyEvent<'_>) {
        if !host.window() {
            return;
        }
        if let Some(i) = rules::KEYS.iter().position(|k| *k == event.key) {
            lock().down[i] = event.pressed;
        }
    }

    /// `ControlsHandler.tick`.
    fn client_tick_post(&self, host: &mut dyn Host, _: &mut ClientTickPostEvent) {
        if !host.window() {
            return;
        }
        let live = playing(&*host);
        let (released, pressed_new, keepalive_keys, send_keepalive) = {
            let mut client = lock();
            if !client.controlling {
                return;
            }
            if client.cooldown > 0 {
                client.cooldown -= 1;
            }
            // A screen open lets go of everything.
            let down: [bool; 6] = if live { client.down } else { [false; 6] };
            let released: Vec<i32> = (0..6).filter(|i| client.sent[*i as usize] && !down[*i as usize]).collect();
            let pressed_new: Vec<i32> = (0..6).filter(|i| !client.sent[*i as usize] && down[*i as usize]).collect();
            let keepalive: Vec<i32> = (0..6).filter(|i| down[*i as usize]).collect();
            let send_keepalive = client.cooldown == 0;
            if !pressed_new.is_empty() || send_keepalive {
                client.cooldown = rules::PACKET_RATE;
            }
            client.sent = down;
            (released, pressed_new, keepalive, send_keepalive)
        };
        send(host, &released, false);
        send(host, &pressed_new, true);
        if send_keepalive {
            send(host, &keepalive_keys, true);
        }
    }

    /// `controls.forEach(kb -> kb.setDown(false))`: the movement keys belong to the train while controlling.
    fn movement_input(&self, host: &dyn Host, event: &mut MovementInputEvent) {
        if host.window() && lock().controlling {
            event.forward = 0.0;
            event.strafe = 0.0;
            event.jump = false;
            event.sneak = false;
        }
    }

    /// The server says the player now drives (`1`) or no longer does (`0`).
    fn net_from_server(&self, host: &mut dyn Host, event: &mut NetFromServerEvent<'_>) {
        if event.channel != CONTROLS_CHANNEL {
            return;
        }
        let mut client = lock();
        match event.data {
            [1] => {
                client.controlling = true;
                client.sent = [false; 6];
                client.cooldown = 0;
            }
            [0] => {
                *client = Client::default();
            }
            _ => {}
        }
        let _ = host;
    }

    // ---- the server -------------------------------------------------------------------------------------------

    /// `ControlsInputPacket` arrives: the keys wait for the player's train's tick.
    fn net_from_client(&self, _: &mut dyn Host, event: &mut NetFromClientEvent<'_>) {
        if event.channel != CONTROLS_CHANNEL {
            return;
        }
        let [pressed, keys @ ..] = event.data else { return };
        let keys: Vec<i32> = keys.iter().map(|k| i32::from(*k)).filter(|k| (0..6).contains(k)).collect();
        let mut inbox = INBOX.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let queue = inbox.entry(event.player).or_default();
        if queue.len() < 64 {
            queue.push((keys, *pressed != 0));
        }
    }
}
