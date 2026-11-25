//! Mail between a train and the Postboxes of the station it stands at: `GlobalStation.runMailTransfer`
//! (content/trains/station/GlobalStation.java), `GlobalPackagePort` (the port's address, offline buffer and `primed`), and the
//! station's side of `PackagePortTarget.TrainStationFrogportTarget` (`StationBlockEntity.attachPackagePort` /
//! `removePackagePort`, `PackagePortTargetSelectionHandler.validateDiff`). No game types: the glue
//! (`machines/train_mail.rs`) reads each carriage's storage and each port's slots into [`Inventory`]s, calls
//! [`run_mail_transfer`] and writes back what changed.
//!
//! What `runMailTransfer` does, for each carriage that has storage (`getAllItems()`):
//! - **Import from the station**: in each port (in position order here, `HashMap` order in Java), every package that is *not*
//!   for the port's own address (`!matchAddress(stack, port.address)`) goes into the carriage (`insertItemStacked`) and its
//!   slot is emptied. A port whose block is loaded is its Postbox's inventory (and spawns the bone-meal particles); one that
//!   isn't is the offline buffer, which is then `primed` (restored into the Postbox when it loads: `restoreOfflineBuffer`).
//! - **Export to the station**: every package in the carriage goes into the first port its address matches
//!   (`matchAddress(stack, port.address)`); the carriage slot is emptied when all of it went in.
//!
//! The Java quirk is kept: a stack that only partly fits stays in its slot after what fitted was inserted (a package stacks
//! to one, so it cannot happen with real packages). A Creative Crate on a carriage voids what is put into it
//! (`BottomlessItemHandler.insertItem` answers EMPTY): `bottomless` names its blocks and their slots are put back as they were.
//!
//! Left out: the ComputerCraft `package_sent` / `package_received` events, `Create.RAILWAYS.markTracksDirty()` (the glue saves
//! the ports when they change).

use std::collections::BTreeMap;

use crate::Pos;
use crate::rules::package::match_address;
use crate::rules::portable_interface::{Inventory, Slot, Stack};

/// A Postbox's inventory (`PackagePortBlockEntity.inventory`: `ItemStackHandler(18)`), and so the offline buffer's size.
pub const PORT_SLOTS: usize = 18;

/// `PackagePortTargetSelectionHandler.validateDiff`'s limit and the server's `+ 2` (`PackagePortPlacementPacket.handle`):
/// `CLogistics.packagePortRange`, 5.
pub const PORT_RANGE: f64 = 5.0;

/// What the glue says of a stack: `None` when it isn't a package (`PackageItem.isPackage`), else its address
/// (`PackageItem.getAddress`: empty when it has none).
pub trait Packages {
    fn address(&self, stack: &Stack) -> Option<String>;
}

impl<F: Fn(&Stack) -> Option<String>> Packages for F {
    fn address(&self, stack: &Stack) -> Option<String> {
        self(stack)
    }
}

/// One package port as `runMailTransfer` sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Port {
    /// `GlobalPackagePort.address`.
    pub address: String,
    /// The 18 slots: the Postbox's inventory when it is loaded, else the offline buffer.
    pub slots: Vec<Option<Stack>>,
    /// The Postbox's block entity is loaded (`level.isLoaded(pos) && instanceof PostboxBlockEntity`).
    pub loaded: bool,
    /// `GlobalPackagePort.primed`: the offline buffer holds what the Postbox must take when it loads.
    pub primed: bool,
}

impl Port {
    pub fn new(address: &str, slots: Vec<Option<Stack>>, loaded: bool, primed: bool) -> Port {
        Port { address: address.to_owned(), slots, loaded, primed }
    }

    fn inventory(&self, at: Pos) -> Inventory {
        Inventory { slots: self.slots.iter().enumerate().map(|(index, stack)| Slot { block: at, index, stack: stack.clone() }).collect() }
    }
}

/// What a transfer did, for the glue to apply.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Ports whose slots changed.
    pub ports_changed: Vec<Pos>,
    /// Indices of the carriages whose storage changed.
    pub carriages_changed: Vec<usize>,
    /// Loaded Postboxes that spawn their particles (`box.spawnParticles()`).
    pub particles: Vec<Pos>,
    /// Offline ports that are now primed.
    pub primed: Vec<Pos>,
}

fn note(list: &mut Vec<Pos>, pos: Pos) {
    if !list.contains(&pos) {
        list.push(pos);
    }
}

fn note_index(list: &mut Vec<usize>, index: usize) {
    if !list.contains(&index) {
        list.push(index);
    }
}

