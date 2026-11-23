//! The commands that stand in for the Factory Gauge's screen and value box and the Table Cloth's price tag
//! (`FactoryPanelScreen`, `FactoryPanelBehaviour.createBoard` / `setValueSettings`, `FactoryPanelConfigurationPacket`,
//! `TableClothFilteringBehaviour.createBoard` / `setValueSettings`): what a player types and what it asks for.
//!
//! No game types. An item is named by id (`minecraft:` is assumed without a namespace).

use super::factory_gauge::{MAX_TARGET, Slot};
use super::stock_commands::{item_id, parse_flag, rest_after};

/// How a gauge's item is set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemArg {
    /// The item in the player's hand (`FilteringBehaviour.setFilter` with the held stack).
    Held,
    /// `none`: the filter is emptied.
    Clear,
    Id(String),
}

/// What the player asked of the Factory Gauge panel they have open (`/gauge ...`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Gauge {
    /// No argument: what the panel holds and how it is doing.
    Show,
    /// `slot <top_left|top_right|bottom_left|bottom_right>`: which of the four panels the next commands are for (Create picks it by
    /// where the click lands on the block).
    Slot(Slot),
    /// `item [<item>|none]`: the item the panel monitors.
    Item(ItemArg),
    /// `amount <0-100> [items|stacks]`: the target in storage (`setValueSettings`: the row is 0 for items and 1 for stacks).
    Amount { value: i32, stacks: bool },
    /// `address [text]`: where the inputs (or a restocker's items) are sent (`recipeAddress`).
    Address(String),
    /// `output <n>`: how many of the item one craft makes (`recipeOutput`).
    Output(u32),
    /// `promises <never|30s|<minutes>>`: when an open promise expires (`promiseClearingInterval`: -1, 0 or minutes).
    Promises(i32),
    /// `clearpromises`: forget what was asked of the network (`clearPromises`).
    ClearPromises,
    /// `connect <x> <y> <z> <slot>`: use that panel as an input of this one (`FactoryPanelConnectionPacket`).
    Connect { pos: [i32; 3], slot: Slot },
    /// `connect <x> <y> <z>`: attach a Redstone Link or Display Link there (`FactoryPanelConnectionPacket` of a click on a
    /// `FactoryPanelSupportBehaviour`; the link's own slot is chosen from where it is).
    ConnectLink { pos: [i32; 3] },
    /// `unlink`: every attached link is let go (`FactoryPanelConfigurationPacket.redstoneReset`).
    Unlink,
    /// `input <x> <y> <z> <slot> <amount>`: how much of that input one craft needs (`inputAmounts`).
    Input { pos: [i32; 3], slot: Slot, amount: i32 },
    /// `disconnect <x> <y> <z> <slot>`: remove an input (`removeConnection`).
    Disconnect { pos: [i32; 3], slot: Slot },
    /// `move <x> <y> <z> <slot>`: relocate this panel to a free slot of a Factory Gauge (`FactoryPanelBehaviour.moveTo`).
    Move { pos: [i32; 3], slot: Slot },
    /// `bend`: cycle the arrow path mode (`cycleArrowBend`).
    Bend,
    /// `reset`: the panel's settings and connections are cleared (`reset`).
    Reset,
}

/// A slot by name: `top_left`, `tl`, and so on.
pub fn parse_slot(text: &str) -> Result<Slot, String> {
    match text.to_lowercase().replace('-', "_").as_str() {
        "top_left" | "tl" => Ok(Slot::TopLeft),
        "top_right" | "tr" => Ok(Slot::TopRight),
        "bottom_left" | "bl" => Ok(Slot::BottomLeft),
        "bottom_right" | "br" => Ok(Slot::BottomRight),
        other => Err(format!("'{other}' is not a panel (top_left, top_right, bottom_left, bottom_right)")),
    }
}

/// The name of a slot (`PanelSlot.getSerializedName`).
pub fn slot_name(slot: Slot) -> &'static str {
    match slot {
        Slot::TopLeft => "top_left",
        Slot::TopRight => "top_right",
        Slot::BottomLeft => "bottom_left",
        Slot::BottomRight => "bottom_right",
    }
}

fn parse_pos_slot(words: &[&str], usage: &str) -> Result<([i32; 3], Slot), String> {
    if words.len() < 5 {
        return Err(usage.to_owned());
    }
    let mut pos = [0; 3];
    for (i, word) in words[1..4].iter().enumerate() {
        pos[i] = word.parse().map_err(|_| usage.to_owned())?;
    }
    Ok((pos, parse_slot(words[4])?))
}

