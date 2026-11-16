//! What a Brass Tunnel does with the stack a belt hands it: which outputs it may use, the
//! distribution modes (split, forced split, round robin, forced round robin, prefer nearest,
//! randomize, synchronize inputs) and the countdown between taking a stack and giving it out.
//! No game types: the glue says what each output answers when offered items.
//!
//! Ported from Create 6.0.10 (MIT, the Creators of Create): `BrassTunnelBlockEntity` (`tick`,
//! `distribute`, `gatherValidOutputs`, `addValidOutputsOf`, `SelectionMode`) and
//! `CLogistics.brassTunnelTimer`.

use crate::{Dir, Pos};

use super::tunnel::HORIZONTALS;

/// `CLogistics.brassTunnelTimer`: ticks a brass tunnel waits between distributions.
pub const TIMER: i32 = 10;

/// `BrassTunnelBlockEntity.SelectionMode`, in Create's order (the saved `ScrollValue`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Split,
    ForcedSplit,
    RoundRobin,
    ForcedRoundRobin,
    PreferNearest,
    Randomize,
    Synchronize,
}

impl Mode {
    pub const ALL: [Mode; 7] = [
        Mode::Split,
        Mode::ForcedSplit,
        Mode::RoundRobin,
        Mode::ForcedRoundRobin,
        Mode::PreferNearest,
        Mode::Randomize,
        Mode::Synchronize,
    ];

    pub const fn ordinal(self) -> i32 {
        self as i32
    }

    /// `ScrollOptionBehaviour` clamps a saved value into range.
    pub fn from_ordinal(ordinal: i32) -> Self {
        Self::ALL[ordinal.clamp(0, Self::ALL.len() as i32 - 1) as usize]
    }

    /// The next one, back to the first after the last (a click's stand-in for the scroll).
    pub fn next(self) -> Self {
        Self::from_ordinal((self.ordinal() + 1) % Self::ALL.len() as i32)
    }

    /// `create.tunnel.selection_mode.*`.
    pub const fn label(self) -> &'static str {
        match self {
            Mode::Split => "Split",
            Mode::ForcedSplit => "Forced Split",
            Mode::RoundRobin => "Round Robin",
            Mode::ForcedRoundRobin => "Forced Round Robin",
            Mode::PreferNearest => "Prefer Nearest",
            Mode::Randomize => "Randomize",
            Mode::Synchronize => "Synchronize Inputs",
        }
    }

    const fn force(self) -> bool {
        matches!(self, Mode::ForcedRoundRobin | Mode::ForcedSplit)
    }

    const fn split(self) -> bool {
        matches!(self, Mode::ForcedSplit | Mode::Split)
    }

    const fn robin(self) -> bool {
        matches!(self, Mode::ForcedRoundRobin | Mode::RoundRobin)
    }
}

/// One way out: a tunnel of the group and the side it opens on (`Pair<BlockPos, Direction>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Output {
    pub tunnel: Pos,
    pub side: Dir,
}

/// What an output answers when offered items (`insertIntoTunnel`). The glue holds the stack: the
/// rules only count.
pub trait Sink {
    /// Offers `count` items of the stack to `output`. `None` when it refuses (its filter, no input,
    /// no room at all); else the number it did not take (0 when it took them all). With
    /// `simulate` nothing is given.
    fn insert(&mut self, output: &Output, count: u32, simulate: bool) -> Option<u32>;
}

/// What `distribute` did to the stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Distributed {
    /// How many items of the stack are left (0 when it all went out).
    pub left: u32,
    /// `previousOutputIndex` afterwards.
    pub previous_index: usize,
}