/// `GlobalStation.runMailTransfer` over the carriages' storage (`carriages`, one inventory each) and the station's ports.
/// `bottomless` says whether a block of carriage `index` is a Creative Crate.
pub fn run_mail_transfer(
    carriages: &mut [Inventory],
    ports: &mut BTreeMap<Pos, Port>,
    packages: &dyn Packages,
    bottomless: &dyn Fn(usize, Pos) -> bool,
) -> Outcome {
    let mut out = Outcome::default();
    if ports.is_empty() {
        return out;
    }
    for (index, inventory) in carriages.iter_mut().enumerate() {
        // Import from station.
        for (&pos, port) in ports.iter_mut() {
            for slot in 0..port.slots.len() {
                let Some(stack) = port.slots[slot].clone() else { continue };
                let Some(address) = packages.address(&stack) else { continue };
                if match_address(&address, &port.address) {
                    continue;
                }
                let left = insert(index, inventory, &stack, bottomless);
                if left > 0 {
                    // Part of it may have gone in; the stack stays where it is (as in Java).
                    if left < stack.count {
                        note_index(&mut out.carriages_changed, index);
                    }
                    continue;
                }
                note_index(&mut out.carriages_changed, index);
                port.slots[slot] = None;
                note(&mut out.ports_changed, pos);
                if port.loaded {
                    note(&mut out.particles, pos);
                } else {
                    port.primed = true;
                    note(&mut out.primed, pos);
                }
            }
        }
        // Export to station.
        for slot in 0..inventory.slots.len() {
            let Some(stack) = inventory.slots[slot].stack.clone() else { continue };
            let Some(address) = packages.address(&stack) else { continue };
            for (&pos, port) in ports.iter_mut() {
                if !match_address(&address, &port.address) {
                    continue;
                }
                let mut into = port.inventory(pos);
                let left = into.insert_stacked(&stack, false);
                port.slots = into.slots.into_iter().map(|s| s.stack).collect();
                if left < stack.count {
                    note(&mut out.ports_changed, pos);
                }
                if left > 0 {
                    continue;
                }
                inventory.slots[slot].stack = None;
                note_index(&mut out.carriages_changed, index);
                if port.loaded {
                    note(&mut out.particles, pos);
                } else {
                    port.primed = true;
                    note(&mut out.primed, pos);
                }
                break;
            }
        }
    }
    out
}

/// `insertItemStacked(carriageInventory, stack, false)`: how many are left over. A Creative Crate's slots are put back after
/// (what went into its void slot is gone, as `BottomlessItemHandler` voids it).
fn insert(index: usize, inventory: &mut Inventory, stack: &Stack, bottomless: &dyn Fn(usize, Pos) -> bool) -> i32 {
    let before = inventory.slots.clone();
    let left = inventory.insert_stacked(stack, false);
    for (i, old) in before.into_iter().enumerate() {
        if bottomless(index, old.block) {
            inventory.slots[i] = old;
        }
    }
    left
}

/// `StationBlockEntity.attachPackagePort` for a port that may already be known: a new port takes the Postbox's address;
/// a known one keeps its record, and if it is `primed` its offline buffer is what the Postbox takes (`restoreOfflineBuffer`).
/// Returns the slots the Postbox must be given, if any, and clears `primed`.
pub fn restore_offline_buffer(port: &mut Port) -> Option<Vec<Option<Stack>>> {
    if !port.primed {
        return None;
    }
    port.primed = false;
    Some(port.slots.clone())
}

/// `PackagePortTargetSelectionHandler.validateDiff` for a Postbox (a Postbox may be placed below the target):
/// `Some(reason)` when the placed block is too far from the station's centre (`package_port.too_far`).
pub fn validate_target(target_centre: [f64; 3], placed: Pos) -> Option<&'static str> {
    let source = [f64::from(placed[0]) + 0.5, f64::from(placed[1]), f64::from(placed[2]) + 0.5];
    let d = [target_centre[0] - source[0], target_centre[1] - source[1], target_centre[2] - source[2]];
    ((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() > PORT_RANGE).then_some("package_port.too_far")
}

/// `PackagePortPlacementPacket.handle`'s own range check: the target within `packagePortRange + 2` of the port's bottom
/// centre (`closerThan`).
pub fn server_accepts(target_centre: [f64; 3], port: Pos) -> bool {
    let source = [f64::from(port[0]) + 0.5, f64::from(port[1]), f64::from(port[2]) + 0.5];
    let d = [target_centre[0] - source[0], target_centre[1] - source[1], target_centre[2] - source[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]) < (PORT_RANGE + 2.0) * (PORT_RANGE + 2.0)
}