/// `/gauge <line>`.
pub fn parse_gauge(line: &str) -> Result<Gauge, String> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let Some(first) = words.first() else {
        return Ok(Gauge::Show);
    };
    match *first {
        "slot" => Ok(Gauge::Slot(parse_slot(words.get(1).ok_or("Use /gauge slot <top_left|top_right|bottom_left|bottom_right>")?)?)),
        "item" => Ok(Gauge::Item(match words.get(1) {
            None => ItemArg::Held,
            Some(&"none") | Some(&"clear") => ItemArg::Clear,
            Some(id) => ItemArg::Id(item_id(id)),
        })),
        "amount" => {
            let text = words.get(1).ok_or("Use /gauge amount <0-100> [items|stacks]")?;
            let value: i32 = text.parse().map_err(|_| format!("'{text}' is not an amount"))?;
            if !(0..=MAX_TARGET).contains(&value) {
                return Err(format!("The target is 0 to {MAX_TARGET}"));
            }
            let stacks = match words.get(2) {
                None | Some(&"items") => false,
                Some(&"stacks") => true,
                Some(other) => return Err(format!("'{other}' is not items or stacks")),
            };
            Ok(Gauge::Amount { value, stacks })
        }
        "address" => Ok(Gauge::Address(rest_after(line, 1).to_owned())),
        "output" => {
            let text = words.get(1).ok_or("Use /gauge output <n>")?;
            text.parse::<u32>().ok().filter(|n| *n > 0).map(Gauge::Output).ok_or_else(|| format!("'{text}' is not an amount"))
        }
        "promises" => match words.get(1) {
            Some(&"never") => Ok(Gauge::Promises(-1)),
            Some(&"30s") => Ok(Gauge::Promises(0)),
            Some(text) => {
                text.parse::<i32>().ok().filter(|n| *n > 0).map(Gauge::Promises).ok_or_else(|| format!("'{text}' is not never, 30s or minutes"))
            }
            None => Err("Use /gauge promises <never|30s|minutes>".to_owned()),
        },
        "clearpromises" => Ok(Gauge::ClearPromises),
        "connect" if words.len() == 4 => {
            let mut pos = [0; 3];
            for (i, word) in words[1..4].iter().enumerate() {
                pos[i] = word.parse().map_err(|_| "Use /gauge connect <x> <y> <z> [slot]".to_owned())?;
            }
            Ok(Gauge::ConnectLink { pos })
        }
        "unlink" => Ok(Gauge::Unlink),
        "connect" => parse_pos_slot(&words, "Use /gauge connect <x> <y> <z> <slot>").map(|(pos, slot)| Gauge::Connect { pos, slot }),
        "disconnect" => parse_pos_slot(&words, "Use /gauge disconnect <x> <y> <z> <slot>").map(|(pos, slot)| Gauge::Disconnect { pos, slot }),
        "input" => {
            let (pos, slot) = parse_pos_slot(&words, "Use /gauge input <x> <y> <z> <slot> <amount>")?;
            let text = words.get(5).ok_or("Use /gauge input <x> <y> <z> <slot> <amount>")?;
            let amount = text.parse::<i32>().ok().filter(|n| *n >= 0).ok_or_else(|| format!("'{text}' is not an amount"))?;
            Ok(Gauge::Input { pos, slot, amount })
        }
        "move" => parse_pos_slot(&words, "Use /gauge move <x> <y> <z> <slot>").map(|(pos, slot)| Gauge::Move { pos, slot }),
        "bend" => Ok(Gauge::Bend),
        "reset" => Ok(Gauge::Reset),
        other => Err(format!(
            "Unknown: /gauge {other} (slot, item, amount, address, output, promises, clearpromises, connect, unlink, input, disconnect, move, bend, reset)"
        )),
    }
}

/// What the player asked of the Table Cloth they have selected (`/cloth ...`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cloth {
    /// No argument: the shop's goods and price.
    Show,
    /// `price <n>`: how many of the price item one order costs (`setValueSettings`: at least 1, at most 100).
    Price(i32),
    /// `price none`: the price item is taken away.
    NoPrice,
    /// `partial`-like switches don't exist on a cloth; `owner` shows who may change the price.
    Owner,
}

