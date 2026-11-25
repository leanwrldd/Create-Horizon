//! Fluid in a Ponder scene's Fluid Tanks: the storyboards fill and drain a tank through its fluid
//! handler (`ifh.fill(stack, EXECUTE)`, `ifh.drain(amount, EXECUTE)`), through `getTankInventory()`
//! and, for a Creative Fluid Tank, `setContainedFluid`. All of them act on the tank's controller
//! (`FluidTankBlockEntity.handlerForCapability`), whichever block of the tank the scene names.
//!
//! The tank blocks come from the scene's structure file (`ponder_assets.tank_data`), which saves each
//! one's `Controller`, `Size`, `Height` and content; the fill and drain rules are those of
//! [`super::fluid_tank::Part`].

use std::collections::BTreeMap;

use super::fluid_tank::{set_tank_size, Part, Pos, CAPACITY};

/// One tank block as the scene file saves it.
#[derive(Clone, Debug, PartialEq)]
pub struct Saved {
    pub pos: Pos,
    pub controller: Pos,
    /// `Size` and `Height` of the controller; 0 where the file has none.
    pub width: i32,
    pub height: i32,
    pub window: bool,
    /// The fluid held, empty for none, and its amount in mB.
    pub fluid: String,
    pub amount: i32,
}

/// What a storyboard instruction does to a tank.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    /// `IFluidHandler.fill(stack, EXECUTE)`.
    Fill { fluid: String, amount: i32 },
    /// `IFluidHandler.drain(amount, EXECUTE)`.
    Drain { amount: i32 },
    /// `CreativeSmartFluidTank.setContainedFluid(stack)`.
    Set { fluid: String },
}

impl Op {
    /// The operation from the generated row's `kind`, `fluid` and `amount` (`ponder_storyboards.fluid_op`).
    pub fn from_row(kind: &str, fluid: &str, amount: i32) -> Option<Self> {
        match kind {
            "fill" => Some(Op::Fill { fluid: fluid.to_owned(), amount }),
            "drain" => Some(Op::Drain { amount }),
            "set" => Some(Op::Set { fluid: fluid.to_owned() }),
            _ => None,
        }
    }
}

/// The tank blocks of a scene as `Part`s: the controller carries the content, and its capacity is
/// the whole tank's (`applyFluidTankSize`). `creative` says which blocks are Creative Fluid Tanks.
pub fn parts(saved: &[Saved], creative: impl Fn(Pos) -> bool) -> BTreeMap<Pos, Part> {
    saved
        .iter()
        .map(|block| {
            let mut part = Part::new(block.pos, creative(block.pos));
            part.controller = block.controller;
            part.width = block.width.max(1);
            part.height = block.height.max(1);
            part.window = block.window;
            part.update_connectivity = false;
            if part.is_controller(block.pos) {
                let blocks = part.width * part.width * part.height;
                set_tank_size(&mut part, blocks);
                if !block.fluid.is_empty() && block.amount > 0 {
                    part.fluid = Some(block.fluid.clone());
                    part.amount = block.amount.min(part.capacity);
                }
            }
            (block.pos, part)
        })
        .collect()
}

/// Applies `op` to the tank the block at `at` belongs to. A position that isn't a tank, or whose
/// controller isn't in the scene, is left alone.
pub fn apply(parts: &mut BTreeMap<Pos, Part>, at: Pos, op: &Op) {
    let Some(controller) = parts.get(&at).map(|part| part.controller) else {
        return;
    };
    let Some(part) = parts.get_mut(&controller) else {
        return;
    };
    match op {
        Op::Fill { fluid, amount } => {
            part.fill(fluid, *amount);
        }
        Op::Drain { amount } => {
            part.drain(*amount);
        }
        Op::Set { fluid } => part.set_contained((!fluid.is_empty()).then(|| fluid.clone())),
    }
}