/// `BrassTunnelBlockEntity.distribute`: gives out `count` items among `targets`. `None` when a
/// forced mode met an output that couldn't take its share, so nothing moved and the stack waits.
/// `random` picks the first output for `Randomize` (`level.random.nextInt`).
pub fn distribute(
    targets: &[Output],
    count: u32,
    mode: Mode,
    previous_index: usize,
    random: impl FnOnce(usize) -> usize,
    sink: &mut dyn Sink,
) -> Option<Distributed> {
    let amount = targets.len();
    if amount == 0 {
        return None;
    }
    let force = mode.force();
    let split = mode.split();
    let robin = mode.robin();

    let mut index_start = previous_index % amount;
    if mode == Mode::Randomize {
        index_start = random(amount);
    }
    if matches!(mode, Mode::PreferNearest | Mode::Synchronize) {
        index_start = 0;
    }

    // `distributed`: what each output is to be given, in the order it was first picked.
    let mut distributed: Vec<(usize, u32)> = Vec::new();
    let mut full = vec![false; amount];
    let mut to_distribute = count;
    for distribute_again in [true, false] {
        let mut this_cycle = 0;
        let mut remaining_outputs = amount as u32;
        let mut leftovers = 0;
        for simulate in [true, false] {
            if remaining_outputs == 0 {
                break;
            }
            leftovers = 0;
            let mut index = index_start;
            let stack_size = to_distribute;
            let split_stack_size = stack_size / remaining_outputs;
            let mut split_remainder = stack_size % remaining_outputs;
            let mut visited = 0;

            this_cycle = to_distribute;
            if !(force || split) && simulate {
                continue;
            }

            while visited < amount {
                let at = index;
                index = (index + 1) % amount;
                visited += 1;

                if full[at] {
                    if split && simulate {
                        remaining_outputs = remaining_outputs.saturating_sub(1);
                    }
                    continue;
                }

                let n = if split {
                    split_stack_size + u32::from(split_remainder > 0)
                } else {
                    stack_size
                };
                let mut to_output = n;
                // Grow by what it was already given, to find out whether the target is full
                // even after a successful transfer.
                let earlier = distributed.iter().find(|(i, _)| *i == at).map(|(_, c)| *c);
                if let Some(earlier) = earlier {
                    to_output += earlier;
                }
                let remainder = sink.insert(&targets[at], to_output, true);

                let refused = match remainder {
                    None => true,
                    Some(rest) => rest == if earlier.is_some() { n + 1 } else { n },
                };
                if refused {
                    if force {
                        return None;
                    }
                    if split && simulate {
                        remaining_outputs = remaining_outputs.saturating_sub(1);
                    }
                    if !simulate {
                        full[at] = true;
                    }
                    if robin {
                        break;
                    }
                    continue;
                }
                let rest = remainder.unwrap_or(0);
                if rest > 0 && !simulate {
                    full[at] = true;
                }

                if !simulate {
                    let given = to_output.saturating_sub(rest);
                    match distributed.iter_mut().find(|(i, _)| *i == at) {
                        Some(entry) => entry.1 = given,
                        None => distributed.push((at, given)),
                    }
                }

                leftovers += rest;
                this_cycle = this_cycle.saturating_sub(n);
                if this_cycle == 0 {
                    break;
                }
                split_remainder = split_remainder.saturating_sub(1);
                if !split {
                    break;
                }
            }
        }
        to_distribute = this_cycle + leftovers;
        if leftovers == 0 && distribute_again {
            break;
        }
        if !split {
            break;
        }
    }

    let mut failed = 0;
    for (at, given) in distributed {
        // `insertIntoTunnel(..., false).getCount()`: what the output gave back, or all of it.
        failed += sink.insert(&targets[at], given, false).unwrap_or(given);
    }
    Some(Distributed {
        left: to_distribute + failed,
        previous_index: (previous_index + 1) % amount,
    })
}

// --- the outputs of a group ---------------------------------------------------------------------

/// What stands past a tunnel's side, one block below it (`addValidOutputsOf`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Neighbour {
    /// A belt funnel pulling towards the tunnel: not an output.
    PullingFunnel,
    /// A block that takes items from a belt (`DirectBeltInputBehaviour`): whether it does from
    /// this side.
    Input(bool),
    /// No such block: items fly off only past the belt's end, and only where the block there
    /// isn't solid.
    NoInput { open: bool },
}

/// A tunnel of the group, and the belt below it.
#[derive(Clone, Debug)]
pub struct Member {
    /// `getMovementFacing` of the belt below it.
    pub movement: Dir,
    /// The belt below it is stopped.
    pub stopped: bool,
    /// Its `sides`.
    pub sides: Vec<Dir>,
}

