//! Minecart Coupling (`MinecartCouplingItem`, `CouplingHandler`, `CouplingPhysics`,
//! `CouplingRenderer`): click one minecart and then another with the item to chain them; they keep
//! their distance and pull each other along. The Wrench on a coupled cart removes its couplings
//! and gives the items back. The rules are `create_kinetics::rules::coupling`; its header lists the
//! known differences from Create (the couplings are a list saved with the level, only the soft
//! velocity step runs, they do not survive a restart, a line is drawn between the carts).
//!
//! Horizon lets a server mod set a minecart's velocity (`Host::entity_set_velocity`: carts are in
//! the entities it simulates), which is all the pull needs; whether the game's own rail movement
//! keeps the corrected cart on its track is not tried in game.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use create_kinetics::rules::coupling::{self as rules, Link};
use crate::compat::{
    AttachmentCodec, AttachmentType, ClientTickPostEvent, Compound, Flow, Holder, Hooks, Host,
    InteractEntityEvent, LevelTickPostEvent, Tag, Text, WorldPrimitivesEvent,
};

use crate::util;

pub const ID: &str = "create:minecart_coupling";

/// The level's couplings, saved with it and sent to the clients that draw them.
static COUPLINGS: AttachmentType<Compound> = AttachmentType {
    namespace: "create",
    name: "couplings",
    default: Compound::new,
    codec: Some(AttachmentCodec {
        encode: |data| Ok(data.clone()),
        decode: |data| Ok(data.clone()),
    }),
    copy_on_death: false,
    sync: true,
};

const LINKS: &str = "Links";

/// A coupling as saved: the cart ids and, to know them again, their uuids.
#[derive(Clone, Debug, PartialEq)]
struct Saved {
    link: Link,
    uuids: [String; 2],
}

/// The first cart each player clicked.
static FIRST: LazyLock<Mutex<HashMap<u128, u32>>> = LazyLock::new(Default::default);
/// What the client last read of the level's couplings, for drawing.
static SEEN: LazyLock<Mutex<Vec<Link>>> = LazyLock::new(Default::default);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

struct Coupling;

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Coupling)
}

fn int(tag: &Tag, key: &str) -> Option<i32> {
    match tag.get(key) {
        Some(Tag::Int(v)) => Some(*v),
        _ => None,
    }
}

fn text(tag: &Tag, key: &str) -> String {
    match tag.get(key) {
        Some(Tag::String(v)) => v.to_string_lossy(),
        _ => String::new(),
    }
}

fn read(nbt: &Compound) -> Vec<Saved> {
    let Some(Tag::List(list)) = util::get(nbt, LINKS) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|entry| {
            Some(Saved {
                link: Link {
                    a: int(entry, "A")? as u32,
                    b: int(entry, "B")? as u32,
                    length: int(entry, "Length")?,
                },
                uuids: [text(entry, "UA"), text(entry, "UB")],
            })
        })
        .collect()
}

fn write(saved: &[Saved]) -> Compound {
    let list = saved
        .iter()
        .map(|s| {
            let mut entry = Compound::new();
            entry.insert("A".into(), Tag::Int(s.link.a as i32));
            entry.insert("B".into(), Tag::Int(s.link.b as i32));
            entry.insert("Length".into(), Tag::Int(s.link.length));
            entry.insert("UA".into(), Tag::String(s.uuids[0].as_str().into()));
            entry.insert("UB".into(), Tag::String(s.uuids[1].as_str().into()));
            Tag::Compound(entry)
        })
        .collect();
    let mut nbt = Compound::new();
    nbt.insert(LINKS.into(), Tag::List(list));
    nbt
}

fn load(host: &mut dyn Host) -> Vec<Saved> {
    // A client has no level attachment to read (the new API gives it no `ServerHost`): nothing to load there.
    if host.window() {
        return Vec::new();
    }
    match COUPLINGS.get_existing(host.new_server(), Holder::Level) {
        Ok(Some(nbt)) => read(&nbt),
        _ => Vec::new(),
    }
}

fn store(host: &mut dyn Host, saved: &[Saved]) {
    if host.window() {
        return;
    }
    let _ = COUPLINGS.set(host.new_server(), Holder::Level, write(saved));
}

fn is_minecart(kind: &str) -> bool {
    kind.starts_with("minecraft:") && kind.ends_with("minecart")
}

fn tell(host: &mut dyn Host, line: &str) {
    if let Some(player) = host.acting_player() {
        host.action_bar(player.uuid, Text::literal(line.to_owned()));
    }
}

fn uuid_of(host: &mut dyn Host, id: u32) -> Option<String> {
    let entity = host.entity(id)?;
    host.entity_uuid(entity).map(|uuid| format!("{uuid:032x}"))
}

