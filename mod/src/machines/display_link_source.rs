//! Reading the block a Display Link looks at (`DisplayLinkContext.getSourceBlockEntity` and the
//! `provideLine` of each source under `displayLink/source`): the glue that turns the world into
//! a [`Reading`]. What the sources make of it is `create_kinetics::rules::display_link`.
//!
//! A source reads the block entities this port has: the speedometer's and stressometer's speed and
//! network, the cuckoo clock's turning, a Smart Observer's block's items and tanks (through the
//! observer's filter), a Threshold Switch's levels, the items on belts, depots and weighted
//! ejectors, a seat's rider, a target block's power and a nixie tube's text.

use create_kinetics::rules::arm_points::block_title;
use create_kinetics::rules::display_link::{Reading, Source};
use create_kinetics::{Dir, Pos};
use crate::compat::{Hooks, Host, ItemStack};

use super::chute_inv as inv;
use super::content_observer_target as observed;
use super::{nixie_tube, seat, weighted_ejector};
use crate::{belt, depot, kinetic, util};

/// A module of shared code: no hooks of its own.
struct Shared;

impl Hooks for Shared {}

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Shared)
}

const OVERWORLD: &str = "minecraft:overworld";
/// `ItemNameDisplaySource`'s reach along a line of belts.
const NAME_REACH: i32 = 32;
/// `handleCenteredProcessingOnAllItems(.25f, ..)`: how far from the middle of a block an item
/// still counts as on it.
const CENTRED: f32 = 0.25;

/// The name an item stack shows (`getHoverName`): its custom name, else its id's words.
fn item_name(stack: &ItemStack) -> String {
    stack
        .extra
        .as_ref()
        .and_then(|data| data.custom_name_plain())
        .unwrap_or_else(|| block_title(&stack.item))
}

/// What `source` reads at `source_pos` for the link at `link_pos` (`facing` is the link's own).
/// `Reading::Missing` where the block entity the source needs isn't there.
pub fn read(host: &mut dyn Host, source: Source, source_pos: Pos, facing: Dir) -> Reading {
    match source {
        Source::KineticSpeed => host
            .mod_block_entity(source_pos)
            .map_or(Reading::Missing, |entity| Reading::Speed(kinetic::speed(&entity))),
        Source::KineticStress => {
            let network = kinetic::networks().network_at(host.dimension(), source_pos);
            match (host.mod_block_entity(source_pos), network) {
                (Some(_), Some(n)) => Reading::Stress { stress: n.stress, capacity: n.capacity },
                (Some(_), None) => Reading::Stress { stress: 0.0, capacity: 0.0 },
                _ => Reading::Missing,
            }
        }
        Source::TimeOfDay => match host.mod_block_entity(source_pos) {
            Some(clock) => Reading::Clock {
                running: kinetic::speed(&clock) != 0.0,
                day_time: host.day_time(),
                natural: host.dimension() == OVERWORLD,
                random: ((util::random() % 70) as i32, (util::random() % 40) as i32),
            },
            None => Reading::Missing,
        },
        Source::Stopwatch => match host.mod_block_entity(source_pos) {
            Some(clock) => Reading::Stopwatch {
                running: kinetic::speed(&clock) != 0.0,
                game_time: util::game_time(host),
            },
            None => Reading::Missing,
        },
        Source::CountItems | Source::ListItems => read_items(host, source, source_pos),
        Source::CountFluids | Source::ListFluids => read_fluids(host, source, source_pos),
        Source::FillLevel => match host.mod_block_entity(source_pos) {
            Some(switch) => Reading::Level {
                current: util::int(&switch.nbt, "CurrentAmount"),
                min: util::int(&switch.nbt, "CurrentMinAmount"),
                max: util::int(&switch.nbt, "CurrentMaxAmount"),
            },
            None => Reading::Missing,
        },
        Source::ItemNames => Reading::Names(read_names(host, source_pos, facing.opposite())),
        Source::EntityName => {
            let dimension = host.dimension();
            let rider = seat::riders_at(&*host, dimension, source_pos).first().copied();
            Reading::Name(rider.and_then(|uuid| host.profile_by_uuid(uuid)))
        }
        Source::RedstonePower => {
            let power = host
                .block_property(source_pos, "power")
                .and_then(|p| p.parse::<i32>().ok())
                .unwrap_or(0);
            let dimension = host.dimension();
            Reading::Power(power.max(host.neighbor_signal(dimension, source_pos)))
        }
        Source::NixieTube => nixie_tube::text_at(&*host, source_pos).map_or(Reading::Missing, Reading::Text),
    }
}