/// The capacity of a tank `width` wide and `height` tall.
pub fn capacity(width: i32, height: i32) -> i32 {
    width * width * height * CAPACITY
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(pos: Pos, controller: Pos, width: i32, height: i32) -> Saved {
        Saved {
            pos,
            controller,
            width,
            height,
            window: true,
            fluid: String::new(),
            amount: 0,
        }
    }

    /// A 2 wide, 2 tall tank: eight blocks round the controller at the origin.
    fn tank() -> Vec<Saved> {
        let mut blocks = Vec::new();
        for y in 0..2 {
            for x in 0..2 {
                for z in 0..2 {
                    let pos = [x, y, z];
                    blocks.push(if pos == [0, 0, 0] { block(pos, pos, 2, 2) } else { block(pos, [0, 0, 0], 0, 0) });
                }
            }
        }
        blocks
    }

    fn fill(fluid: &str, amount: i32) -> Op {
        Op::Fill { fluid: fluid.into(), amount }
    }

    #[test]
    fn the_controller_holds_the_fluid_whichever_block_is_named() {
        let mut tanks = parts(&tank(), |_| false);
        apply(&mut tanks, [1, 1, 1], &fill("create:chocolate", 1000));
        assert_eq!(tanks[&[0, 0, 0]].amount, 1000);
        assert_eq!(tanks[&[0, 0, 0]].fluid.as_deref(), Some("create:chocolate"));
        assert_eq!(tanks[&[1, 1, 1]].amount, 0);
        apply(&mut tanks, [0, 1, 0], &Op::Drain { amount: 400 });
        assert_eq!(tanks[&[0, 0, 0]].amount, 600);
    }

    #[test]
    fn a_tank_is_as_big_as_all_its_blocks_and_stops_taking_fluid_when_full() {
        let mut tanks = parts(&tank(), |_| false);
        assert_eq!(tanks[&[0, 0, 0]].capacity, capacity(2, 2));
        assert_eq!(capacity(2, 2), 64_000);
        apply(&mut tanks, [0, 0, 0], &fill("minecraft:water", 63_000));
        apply(&mut tanks, [0, 0, 0], &fill("minecraft:water", 5_000));
        assert_eq!(tanks[&[0, 0, 0]].amount, 64_000);
    }

    #[test]
    fn another_fluid_is_refused_and_an_emptied_tank_forgets_its_fluid() {
        let mut tanks = parts(&tank(), |_| false);
        apply(&mut tanks, [0, 0, 0], &fill("minecraft:water", 1000));
        apply(&mut tanks, [0, 0, 0], &fill("minecraft:lava", 1000));
        assert_eq!(tanks[&[0, 0, 0]].amount, 1000);
        assert_eq!(tanks[&[0, 0, 0]].fluid.as_deref(), Some("minecraft:water"));
        apply(&mut tanks, [0, 0, 0], &Op::Drain { amount: 5000 });
        assert_eq!(tanks[&[0, 0, 0]].amount, 0);
        assert_eq!(tanks[&[0, 0, 0]].fluid, None);
        apply(&mut tanks, [0, 0, 0], &fill("minecraft:lava", 1000));
        assert_eq!(tanks[&[0, 0, 0]].fluid.as_deref(), Some("minecraft:lava"));
    }

    #[test]
    fn what_the_file_saved_is_the_starting_content_and_cannot_exceed_the_capacity() {
        let mut saved = vec![block([3, 0, 0], [3, 0, 0], 1, 1)];
        saved[0].fluid = "minecraft:lava".into();
        saved[0].amount = 20_000;
        let tanks = parts(&saved, |_| false);
        assert_eq!(tanks[&[3, 0, 0]].amount, CAPACITY);
        assert_eq!(tanks[&[3, 0, 0]].fluid.as_deref(), Some("minecraft:lava"));
    }

    #[test]
    fn a_creative_tank_is_always_full_of_what_it_was_set_to_and_gives_without_losing_any() {
        let saved = vec![block([0, 0, 0], [0, 0, 0], 1, 1)];
        let mut tanks = parts(&saved, |_| true);
        apply(&mut tanks, [0, 0, 0], &Op::Set { fluid: "minecraft:lava".into() });
        assert_eq!(tanks[&[0, 0, 0]].amount_held(), CAPACITY);
        apply(&mut tanks, [0, 0, 0], &Op::Drain { amount: 3000 });
        assert_eq!(tanks[&[0, 0, 0]].amount_held(), CAPACITY);
        apply(&mut tanks, [0, 0, 0], &Op::Set { fluid: String::new() });
        assert_eq!(tanks[&[0, 0, 0]].amount_held(), 0);
    }

    #[test]
    fn a_position_that_is_no_tank_is_ignored() {
        let mut tanks = parts(&tank(), |_| false);
        apply(&mut tanks, [9, 9, 9], &fill("minecraft:water", 1000));
        assert!(tanks.values().all(|part| part.amount == 0));
    }

    #[test]
    fn generated_rows_become_operations() {
        assert_eq!(Op::from_row("fill", "create:honey", 250), Some(fill("create:honey", 250)));
        assert_eq!(Op::from_row("drain", "", 8000), Some(Op::Drain { amount: 8000 }));
        assert_eq!(Op::from_row("set", "minecraft:lava", 0), Some(Op::Set { fluid: "minecraft:lava".into() }));
        assert_eq!(Op::from_row("pour", "", 1), None);
    }
}