/// `addValidOutputsOf`: the outputs of one tunnel, in the order Create tries them. The tunnel
/// the stack sits in (`origin`) puts its sides across the belt first and then the ones along it;
/// every other tunnel of the group is tried in compass order. `look` reads what is beyond a side.
pub fn valid_outputs(member: &Member, origin: bool, look: &mut dyn FnMut(Dir) -> Neighbour) -> Vec<Dir> {
    let mut out = Vec::new();
    for side_pass in [true, false] {
        if !origin && side_pass {
            continue;
        }
        for direction in HORIZONTALS {
            if direction == member.movement && member.stopped {
                continue;
            }
            if origin && side_pass == (direction.axis() == member.movement.axis()) {
                continue;
            }
            if direction == member.movement.opposite() {
                continue;
            }
            if !member.sides.contains(&direction) {
                continue;
            }
            match look(direction) {
                Neighbour::PullingFunnel => {}
                Neighbour::NoInput { open } => {
                    if direction == member.movement && open {
                        out.push(direction);
                    }
                }
                Neighbour::Input(accepts) => {
                    if accepts {
                        out.push(direction);
                    }
                }
            }
        }
    }
    out
}

/// What a tunnel's search for outputs found (`gatherValidOutputs`).
#[derive(Clone, Debug, Default)]
pub struct Gathered {
    pub outputs: Vec<Output>,
    /// Whether each tunnel of the group holds a stack (`syncSet`), the one asking first.
    pub holding: Vec<bool>,
    /// False when a chunk of the group isn't loaded.
    pub complete: bool,
}

/// What a tunnel needs of its surroundings in a tick.
pub trait Group: Sink {
    /// `beltBelow != null && beltBelow.getSpeed() != 0`.
    fn belt_moving(&self) -> bool;
    /// `gatherValidOutputs`, less the synchronise check. The tunnel's own stack is `holding` first.
    fn gather(&mut self) -> Gathered;
    /// `flapFilterEmpty`: the output's side has no filter.
    fn filter_empty(&self, output: &Output) -> bool;
    /// The tunnel of the output still stands (`level.getBlockEntity(tunnelPos) instanceof Brass`).
    fn exists(&self, output: &Output) -> bool;
}

/// The saved state of one Brass Tunnel's distribution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    pub pos: Pos,
    /// How many items of the stack it holds (`stackToDistribute`); 0 for none.
    pub count: u32,
    /// `stackEnteredFrom`.
    pub entered_from: Option<Dir>,
    /// `distributionProgress`: -1 looking for outputs, then counting down to 0.
    pub progress: i32,
    pub previous_index: usize,
    /// `newItemArrived`.
    pub new_item_arrived: bool,
    /// `syncedOutputActive`.
    pub synced_output_active: bool,
    /// `distributionTargets`: the outputs with a filter on their side first, then the others.
    pub targets: [Vec<Output>; 2],
}

impl State {
    pub fn new(pos: Pos) -> Self {
        Self {
            pos,
            count: 0,
            entered_from: None,
            progress: -1,
            previous_index: 0,
            new_item_arrived: false,
            synced_output_active: false,
            targets: [Vec::new(), Vec::new()],
        }
    }

    /// `setStackToDistribute`: a stack came in from `entered_from` (none: by hand).
    pub fn set_stack(&mut self, count: u32, entered_from: Option<Dir>) {
        self.count = count;
        self.entered_from = entered_from;
        self.progress = -1;
        if count > 0 {
            self.new_item_arrived = true;
        }
    }

    /// `canTakeItems`.
    pub fn can_take(&self) -> bool {
        self.count == 0 && !self.synced_output_active
    }
}

/// What one tick asks the glue to do besides the changes in the state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tick {
    /// Set `syncedOutputActive` of every tunnel of the group to this (the state's own is already).
    pub sync_group: Option<bool>,
    /// Clients are to be sent the state (`sendData`).
    pub send: bool,
    /// The stack changed (`notifyUpdate`).
    pub distributed: bool,
}