/// The Smart Observer at `pos`'s filter and the block it looks at.
fn observed_block(host: &mut dyn Host, pos: Pos) -> Option<(inv::Filter, Pos)> {
    let (_, target, facing) = observed::view(&*host, pos)?;
    let (at, _) = observed::target_pos(pos, target, facing);
    let entity = host.mod_block_entity(pos)?;
    Some((inv::filter_of(&entity.nbt), at))
}

/// `ItemCountDisplaySource` and `ItemListDisplaySource`: the observed inventory's stacks that pass
/// the filter, counted, or listed by kind.
fn read_items(host: &mut dyn Host, source: Source, pos: Pos) -> Reading {
    let Some((filter, at)) = observed_block(host, pos) else {
        return Reading::Missing;
    };
    let Some(slots) = observed::read_slots(host, at) else {
        return Reading::Missing;
    };
    let stacks = slots.into_iter().flatten().filter(|stack| stack.count > 0 && filter.test(host, stack));
    if source == Source::CountItems {
        return Reading::Total(stacks.map(|stack| i64::from(stack.count)).sum());
    }
    // `CountedItemStackList`: one entry for each item with the same components.
    let mut kinds: Vec<(ItemStack, i64)> = Vec::new();
    for stack in stacks {
        match kinds.iter_mut().find(|(have, _)| util::same_item(have, &stack)) {
            Some((_, count)) => *count += i64::from(stack.count),
            None => {
                let count = i64::from(stack.count);
                kinds.push((stack, count));
            }
        }
    }
    Reading::List(kinds.iter().map(|(stack, count)| (item_name(stack), *count)).collect())
}

/// `FluidAmountDisplaySource` and `FluidListDisplaySource`.
fn read_fluids(host: &mut dyn Host, source: Source, pos: Pos) -> Reading {
    let Some((filter, at)) = observed_block(host, pos) else {
        return Reading::Missing;
    };
    let Some(tanks) = observed::read_tanks(host, at) else {
        return Reading::Missing;
    };
    let held = tanks.into_iter().filter_map(|tank| {
        let fluid = tank.fluid?;
        (tank.amount > 0 && observed::passes_fluid(&filter, &fluid)).then_some((fluid, i64::from(tank.amount)))
    });
    if source == Source::CountFluids {
        return Reading::Total(held.map(|(_, amount)| amount).sum());
    }
    let mut kinds: Vec<(String, i64)> = Vec::new();
    for (fluid, amount) in held {
        match kinds.iter_mut().find(|(name, _)| *name == fluid) {
            Some((_, total)) => *total += amount,
            None => kinds.push((fluid, amount)),
        }
    }
    Reading::List(kinds.into_iter().map(|(fluid, amount)| (block_title(&fluid), amount)).collect())
}

/// `ItemNameDisplaySource.provideLine`: the stack in the middle of each belt, depot or ejector in
/// a line away from the link, up to 32 of them.
fn read_names(host: &mut dyn Host, start: Pos, along: Dir) -> Vec<String> {
    let mut names = Vec::new();
    let mut at = start;
    for _ in 0..NAME_REACH {
        let block = host.block(at);
        let stack = if block == belt::ID {
            let mut centred = None;
            belt::edit_items(host, at, &mut |_, lane, items| {
                for item in items.iter() {
                    if (item.pos - (lane.index as f32 + 0.5)).abs() < CENTRED {
                        centred = Some(item.stack.clone());
                    }
                }
                false
            });
            centred
        } else if block == depot::ID || block == weighted_ejector::ID {
            host.mod_block_entity(at).and_then(|entity| {
                let along_belt = match util::get(&entity.nbt, "HeldItem") {
                    Some(crate::compat::Tag::Compound(held)) => util::float(held, "Pos").unwrap_or(0.5),
                    _ => 0.5,
                };
                ((along_belt - 0.5).abs() < CENTRED).then(|| entity.items.first().cloned().flatten()).flatten()
            })
        } else {
            break;
        };
        let n = along.normal();
        at = [at[0] + n[0], at[1] + n[1], at[2] + n[2]];
        if let Some(stack) = stack.filter(|s| s.count > 0) {
            names.push(item_name(&stack));
        }
    }
    names
}