/// A block with fluid tanks that is not a Fluid Tank: what a scene's `fill` and `drain` reach through its capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// An Item Drain (`SmartFluidTankBehaviour.single(this, 1500)`) or a Spout (`single(this, 1000)`): one tank.
    Single(i32),
    /// A Basin: two input tanks and two output tanks of 1000 (`SmartFluidTankBehaviour(INPUT | OUTPUT, this, 2, 1000)`).
    Basin,
}

impl Kind {
    /// The kind of a block id, `None` for any other.
    pub fn of(block: &str) -> Option<Self> {
        match block {
            "create:item_drain" => Some(Kind::Single(1500)),
            "create:spout" => Some(Kind::Single(1000)),
            "create:basin" => Some(Kind::Basin),
            _ => None,
        }
    }
}

/// A tank's fluid and amount.
pub type Tank = (Option<String>, i32);

/// The tanks of one such block as a scene left them.
#[derive(Clone, Debug, PartialEq)]
pub struct Vessel {
    pub kind: Kind,
    pub tanks: Vec<Tank>,
}

impl Vessel {
    pub fn new(kind: Kind) -> Self {
        let count = match kind {
            Kind::Single(_) => 1,
            Kind::Basin => 4,
        };
        Vessel { kind, tanks: vec![(None, 0); count] }
    }

    const TANK: i32 = 1000;

    fn capacity(&self, index: usize) -> i32 {
        match self.kind {
            Kind::Single(capacity) => capacity,
            Kind::Basin => {
                let _ = index;
                Self::TANK
            }
        }
    }

    /// The tanks a fill goes to: all of a single's, the input tanks of a Basin (its output tanks forbid insertion).
    fn inputs(&self) -> std::ops::Range<usize> {
        match self.kind {
            Kind::Single(_) => 0..1,
            Kind::Basin => 0..2,
        }
    }

    /// The tanks a drain takes from: the output tanks of a Basin (its input tanks forbid extraction).
    fn outputs(&self) -> std::ops::Range<usize> {
        match self.kind {
            Kind::Single(_) => 0..1,
            Kind::Basin => 2..4,
        }
    }