/// `/cloth <line>`.
pub fn parse_cloth(line: &str) -> Result<Cloth, String> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let Some(first) = words.first() else {
        return Ok(Cloth::Show);
    };
    match *first {
        "price" => match words.get(1) {
            Some(&"none") => Ok(Cloth::NoPrice),
            Some(text) => {
                let n: i32 = text.parse().map_err(|_| format!("'{text}' is not an amount"))?;
                Ok(Cloth::Price(n.clamp(1, 100)))
            }
            None => Err("Use /cloth price <1-100|none>; sneak-click the cloth with the price item first".to_owned()),
        },
        "owner" => Ok(Cloth::Owner),
        other => Err(format!("Unknown: /cloth {other} (price, owner)")),
    }
}

/// A switch like `on` and `off`, for commands that take one.
pub fn flag(text: &str) -> Result<bool, String> {
    parse_flag(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_have_short_and_long_names() {
        assert_eq!(parse_slot("TL"), Ok(Slot::TopLeft));
        assert_eq!(parse_slot("bottom-right"), Ok(Slot::BottomRight));
        assert!(parse_slot("middle").is_err());
        for slot in Slot::ALL {
            assert_eq!(parse_slot(slot_name(slot)), Ok(slot));
        }
    }

    #[test]
    fn gauge_commands_parse() {
        assert_eq!(parse_gauge(""), Ok(Gauge::Show));
        assert_eq!(parse_gauge("move 1 64 -2 tr"), Ok(Gauge::Move { pos: [1, 64, -2], slot: Slot::TopRight }));
        assert!(parse_gauge("move 1 64").is_err());
        assert_eq!(parse_gauge("connect 3 64 -1"), Ok(Gauge::ConnectLink { pos: [3, 64, -1] }));
        assert_eq!(parse_gauge("connect 3 64 -1 tl"), Ok(Gauge::Connect { pos: [3, 64, -1], slot: Slot::TopLeft }));
        assert!(parse_gauge("connect 3 x -1").is_err());
        assert_eq!(parse_gauge("unlink"), Ok(Gauge::Unlink));
        assert_eq!(parse_gauge("item"), Ok(Gauge::Item(ItemArg::Held)));
        assert_eq!(parse_gauge("item none"), Ok(Gauge::Item(ItemArg::Clear)));
        assert_eq!(parse_gauge("item copper_ingot"), Ok(Gauge::Item(ItemArg::Id("minecraft:copper_ingot".into()))));
        assert_eq!(parse_gauge("amount 64"), Ok(Gauge::Amount { value: 64, stacks: false }));
        assert_eq!(parse_gauge("amount 3 stacks"), Ok(Gauge::Amount { value: 3, stacks: true }));
        assert!(parse_gauge("amount 101").is_err());
        assert!(parse_gauge("amount -1").is_err());
        assert!(parse_gauge("amount 4 boxes").is_err());
        assert_eq!(parse_gauge("address Press Line 2"), Ok(Gauge::Address("Press Line 2".into())));
        assert_eq!(parse_gauge("output 2"), Ok(Gauge::Output(2)));
        assert!(parse_gauge("output 0").is_err());
        assert_eq!(parse_gauge("promises never"), Ok(Gauge::Promises(-1)));
        assert_eq!(parse_gauge("promises 30s"), Ok(Gauge::Promises(0)));
        assert_eq!(parse_gauge("promises 5"), Ok(Gauge::Promises(5)));
        assert!(parse_gauge("promises").is_err());
        assert_eq!(parse_gauge("connect 1 -2 3 tr"), Ok(Gauge::Connect { pos: [1, -2, 3], slot: Slot::TopRight }));
        assert_eq!(parse_gauge("input 1 2 3 bl 4"), Ok(Gauge::Input { pos: [1, 2, 3], slot: Slot::BottomLeft, amount: 4 }));
        assert_eq!(parse_gauge("disconnect 0 0 0 br"), Ok(Gauge::Disconnect { pos: [0, 0, 0], slot: Slot::BottomRight }));
        assert!(parse_gauge("connect 1 2").is_err());
        assert!(parse_gauge("fly").is_err());
    }

    #[test]
    fn cloth_prices_stay_in_the_value_box_range() {
        assert_eq!(parse_cloth(""), Ok(Cloth::Show));
        assert_eq!(parse_cloth("price 5"), Ok(Cloth::Price(5)));
        assert_eq!(parse_cloth("price 0"), Ok(Cloth::Price(1)));
        assert_eq!(parse_cloth("price 500"), Ok(Cloth::Price(100)));
        assert_eq!(parse_cloth("price none"), Ok(Cloth::NoPrice));
        assert!(parse_cloth("price").is_err());
        assert!(parse_cloth("price x").is_err());
    }
}