/// English for the errors (`create.package_port.too_far`): Create's lang file.
pub fn error_text(key: &str) -> &'static str {
    match key {
        "package_port.too_far" => "Too far away",
        "package_port.cannot_reach_down" => "Cannot connect downward",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAR: Pos = [0, 0, 0];
    const CRATE: Pos = [9, 9, 9];

    fn package(id: &str) -> Stack {
        Stack { key: id.into(), count: 1, max: 1 }
    }

    /// A key is `<style>@<address>`; `plain` is not a package.
    fn packages() -> impl Fn(&Stack) -> Option<String> {
        |s: &Stack| s.key.strip_prefix("box@").map(str::to_owned)
    }

    fn inventory(slots: &[Option<&str>]) -> Inventory {
        Inventory { slots: slots.iter().enumerate().map(|(i, s)| Slot { block: CAR, index: i, stack: s.map(package) }).collect() }
    }

    fn port(address: &str, slots: &[Option<&str>], loaded: bool) -> Port {
        let mut all: Vec<Option<Stack>> = slots.iter().map(|s| s.map(package)).collect();
        all.resize(PORT_SLOTS, None);
        Port::new(address, all, loaded, false)
    }

    fn keys(slots: &[Option<Stack>]) -> Vec<String> {
        slots.iter().flatten().map(|s| s.key.clone()).collect()
    }

    fn never(_: usize, _: Pos) -> bool {
        false
    }

    #[test]
    fn a_package_for_another_address_is_taken_from_the_port() {
        let mut train = [inventory(&[None, None])];
        let mut ports = BTreeMap::from([([5, 0, 0], port("Home", &[Some("box@Farm")], true))]);
        let out = run_mail_transfer(&mut train, &mut ports, &packages(), &never);
        assert_eq!(keys(&ports[&[5, 0, 0]].slots), Vec::<String>::new());
        assert_eq!(keys(&train[0].slots.iter().map(|s| s.stack.clone()).collect::<Vec<_>>()), vec!["box@Farm"]);
        assert_eq!(out.carriages_changed, vec![0]);
        assert_eq!(out.particles, vec![[5, 0, 0]]);
        assert!(out.primed.is_empty());
    }

    #[test]
    fn a_package_for_the_ports_own_address_stays_there() {
        let mut train = [inventory(&[None])];
        let mut ports = BTreeMap::from([([5, 0, 0], port("Home", &[Some("box@Home")], true))]);
        let out = run_mail_transfer(&mut train, &mut ports, &packages(), &never);
        assert_eq!(keys(&ports[&[5, 0, 0]].slots), vec!["box@Home"]);
        assert_eq!(out, Outcome::default());
    }

    #[test]
    fn what_is_not_a_package_is_left_alone() {
        let mut train = [inventory(&[None])];
        let mut ports = BTreeMap::from([([5, 0, 0], port("Home", &[Some("stone")], true))]);
        let out = run_mail_transfer(&mut train, &mut ports, &packages(), &never);
        assert_eq!(keys(&ports[&[5, 0, 0]].slots), vec!["stone"]);
        assert!(train[0].slots[0].stack.is_none());
        assert_eq!(out, Outcome::default());
    }

    #[test]
    fn a_carried_package_goes_to_the_port_that_matches_its_address() {
        let mut train = [inventory(&[Some("box@Home"), Some("stone")])];
        let mut ports = BTreeMap::from([
            ([1, 0, 0], port("Farm", &[], true)),
            ([2, 0, 0], port("Home", &[], true)),
            ([3, 0, 0], port("Home", &[], true)),
        ]);
        let out = run_mail_transfer(&mut train, &mut ports, &packages(), &never);
        // The first port that matches takes it, and no other (`break`).
        assert_eq!(keys(&ports[&[2, 0, 0]].slots), vec!["box@Home"]);
        assert!(ports[&[1, 0, 0]].slots.iter().all(Option::is_none));
        assert!(ports[&[3, 0, 0]].slots.iter().all(Option::is_none));
        assert!(train[0].slots[0].stack.is_none());
        assert_eq!(train[0].slots[1].stack.as_ref().map(|s| s.key.as_str()), Some("stone"));
        assert_eq!(out.ports_changed, vec![[2, 0, 0]]);
    }

    #[test]
    fn a_wildcard_address_matches_every_port() {
        let mut train = [inventory(&[Some("box@*")])];
        let mut ports = BTreeMap::from([([1, 0, 0], port("Farm", &[], true))]);
        run_mail_transfer(&mut train, &mut ports, &packages(), &never);
        assert_eq!(keys(&ports[&[1, 0, 0]].slots), vec!["box@*"]);
    }

    #[test]
    fn a_full_port_keeps_the_package_on_the_train() {
        let mut train = [inventory(&[Some("box@Home")])];
        let full: Vec<Option<&str>> = vec![Some("box@Other"); PORT_SLOTS];
        let mut ports = BTreeMap::from([([1, 0, 0], port("Home", &full, true))]);
        let out = run_mail_transfer(&mut train, &mut ports, &packages(), &never);
        assert!(train[0].slots[0].stack.is_some());
        assert!(out.carriages_changed.is_empty() && out.ports_changed.is_empty());
    }

    #[test]
    fn a_full_train_leaves_the_package_in_the_port() {
        let mut train = [inventory(&[Some("box@Farm")])];
        let mut ports = BTreeMap::from([([1, 0, 0], port("Home", &[Some("box@Farm")], true))]);
        let out = run_mail_transfer(&mut train, &mut ports, &packages(), &never);
        assert_eq!(keys(&ports[&[1, 0, 0]].slots), vec!["box@Farm"]);
        assert_eq!(out, Outcome::default());
    }

    #[test]
    fn an_offline_port_is_primed_by_what_moves() {
        let mut train = [inventory(&[Some("box@Home"), None])];
        let mut ports = BTreeMap::from([([1, 0, 0], port("Home", &[Some("box@Farm")], false))]);
        let out = run_mail_transfer(&mut train, &mut ports, &packages(), &never);
        let p = &ports[&[1, 0, 0]];
        // The carried package went in and the farm one came out.
        assert_eq!(keys(&p.slots), vec!["box@Home"]);
        assert!(p.primed);
        assert_eq!(out.primed, vec![[1, 0, 0]]);
        assert!(out.particles.is_empty());
    }

    #[test]
    fn each_carriage_is_visited() {
        let mut train = [inventory(&[None]), inventory(&[Some("box@Home")])];
        let mut ports = BTreeMap::from([([1, 0, 0], port("Home", &[Some("box@Farm")], true))]);
        let out = run_mail_transfer(&mut train, &mut ports, &packages(), &never);
        // The first carriage takes the farm package; the second delivers its own.
        assert_eq!(keys(&train[0].slots.iter().map(|s| s.stack.clone()).collect::<Vec<_>>()), vec!["box@Farm"]);
        assert_eq!(keys(&ports[&[1, 0, 0]].slots), vec!["box@Home"]);
        assert_eq!(out.carriages_changed, vec![0, 1]);
    }

    #[test]
    fn nothing_happens_without_ports() {
        let mut train = [inventory(&[Some("box@Home")])];
        let mut ports = BTreeMap::new();
        assert_eq!(run_mail_transfer(&mut train, &mut ports, &packages(), &never), Outcome::default());
        assert!(train[0].slots[0].stack.is_some());
    }

    #[test]
    fn a_creative_crate_on_the_train_swallows_what_it_is_given() {
        // The crate's void slot comes first among the empty slots: the package is lost, as in Create.
        let mut train = [Inventory {
            slots: vec![
                Slot { block: CRATE, index: 0, stack: None },
                Slot { block: CRATE, index: 1, stack: None },
                Slot { block: CAR, index: 0, stack: None },
            ],
        }];
        let mut ports = BTreeMap::from([([1, 0, 0], port("Home", &[Some("box@Farm")], true))]);
        run_mail_transfer(&mut train, &mut ports, &packages(), &|_, p| p == CRATE);
        assert!(ports[&[1, 0, 0]].slots.iter().all(Option::is_none));
        assert!(train[0].slots.iter().all(|s| s.stack.is_none()));
    }

    #[test]
    fn a_primed_port_gives_its_buffer_to_the_postbox_once() {
        let mut p = port("Home", &[Some("box@Farm")], false);
        assert_eq!(restore_offline_buffer(&mut p), None);
        p.primed = true;
        let slots = restore_offline_buffer(&mut p).expect("restored");
        assert_eq!(keys(&slots), vec!["box@Farm"]);
        assert!(!p.primed);
        assert_eq!(restore_offline_buffer(&mut p), None);
    }

    #[test]
    fn a_postbox_must_be_within_five_blocks_of_the_station() {
        let station = [10.5, 64.5, 10.5];
        assert_eq!(validate_target(station, [10, 64, 12]), None);
        assert_eq!(validate_target(station, [10, 64, 17]), Some("package_port.too_far"));
        assert!(server_accepts(station, [10, 64, 16]));
        assert!(!server_accepts(station, [10, 64, 20]));
    }
}