/// `BrassTunnelBlockEntity.tick`, after the belt below was found. Changes `state` and says what
/// else to do.
pub fn tick(
    state: &mut State,
    mode: Mode,
    timer: i32,
    group: &mut dyn Group,
    mut random: impl FnMut(usize) -> usize,
) -> Tick {
    let mut out = Tick::default();
    if state.progress > 0 {
        state.progress -= 1;
    }
    if !group.belt_moving() {
        return out;
    }
    if state.count == 0 && !state.synced_output_active {
        return out;
    }

    if state.progress == -1 {
        state.targets = [Vec::new(), Vec::new()];
        let gathered = group.gather();
        let usable = gathered.complete && !(mode == Mode::Synchronize && !state.synced_output_active);
        if mode == Mode::Synchronize {
            let all_empty = gathered.holding.iter().all(|h| !h);
            let all_full = gathered.holding.iter().all(|h| *h);
            if all_full || all_empty {
                state.synced_output_active = !all_empty;
                out.sync_group = Some(!all_empty);
            }
        }
        if !usable || state.count == 0 {
            return out;
        }
        for output in &gathered.outputs {
            if group.insert(output, state.count, true).is_none() {
                continue;
            }
            // `distributionTargets.get(!flapFilterEmpty)`: the first list has a filter.
            let list = usize::from(group.filter_empty(output));
            state.targets[list].push(*output);
        }
        if state.targets[0].is_empty() && state.targets[1].is_empty() {
            return out;
        }
        if state.new_item_arrived {
            state.new_item_arrived = false;
            state.progress = 2;
        } else {
            if mode != Mode::Synchronize || state.synced_output_active {
                state.progress = timer;
                out.send = true;
            }
            return out;
        }
    }

    if state.progress != 0 {
        return out;
    }

    for list in 0..2 {
        if state.count == 0 {
            continue;
        }
        let valid: Vec<Output> = state.targets[list]
            .iter()
            .filter(|o| !(o.tunnel == state.pos && Some(o.side) == state.entered_from))
            .filter(|o| group.exists(o))
            .copied()
            .collect();
        if let Some(done) = distribute(
            &valid,
            state.count,
            mode,
            state.previous_index,
            |n| random(n),
            group,
        ) {
            state.count = done.left;
            if state.count == 0 {
                state.entered_from = None;
            }
            state.previous_index = done.previous_index;
            out.distributed = true;
        }
        state.progress = -1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Outputs that take up to `room[i]` items each (`None`: refuse), remembering what was given.
    struct Rooms {
        room: Vec<Option<u32>>,
        given: Vec<u32>,
        offered: Vec<(usize, u32, bool)>,
    }

    impl Rooms {
        fn new(room: &[Option<u32>]) -> Self {
            Self { room: room.to_vec(), given: vec![0; room.len()], offered: Vec::new() }
        }
    }

    fn target(i: usize) -> Output {
        Output { tunnel: [i as i32, 0, 0], side: Dir::North }
    }

    fn targets(n: usize) -> Vec<Output> {
        (0..n).map(target).collect()
    }

    impl Sink for Rooms {
        fn insert(&mut self, output: &Output, count: u32, simulate: bool) -> Option<u32> {
            let i = output.tunnel[0] as usize;
            self.offered.push((i, count, simulate));
            let room = self.room[i]?;
            let free = room.saturating_sub(self.given[i]);
            let taken = count.min(free);
            if taken == 0 {
                return Some(count);
            }
            if !simulate {
                self.given[i] += taken;
            }
            Some(count - taken)
        }
    }

    fn no_random(_: usize) -> usize {
        panic!("not random");
    }

    #[test]
    fn modes_are_named_and_cycle() {
        assert_eq!(Mode::from_ordinal(0), Mode::Split);
        assert_eq!(Mode::from_ordinal(99), Mode::Synchronize);
        assert_eq!(Mode::from_ordinal(-4), Mode::Split);
        assert_eq!(Mode::Split.next(), Mode::ForcedSplit);
        assert_eq!(Mode::Synchronize.next(), Mode::Split);
        assert_eq!(Mode::ForcedRoundRobin.label(), "Forced Round Robin");
        let mut mode = Mode::Split;
        for _ in 0..7 {
            mode = mode.next();
        }
        assert_eq!(mode, Mode::Split);
    }

    #[test]
    fn split_shares_a_stack_out_evenly() {
        let mut rooms = Rooms::new(&[Some(64), Some(64), Some(64)]);
        let done = distribute(&targets(3), 7, Mode::Split, 0, no_random, &mut rooms).unwrap();
        // The remainder goes to the first outputs: 3, 2, 2.
        assert_eq!(rooms.given, vec![3, 2, 2]);
        assert_eq!(done.left, 0);
        assert_eq!(done.previous_index, 1);
    }

    #[test]
    fn split_starts_after_the_output_used_last() {
        let mut rooms = Rooms::new(&[Some(64), Some(64), Some(64)]);
        let done = distribute(&targets(3), 4, Mode::Split, 1, no_random, &mut rooms).unwrap();
        // Output 1 first: it gets the extra item.
        assert_eq!(rooms.given, vec![1, 2, 1]);
        assert_eq!(done.previous_index, 2);
    }

    #[test]
    fn split_skips_an_output_that_is_full_and_gives_its_share_to_the_others() {
        let mut rooms = Rooms::new(&[Some(0), Some(64), Some(64)]);
        let done = distribute(&targets(3), 8, Mode::Split, 0, no_random, &mut rooms).unwrap();
        assert_eq!(rooms.given, vec![0, 4, 4]);
        assert_eq!(done.left, 0);
    }

    #[test]
    fn split_keeps_what_the_outputs_could_not_take() {
        let mut rooms = Rooms::new(&[Some(1), Some(1)]);
        let done = distribute(&targets(2), 6, Mode::Split, 0, no_random, &mut rooms).unwrap();
        assert_eq!(rooms.given, vec![1, 1]);
        assert_eq!(done.left, 4);
    }

    #[test]
    fn forced_split_waits_rather_than_skip() {
        let mut rooms = Rooms::new(&[Some(0), Some(64)]);
        assert_eq!(distribute(&targets(2), 8, Mode::ForcedSplit, 0, no_random, &mut rooms), None);
        // Nothing was given while it waited.
        assert_eq!(rooms.given, vec![0, 0]);
        // With every output willing it splits as Split does.
        let mut rooms = Rooms::new(&[Some(64), Some(64)]);
        let done = distribute(&targets(2), 5, Mode::ForcedSplit, 0, no_random, &mut rooms).unwrap();
        assert_eq!(rooms.given, vec![3, 2]);
        assert_eq!(done.left, 0);
    }

    #[test]
    fn round_robin_keeps_the_stack_whole_and_moves_on() {
        let mut rooms = Rooms::new(&[Some(64), Some(64), Some(64)]);
        let mut at = 0;
        for _ in 0..4 {
            let done = distribute(&targets(3), 5, Mode::RoundRobin, at, no_random, &mut rooms).unwrap();
            assert_eq!(done.left, 0);
            at = done.previous_index;
        }
        // Four stacks of five went to outputs 0, 1, 2, 0.
        assert_eq!(rooms.given, vec![10, 5, 5]);
    }

    #[test]
    fn round_robin_gives_up_for_now_on_a_full_output_and_tries_the_next_one_next_time() {
        let mut rooms = Rooms::new(&[Some(0), Some(64), Some(64)]);
        // Its turn is the full output: this time nothing moves, and the turn passes.
        let done = distribute(&targets(3), 5, Mode::RoundRobin, 0, no_random, &mut rooms).unwrap();
        assert_eq!(rooms.given, vec![0, 0, 0]);
        assert_eq!(done.left, 5);
        assert_eq!(done.previous_index, 1);
        let done = distribute(&targets(3), 5, Mode::RoundRobin, done.previous_index, no_random, &mut rooms).unwrap();
        assert_eq!(rooms.given, vec![0, 5, 0]);
        assert_eq!(done.left, 0);
    }

    #[test]
    fn forced_round_robin_waits_for_its_turn() {
        let mut rooms = Rooms::new(&[Some(0), Some(64)]);
        assert_eq!(
            distribute(&targets(2), 5, Mode::ForcedRoundRobin, 0, no_random, &mut rooms),
            None
        );
        assert_eq!(rooms.given, vec![0, 0]);
        // Its turn is the next output.
        let done = distribute(&targets(2), 5, Mode::ForcedRoundRobin, 1, no_random, &mut rooms).unwrap();
        assert_eq!(rooms.given, vec![0, 5]);
        assert_eq!(done.previous_index, 0);
    }

    #[test]
    fn prefer_nearest_always_tries_the_first_output_first() {
        let mut rooms = Rooms::new(&[Some(64), Some(64)]);
        for at in 0..3 {
            distribute(&targets(2), 3, Mode::PreferNearest, at, no_random, &mut rooms).unwrap();
        }
        assert_eq!(rooms.given, vec![9, 0]);
        // A full first output hands on to the next.
        let mut rooms = Rooms::new(&[Some(0), Some(64)]);
        distribute(&targets(2), 3, Mode::PreferNearest, 0, no_random, &mut rooms).unwrap();
        assert_eq!(rooms.given, vec![0, 3]);
    }

    #[test]
    fn randomize_starts_at_the_output_picked() {
        let mut rooms = Rooms::new(&[Some(64), Some(64), Some(64)]);
        distribute(&targets(3), 3, Mode::Randomize, 0, |n| {
            assert_eq!(n, 3);
            2
        }, &mut rooms)
        .unwrap();
        assert_eq!(rooms.given, vec![0, 0, 3]);
    }

    #[test]
    fn no_outputs_means_nothing_happens() {
        let mut rooms = Rooms::new(&[]);
        assert_eq!(distribute(&[], 3, Mode::Split, 0, no_random, &mut rooms), None);
    }

    #[test]
    fn a_refusing_output_is_skipped_like_a_full_one() {
        let mut rooms = Rooms::new(&[None, Some(64)]);
        let done = distribute(&targets(2), 4, Mode::Split, 0, no_random, &mut rooms).unwrap();
        assert_eq!(rooms.given, vec![0, 4]);
        assert_eq!(done.left, 0);
    }

    fn member(movement: Dir, stopped: bool, sides: &[Dir]) -> Member {
        Member { movement, stopped, sides: sides.to_vec() }
    }

    #[test]
    fn the_stack_leaves_across_the_belt_before_along_it() {
        // A tunnel over a belt moving south, open west, east and at both ends.
        let m = member(Dir::South, false, &[Dir::North, Dir::South, Dir::West, Dir::East]);
        let outs = valid_outputs(&m, true, &mut |_| Neighbour::Input(true));
        // The sides first (west, east), then ahead (south); never back (north).
        assert_eq!(outs, vec![Dir::West, Dir::East, Dir::South]);
    }

    #[test]
    fn other_tunnels_of_the_group_are_tried_in_compass_order() {
        let m = member(Dir::South, false, &[Dir::North, Dir::South, Dir::West, Dir::East]);
        let outs = valid_outputs(&m, false, &mut |_| Neighbour::Input(true));
        assert_eq!(outs, vec![Dir::South, Dir::West, Dir::East]);
    }

    #[test]
    fn a_stopped_belt_has_no_way_forward() {
        let m = member(Dir::South, true, &[Dir::North, Dir::South, Dir::West]);
        let outs = valid_outputs(&m, true, &mut |_| Neighbour::Input(true));
        assert_eq!(outs, vec![Dir::West]);
    }

    #[test]
    fn a_side_must_take_items_from_that_side() {
        let m = member(Dir::South, false, &[Dir::North, Dir::South, Dir::West, Dir::East]);
        let outs = valid_outputs(&m, true, &mut |d| Neighbour::Input(d != Dir::West));
        assert_eq!(outs, vec![Dir::East, Dir::South]);
    }

    #[test]
    fn items_fly_off_the_end_of_the_belt_where_there_is_no_wall() {
        let m = member(Dir::South, false, &[Dir::North, Dir::South, Dir::West]);
        let open = valid_outputs(&m, true, &mut |_| Neighbour::NoInput { open: true });
        // Only the end of the belt is an exit; a side with nothing there is not.
        assert_eq!(open, vec![Dir::South]);
        let walled = valid_outputs(&m, true, &mut |_| Neighbour::NoInput { open: false });
        assert!(walled.is_empty());
    }

    #[test]
    fn a_funnel_pulling_into_the_tunnel_is_not_an_exit() {
        let m = member(Dir::South, false, &[Dir::North, Dir::South, Dir::West, Dir::East]);
        let outs = valid_outputs(&m, true, &mut |d| {
            if d == Dir::West { Neighbour::PullingFunnel } else { Neighbour::Input(true) }
        });
        assert_eq!(outs, vec![Dir::East, Dir::South]);
    }

    /// A group of one tunnel with fixed outputs and rooms.
    struct One {
        moving: bool,
        outputs: Vec<Output>,
        rooms: Rooms,
        filtered: Vec<bool>,
        holding: Vec<bool>,
        complete: bool,
    }

    impl Sink for One {
        fn insert(&mut self, output: &Output, count: u32, simulate: bool) -> Option<u32> {
            self.rooms.insert(output, count, simulate)
        }
    }

    impl Group for One {
        fn belt_moving(&self) -> bool {
            self.moving
        }
        fn gather(&mut self) -> Gathered {
            Gathered {
                outputs: self.outputs.clone(),
                holding: self.holding.clone(),
                complete: self.complete,
            }
        }
        fn filter_empty(&self, output: &Output) -> bool {
            !self.filtered[output.tunnel[0] as usize]
        }
        fn exists(&self, _: &Output) -> bool {
            true
        }
    }

    fn one(rooms: &[Option<u32>]) -> One {
        One {
            moving: true,
            outputs: targets(rooms.len()),
            rooms: Rooms::new(rooms),
            filtered: vec![false; rooms.len()],
            holding: vec![true],
            complete: true,
        }
    }

    fn run(state: &mut State, mode: Mode, group: &mut One, ticks: usize) -> Vec<Tick> {
        (0..ticks).map(|_| tick(state, mode, TIMER, group, |_| 0)).collect()
    }

    #[test]
    fn a_new_stack_is_given_out_two_ticks_after_it_arrived() {
        let mut state = State::new([9, 9, 9]);
        let mut group = one(&[Some(64), Some(64)]);
        state.set_stack(4, Some(Dir::North));
        // Tick 1 finds the outputs and starts the countdown at 2; tick 2 takes it to 1; tick 3 to 0.
        let ticks = run(&mut state, Mode::Split, &mut group, 2);
        assert!(ticks.iter().all(|t| !t.distributed));
        assert_eq!(state.progress, 1);
        let t = tick(&mut state, Mode::Split, TIMER, &mut group, |_| 0);
        assert!(t.distributed);
        assert_eq!(group.rooms.given, vec![2, 2]);
        assert_eq!(state.count, 0);
        assert_eq!(state.entered_from, None);
        assert_eq!(state.progress, -1);
    }

    #[test]
    fn a_stack_that_was_already_waiting_waits_the_timer() {
        let mut state = State::new([9, 9, 9]);
        let mut group = one(&[Some(64)]);
        state.set_stack(4, None);
        state.new_item_arrived = false;
        let first = tick(&mut state, Mode::Split, TIMER, &mut group, |_| 0);
        assert!(first.send);
        assert_eq!(state.progress, TIMER);
        for _ in 0..TIMER - 1 {
            let t = tick(&mut state, Mode::Split, TIMER, &mut group, |_| 0);
            assert!(!t.distributed);
        }
        let last = tick(&mut state, Mode::Split, TIMER, &mut group, |_| 0);
        assert!(last.distributed);
        assert_eq!(group.rooms.given, vec![4]);
    }

    #[test]
    fn a_stopped_belt_holds_the_stack() {
        let mut state = State::new([9, 9, 9]);
        let mut group = one(&[Some(64)]);
        group.moving = false;
        state.set_stack(4, None);
        run(&mut state, Mode::Split, &mut group, 10);
        assert_eq!(state.count, 4);
        assert_eq!(group.rooms.given, vec![0]);
    }

    #[test]
    fn with_nowhere_to_go_it_keeps_looking_every_tick() {
        let mut state = State::new([9, 9, 9]);
        // The only output takes nothing of this stack at all.
        let mut group = one(&[None]);
        state.set_stack(4, None);
        run(&mut state, Mode::Split, &mut group, 5);
        assert_eq!(state.progress, -1);
        assert_eq!(state.count, 4);
        // It starts taking it and the stack goes out after the countdown.
        group.rooms.room = vec![Some(64)];
        let ticks = run(&mut state, Mode::Split, &mut group, 3);
        assert!(ticks.last().unwrap().distributed);
        assert_eq!(group.rooms.given, vec![4]);
    }

    #[test]
    fn an_output_that_is_full_is_waited_for_and_tried_again() {
        let mut state = State::new([9, 9, 9]);
        let mut group = one(&[Some(0)]);
        state.set_stack(4, None);
        state.new_item_arrived = false;
        run(&mut state, Mode::Split, &mut group, 1);
        assert_eq!(state.progress, TIMER);
        // The timer runs out, nothing goes, and it looks again.
        run(&mut state, Mode::Split, &mut group, TIMER as usize);
        assert_eq!(state.count, 4);
        assert_eq!(state.progress, -1);
        group.rooms.room = vec![Some(64)];
        run(&mut state, Mode::Split, &mut group, TIMER as usize + 2);
        assert_eq!(group.rooms.given, vec![4]);
    }

    #[test]
    fn filtered_outputs_are_served_before_the_rest() {
        let mut state = State::new([9, 9, 9]);
        let mut group = one(&[Some(64), Some(64)]);
        group.filtered = vec![false, true];
        state.set_stack(3, None);
        run(&mut state, Mode::Split, &mut group, 3);
        // The filtered output (1) took the whole stack; nothing was left for the other.
        assert_eq!(group.rooms.given, vec![0, 3]);
        assert_eq!(state.count, 0);
    }

    #[test]
    fn a_stack_never_goes_back_where_it_came_from() {
        let mut state = State::new([0, 0, 0]);
        let mut group = one(&[Some(64), Some(64)]);
        // Output 0 is this tunnel's own north side, where the stack came in.
        group.outputs = vec![
            Output { tunnel: [0, 0, 0], side: Dir::North },
            Output { tunnel: [1, 0, 0], side: Dir::North },
        ];
        state.set_stack(4, Some(Dir::North));
        run(&mut state, Mode::Split, &mut group, 3);
        assert_eq!(group.rooms.given, vec![0, 4]);
    }

    #[test]
    fn the_distribution_round_robin_moves_between_stacks() {
        let mut state = State::new([9, 9, 9]);
        let mut group = one(&[Some(64), Some(64)]);
        for _ in 0..2 {
            state.set_stack(2, None);
            run(&mut state, Mode::RoundRobin, &mut group, 3);
        }
        assert_eq!(group.rooms.given, vec![2, 2]);
    }

    #[test]
    fn synchronize_waits_until_every_tunnel_holds_a_stack() {
        let mut state = State::new([9, 9, 9]);
        let mut group = one(&[Some(64)]);
        // Two tunnels in the group; the other holds nothing yet.
        group.holding = vec![true, false];
        state.set_stack(2, None);
        let ticks = run(&mut state, Mode::Synchronize, &mut group, 6);
        assert!(ticks.iter().all(|t| t.sync_group.is_none() && !t.distributed));
        assert!(!state.synced_output_active);
        assert_eq!(group.rooms.given, vec![0]);
        // The other fills: all full, so the group's outputs open.
        group.holding = vec![true, true];
        let t = tick(&mut state, Mode::Synchronize, TIMER, &mut group, |_| 0);
        assert_eq!(t.sync_group, Some(true));
        assert!(state.synced_output_active);
        // The next look finds the outputs; the countdown for a stack that arrived is 2 or the timer.
        run(&mut state, Mode::Synchronize, &mut group, 12);
        assert_eq!(group.rooms.given, vec![2]);
    }

    #[test]
    fn synchronize_closes_the_group_when_every_stack_is_gone() {
        let mut state = State::new([9, 9, 9]);
        let mut group = one(&[Some(64)]);
        state.synced_output_active = true;
        state.progress = -1;
        group.holding = vec![false, false];
        let t = tick(&mut state, Mode::Synchronize, TIMER, &mut group, |_| 0);
        assert_eq!(t.sync_group, Some(false));
        assert!(!state.synced_output_active);
    }

    #[test]
    fn a_tunnel_waiting_for_synchronisation_doesnt_take_items() {
        let mut state = State::new([9, 9, 9]);
        assert!(state.can_take());
        state.synced_output_active = true;
        assert!(!state.can_take());
        state.synced_output_active = false;
        state.set_stack(1, None);
        assert!(!state.can_take());
    }

    #[test]
    fn an_unloaded_chunk_in_the_group_pauses_distribution() {
        let mut state = State::new([9, 9, 9]);
        let mut group = one(&[Some(64)]);
        group.complete = false;
        state.set_stack(2, None);
        run(&mut state, Mode::Split, &mut group, 5);
        assert_eq!(state.count, 2);
        assert_eq!(group.rooms.given, vec![0]);
    }
}
