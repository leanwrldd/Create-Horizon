//! The Clutch in the game (`ClutchBlock`, `ClutchBlockEntity`): a split shaft that redstone power
//! cuts. It shares everything with the Gearshift (`machines/gearshift.rs`); only the rule for
//! what a powered one does with its two ends differs (`create_kinetics::rules::gearshift`).

use create_kinetics::rules::gearshift::Split;
use horizon_client::mod_api::Hooks;

use super::gearshift::SplitShaft;

pub const CLUTCH: &str = "create:clutch";

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(SplitShaft::new(CLUTCH, Split::Clutch))
}