impl Hooks for Coupling {
    /// `MinecartCouplingItem.handleInteractionWithMinecart`.
    fn interact_entity(&self, host: &mut dyn Host, event: &mut InteractEntityEvent<'_>) -> Flow {
        if !is_minecart(event.kind) {
            return Flow::Continue;
        }
        let Some(stack) = event.stack else {
            return Flow::Continue;
        };
        let wrench = &*stack.item == util::WRENCH;
        if &*stack.item != ID && !wrench {
            return Flow::Continue;
        }
        let Some(player) = host.acting_player() else {
            return Flow::Continue;
        };
        let cart = event.entity as u32;
        let mut saved = load(host);
        if wrench {
            // `onWrenchInteractOnMinecart`.
            let mut links: Vec<Link> = saved.iter().map(|s| s.link).collect();
            let removed = rules::decouple(&mut links, cart);
            if removed == 0 {
                return Flow::Continue;
            }
            if util::click_repeated_anywhere(host, player.uuid) {
                return Flow::Cancel;
            }
            util::click_taken(host, player.uuid, [0, 0, 0]);
            saved.retain(|s| links.contains(&s.link));
            store(host, &saved);
            if player.game_mode != util::CREATIVE {
                host.give_items(ID, removed as u32);
            }
            tell(host, "Removed all couplings from minecart");
            return Flow::Cancel;
        }
        if util::click_repeated_anywhere(host, player.uuid) {
            return Flow::Cancel;
        }
        util::click_taken(host, player.uuid, [0, 0, 0]);
        let first = lock(&FIRST).remove(&player.uuid);
        let Some(first) = first.filter(|first| *first != cart) else {
            // `CouplingHandlerClient.onCartClicked`: the first click picks the cart.
            lock(&FIRST).insert(player.uuid, cart);
            tell(host, "Now click the second minecart");
            return Flow::Cancel;
        };
        let links: Vec<Link> = saved.iter().map(|s| s.link).collect();
        let positions = host
            .entity(first)
            .zip(host.entity(cart))
            .and_then(|(a, b)| host.entity_position(a).zip(host.entity_position(b)));
        let Some((pa, pb)) = positions else {
            tell(host, "Parts of your train seem to be in unloaded chunks");
            return Flow::Cancel;
        };
        let distance = ((pa[0] - pb[0]).powi(2) + (pa[1] - pb[1]).powi(2) + (pa[2] - pb[2]).powi(2)).sqrt();
        match rules::try_couple(&links, first, cart, distance, rules::MAX_LENGTH) {
            Ok(link) => {
                let (Some(ua), Some(ub)) = (uuid_of(host, first), uuid_of(host, cart)) else {
                    return Flow::Cancel;
                };
                // `heldItem.shrink(1)` unless creative.
                if player.game_mode != util::CREATIVE && host.take_items(ID, 1) != 1 {
                    return Flow::Cancel;
                }
                saved.push(Saved { link, uuids: [ua, ub] });
                store(host, &saved);
            }
            Err(refusal) => {
                if let Some(line) = refusal.message() {
                    tell(host, line);
                }
            }
        }
        Flow::Cancel
    }

    /// `CouplingPhysics.tick`: pulls each coupled pair together, and drops the couplings of carts
    /// that are gone.
    fn level_tick_post(&self, host: &mut dyn Host, _: &mut LevelTickPostEvent<'_>) {
        let saved = load(host);
        if saved.is_empty() {
            return;
        }
        let mut keep: Vec<Saved> = Vec::new();
        for s in &saved {
            let carts = [s.link.a, s.link.b].map(|id| {
                let entity = host.entity(id)?;
                let kind = host.entity_kind(entity)?;
                is_minecart(&kind).then_some(entity)
            });
            let [Some(a), Some(b)] = carts else {
                continue;
            };
            // The ids of a cart that left and came back (or a restart) no longer name the same cart.
            if [(a, 0), (b, 1)]
                .iter()
                .any(|(entity, i)| host.entity_uuid(*entity).map(|u| format!("{u:032x}")).as_deref() != Some(s.uuids[*i].as_str()))
            {
                continue;
            }
            keep.push(s.clone());
            let (Some(pa), Some(pb), Some(va), Some(vb)) = (
                host.entity_position(a),
                host.entity_position(b),
                host.entity_velocity(a),
                host.entity_velocity(b),
            ) else {
                continue;
            };
            let [na, nb] = rules::soft_step([pa, pb], [va, vb], f64::from(s.link.length));
            if na != va {
                host.entity_set_velocity(a, na);
            }
            if nb != vb {
                host.entity_set_velocity(b, nb);
            }
        }
        if keep.len() != saved.len() {
            store(host, &keep);
        }
    }

    /// Reads the couplings the server sent, for drawing.
    fn client_tick_post(&self, host: &mut dyn Host, _: &mut ClientTickPostEvent) {
        let links: Vec<Link> = load(host).iter().map(|s| s.link).collect();
        *lock(&SEEN) = links;
    }

    /// `CouplingRenderer`: a line between the two carts.
    fn world_primitives(&self, host: &dyn Host, event: &mut WorldPrimitivesEvent<'_>) {
        let links = lock(&SEEN).clone();
        for link in links {
            let ends = [link.a, link.b].map(|id| {
                let entity = host.entity(id)?;
                host.entity_position(entity)
            });
            let [Some(a), Some(b)] = ends else {
                continue;
            };
            // From the middle of each cart.
            let up = |p: [f64; 3]| [p[0], p[1] + 0.45, p[2]];
            event.primitives.line(up(a), up(b), 0xFF4A_3A2A, 3.0);
        }
    }
}