    /// `IFluidHandler.fill(stack, EXECUTE)`: into a tank that already holds the fluid, else the first with room, then
    /// on to the next while some is left (`SmartFluidTankBehaviour`'s handler); how much went in.
    pub fn fill(&mut self, fluid: &str, amount: i32) -> i32 {
        let mut left = amount.max(0);
        let range = self.inputs();
        let holding = range.clone().filter(|&i| self.tanks[i].0.as_deref() == Some(fluid) && self.tanks[i].1 > 0);
        let rest = range.clone().filter(|&i| self.tanks[i].1 == 0);
        let order: Vec<usize> = holding.chain(rest).collect();
        for index in order {
            if left == 0 {
                break;
            }
            let room = self.capacity(index) - self.tanks[index].1;
            let put = left.min(room.max(0));
            if put > 0 {
                self.tanks[index].0 = Some(fluid.to_owned());
                self.tanks[index].1 += put;
                left -= put;
            }
        }
        amount.max(0) - left
    }

    /// `IFluidHandler.drain(amount, EXECUTE)`: from the first tank with something in it; how much came out.
    pub fn drain(&mut self, amount: i32) -> i32 {
        for index in self.outputs() {
            if self.tanks[index].1 > 0 {
                let taken = amount.max(0).min(self.tanks[index].1);
                self.tanks[index].1 -= taken;
                if self.tanks[index].1 == 0 {
                    self.tanks[index].0 = None;
                }
                return taken;
            }
        }
        0
    }

    /// What a scene's instruction does (`CreativeSmartFluidTank.setContainedFluid` is for a Creative Fluid Tank only).
    pub fn apply(&mut self, op: &Op) {
        match op {
            Op::Fill { fluid, amount } => {
                self.fill(fluid, *amount);
            }
            Op::Drain { amount } => {
                self.drain(*amount);
            }
            Op::Set { .. } => {}
        }
    }
}

#[cfg(test)]
mod vessel_tests {
    use super::*;

    fn fill(fluid: &str, amount: i32) -> Op {
        Op::Fill { fluid: fluid.into(), amount }
    }

    #[test]
    fn an_item_drain_holds_1500_and_gives_back_what_it_holds() {
        let mut drain = Vessel::new(Kind::of("create:item_drain").unwrap());
        drain.apply(&fill("minecraft:water", 1800));
        assert_eq!(drain.tanks, vec![(Some("minecraft:water".to_owned()), 1500)]);
        drain.apply(&Op::Drain { amount: 500 });
        assert_eq!(drain.tanks[0].1, 1000);
        drain.apply(&Op::Drain { amount: 5000 });
        assert_eq!(drain.tanks, vec![(None, 0)]);
    }

    #[test]
    fn a_spout_refuses_a_second_fluid_while_it_holds_one() {
        let mut spout = Vessel::new(Kind::of("create:spout").unwrap());
        assert_eq!(spout.fill("create:chocolate", 1000), 1000);
        assert_eq!(spout.fill("minecraft:lava", 1000), 0);
        assert_eq!(spout.tanks[0], (Some("create:chocolate".to_owned()), 1000));
    }

    #[test]
    fn a_basin_fills_its_input_tanks_and_the_second_fluid_takes_the_other() {
        let mut basin = Vessel::new(Kind::Basin);
        basin.apply(&fill("create:chocolate", 600));
        basin.apply(&fill("minecraft:milk", 1000));
        basin.apply(&fill("create:chocolate", 700));
        assert_eq!(basin.tanks[0], (Some("create:chocolate".to_owned()), 1000));
        assert_eq!(basin.tanks[1], (Some("minecraft:milk".to_owned()), 1000));
        // Both are full of something else now: the rest has nowhere to go.
        assert_eq!(basin.fill("minecraft:water", 100), 0);
        // The output tanks are empty and nothing drains from the inputs.
        basin.apply(&Op::Drain { amount: 500 });
        assert_eq!(basin.tanks[0].1, 1000);
        assert_eq!(basin.tanks[2], (None, 0));
    }

    #[test]
    fn only_these_blocks_are_vessels() {
        assert_eq!(Kind::of("create:basin"), Some(Kind::Basin));
        assert_eq!(Kind::of("create:fluid_tank"), None);
    }
}
