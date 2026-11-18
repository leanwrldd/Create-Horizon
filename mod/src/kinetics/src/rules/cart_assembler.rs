//! The Cart Assembler's rules: what it does to a minecart that passes (`CartAssemblerBlock`:
//! `CartAssemblerAction`, `getActionForCart`, `canAssembleTo`, `CartAssembleRailType`). Plain values.
//!
//! Only the rules are in this port: no block, item or entity is declared for the Cart Assembler,
//! the Minecart Contraption items, the Controller Rail or the Minecart Anchor, because a structure
//! can't follow and be drawn on a vanilla minecart in Horizon's mod API. What is missing is
//! written in the README ("Cart Assembler and minecart contraptions: what Horizon lacks"); these
//! rules are kept, tested, for the day it can.

/// `CartAssembleRailType`: the rail a Cart Assembler is made from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RailType {
    Regular,
    PoweredRail,
    DetectorRail,
    ActivatorRail,
    ControllerRail,
}

impl RailType {
    /// The rail block the assembler stands for (`CartAssembleRailType.getBlock`).
    pub const fn block(self) -> &'static str {
        match self {
            RailType::Regular => "minecraft:rail",
            RailType::PoweredRail => "minecraft:powered_rail",
            RailType::DetectorRail => "minecraft:detector_rail",
            RailType::ActivatorRail => "minecraft:activator_rail",
            RailType::ControllerRail => "create:controller_rail",
        }
    }
}

/// `CartAssemblerBlock.CartAssemblerAction`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Assemble,
    Disassemble,
    AssembleAccelerate,
    DisassembleBrake,
    AssembleAccelerateDirectional,
    Pass,
}

impl Action {
    pub const fn should_assemble(self) -> bool {
        matches!(self, Action::Assemble | Action::AssembleAccelerate | Action::AssembleAccelerateDirectional)
    }

    pub const fn should_disassemble(self) -> bool {
        matches!(self, Action::Disassemble | Action::DisassembleBrake)
    }
}

/// `CartAssemblerBlock.getActionForCart`: what an assembler of `rail` does to a passing cart,
/// `powered` being its redstone state and `has_passengers` whether the cart carries anyone.
pub fn action_for_cart(rail: RailType, powered: bool, has_passengers: bool) -> Action {
    match rail {
        RailType::ActivatorRail => if powered { Action::Disassemble } else { Action::Pass },
        RailType::ControllerRail => {
            if powered { Action::AssembleAccelerateDirectional } else { Action::DisassembleBrake }
        }
        RailType::DetectorRail => if has_passengers { Action::Disassemble } else { Action::AssembleAccelerate },
        RailType::PoweredRail => if powered { Action::AssembleAccelerate } else { Action::DisassembleBrake },
        RailType::Regular => if powered { Action::Assemble } else { Action::Disassemble },
    }
}

/// `CartAssemblerBlock.canAssembleTo`: a cart that can be ridden, or a furnace or chest cart.
pub fn can_assemble_to(cart_kind: &str, can_be_ridden: bool) -> bool {
    can_be_ridden || matches!(cart_kind, "minecraft:furnace_minecart" | "minecraft:chest_minecart")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_regular_assembler_follows_its_power() {
        assert_eq!(action_for_cart(RailType::Regular, true, false), Action::Assemble);
        assert_eq!(action_for_cart(RailType::Regular, false, false), Action::Disassemble);
    }

    #[test]
    fn a_powered_rail_assembler_also_holds_and_releases_the_cart() {
        assert_eq!(action_for_cart(RailType::PoweredRail, true, false), Action::AssembleAccelerate);
        assert_eq!(action_for_cart(RailType::PoweredRail, false, false), Action::DisassembleBrake);
        assert_eq!(action_for_cart(RailType::ControllerRail, true, false), Action::AssembleAccelerateDirectional);
        assert_eq!(action_for_cart(RailType::ControllerRail, false, false), Action::DisassembleBrake);
    }

    #[test]
    fn detector_and_activator_assemblers_read_the_cart_and_the_signal() {
        assert_eq!(action_for_cart(RailType::DetectorRail, false, false), Action::AssembleAccelerate);
        assert_eq!(action_for_cart(RailType::DetectorRail, false, true), Action::Disassemble);
        assert_eq!(action_for_cart(RailType::ActivatorRail, true, false), Action::Disassemble);
        assert_eq!(action_for_cart(RailType::ActivatorRail, false, false), Action::Pass);
    }

    #[test]
    fn only_some_carts_take_a_structure() {
        assert!(can_assemble_to("minecraft:minecart", true));
        assert!(can_assemble_to("minecraft:furnace_minecart", false));
        assert!(can_assemble_to("minecraft:chest_minecart", false));
        assert!(!can_assemble_to("minecraft:hopper_minecart", false));
        assert!(Action::AssembleAccelerate.should_assemble() && Action::DisassembleBrake.should_disassemble());
        assert!(!Action::Pass.should_assemble() && !Action::Pass.should_disassemble());
        assert_eq!(RailType::ControllerRail.block(), "create:controller_rail");
    }
}
