//! The commands that stand in for Create's logistics screens (`StockKeeperRequestScreen`, `RedstoneRequesterScreen`):
//! what a player types and what it asks for. The request screen builds an order from the stock list with clicks
//! (`StockKeeperRequestScreen.mouseClicked`: one, ten or a stack at a time, `OrderList` in `rules::stock_ticker`), types an
//! address in a text field and sends or programs it; the Redstone Requester's screen sets the address, whether partial
//! requests are allowed, and the amounts.
//!
//! No game types. An item is named by id (`minecraft:` is assumed without a namespace); a count is a number, or
//! `stack`, `all` or `max` for as many as there are.

/// What the player asked of the Stock Ticker they have open (`/stock ...`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stock {
    /// No argument: what the ticker's network holds, and the order so far.
    Show,
    /// `list [page]`: the stock, the most first, in pages of ten.
    List(usize),
    /// `add <item> [count]`: put the item in the order (`StockKeeperRequestScreen`'s left click, `transfer` times).
    Add { item: String, count: Count },
    /// `remove <item> [count]`: take it off the order.
    Remove { item: String, count: Count },
    /// `clear`: an empty order.
    Clear,
    /// `address [text]`: the address the order goes to (the screen's text field); no text clears it.
    Address(String),
    /// `send`: broadcast the order (`PackageOrderRequestPacket`).
    Send,
    /// `program`: remember the order, the address and this ticker for the next Redstone Requester (or Table Cloth) the
    /// player places (`RedstoneRequesterBlock.programRequester`).
    Program,
    /// `open <x> <y> <z>`: open another ticker.
    Open([i32; 3]),
}

/// How many of an item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Count {
    Exactly(u32),
    /// `stack`: one stack of the item (`click_transfer` with shift).
    Stack,
    /// `all` and `max`: everything the network holds.
    All,
}

impl Count {
    /// The number for an item with `max_stack` and `stock` held.
    pub fn resolve(self, max_stack: u32, stock: u32) -> u32 {
        match self {
            Count::Exactly(n) => n,
            Count::Stack => max_stack,
            Count::All => stock,
        }
    }
}

/// What the player asked of the Redstone Requester they have open (`/requester ...`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Requester {
    /// No argument: its address, order and flags.
    Show,
    /// `set <item> <count>`: a ghost slot with an amount (`saveData`, `applySettings`); at most nine.
    Set { item: String, count: u32 },
    /// `remove <item>`.
    Remove { item: String },
    /// `clear`.
    Clear,
    /// `address [text]`.
    Address(String),
    /// `partial <true|false>`: `allowPartialRequests`.
    Partial(bool),
}

/// `namespace:path`, with `minecraft` when the player left the namespace out.
pub fn item_id(text: &str) -> String {
    let text = text.trim().to_lowercase();
    if text.contains(':') { text } else { format!("minecraft:{text}") }
}

fn parse_count(text: &str) -> Result<Count, String> {
    match text {
        "stack" => Ok(Count::Stack),
        "all" | "max" => Ok(Count::All),
        _ => match text.parse::<u32>() {
            Ok(n) if n > 0 => Ok(Count::Exactly(n)),
            _ => Err(format!("'{text}' is not a count (a number, stack or all)")),
        },
    }
}

pub(crate) fn parse_flag(text: &str) -> Result<bool, String> {
    match text {
        "true" | "yes" | "on" => Ok(true),
        "false" | "no" | "off" => Ok(false),
        _ => Err(format!("'{text}' is not true or false")),
    }
}

/// The words of a line, with the rest of it after `n` words kept whole (an address may hold spaces).
pub(crate) fn rest_after(line: &str, words: usize) -> &str {
    let mut rest = line.trim_start();
    for _ in 0..words {
        rest = rest.trim_start().split_once(char::is_whitespace).map_or("", |(_, tail)| tail);
    }
    rest.trim()
}

/// `/stock <line>`.
pub fn parse_stock(line: &str) -> Result<Stock, String> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let Some(first) = words.first() else {
        return Ok(Stock::Show);
    };
    match *first {
        "list" => match words.get(1) {
            None => Ok(Stock::List(1)),
            Some(page) => page.parse::<usize>().ok().filter(|p| *p > 0).map(Stock::List).ok_or_else(|| format!("'{page}' is not a page")),
        },
        "add" | "remove" => {
            let item = item_id(words.get(1).ok_or("Name an item: /stock add <item> [count]")?);
            let count = match words.get(2) {
                None => Count::Exactly(1),
                Some(text) => parse_count(text)?,
            };
            Ok(if *first == "add" { Stock::Add { item, count } } else { Stock::Remove { item, count } })
        }
        "clear" => Ok(Stock::Clear),
        "address" => Ok(Stock::Address(rest_after(line, 1).to_owned())),
        "send" => Ok(Stock::Send),
        "program" => Ok(Stock::Program),
        "open" => {
            let coords: Vec<i32> = words[1..].iter().filter_map(|w| w.parse().ok()).collect();
            match coords[..] {
                [x, y, z] if words.len() == 4 => Ok(Stock::Open([x, y, z])),
                _ => Err("Use /stock open <x> <y> <z>".to_owned()),
            }
        }
        other => Err(format!("Unknown: /stock {other} (list, add, remove, clear, address, send, program)")),
    }
}

/// `/requester <line>`.
pub fn parse_requester(line: &str) -> Result<Requester, String> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let Some(first) = words.first() else {
        return Ok(Requester::Show);
    };
    match *first {
        "set" => {
            let item = item_id(words.get(1).ok_or("Use /requester set <item> <count>")?);
            let count = match words.get(2) {
                None => 1,
                Some(text) => text.parse::<u32>().ok().filter(|n| *n > 0).ok_or_else(|| format!("'{text}' is not a count"))?,
            };
            Ok(Requester::Set { item, count })
        }
        "remove" => Ok(Requester::Remove { item: item_id(words.get(1).ok_or("Use /requester remove <item>")?) }),
        "clear" => Ok(Requester::Clear),
        "address" => Ok(Requester::Address(rest_after(line, 1).to_owned())),
        "partial" => Ok(Requester::Partial(parse_flag(words.get(1).ok_or("Use /requester partial <true|false>")?)?)),
        other => Err(format!("Unknown: /requester {other} (set, remove, clear, address, partial)")),
    }
}

/// `RedstoneRequesterConfigurationPacket.applySettings` and `RedstoneRequesterMenu.saveData`, one change at a time: `Set` gives an
/// item an amount (a ghost slot of its own, nine at most), `Remove` takes it out, and the amounts, the address and the partial
/// switch go through `Requester::apply_configuration`, which also drops recipes that no longer use up what is ordered.
/// `key_of` makes the item of an id, `id_of` names one.
pub fn configure<K: PartialEq + Clone>(
    state: &mut super::redstone_requester::Requester<K>,
    command: Requester,
    key_of: &dyn Fn(&str) -> K,
    id_of: &dyn Fn(&K) -> String,
) -> Result<(), String> {
    use super::package::Context;
    let address = state.encoded_target_address.clone();
    let partial = state.allow_partial_requests;
    let amounts = |state: &super::redstone_requester::Requester<K>| -> Vec<i32> {
        state.encoded_request.stacks.iter().map(|(_, count)| (*count).min(i32::MAX as u32) as i32).collect()
    };
    match command {
        Requester::Show => {}
        Requester::Set { item, count } => {
            let key = key_of(&item);
            match state.encoded_request.stacks.iter().position(|(k, _)| *k == key) {
                Some(index) => state.encoded_request.stacks[index].1 = count,
                None if state.encoded_request.stacks.len() < super::redstone_requester::GHOST_SLOTS => {
                    state.encoded_request.stacks.push((key, count));
                }
                None => return Err("A requester holds nine different items at most".to_owned()),
            }
            let all = amounts(state);
            state.apply_configuration(&address, partial, &all);
        }
        Requester::Remove { item } => {
            let before = state.encoded_request.stacks.len();
            state.encoded_request.stacks.retain(|(key, _)| id_of(key) != item);
            if state.encoded_request.stacks.len() == before {
                return Err("It is not in the order".to_owned());
            }
            let all = amounts(state);
            state.apply_configuration(&address, partial, &all);
        }
        Requester::Clear => state.encoded_request = Context::default(),
        Requester::Address(text) => {
            let all = amounts(state);
            state.apply_configuration(&text, partial, &all);
        }
        Requester::Partial(allow) => {
            let all = amounts(state);
            state.apply_configuration(&address, allow, &all);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_get_a_namespace() {
        assert_eq!(item_id("Diamond"), "minecraft:diamond");
        assert_eq!(item_id("create:cogwheel"), "create:cogwheel");
    }

    #[test]
    fn stock_commands_parse() {
        assert_eq!(parse_stock(""), Ok(Stock::Show));
        assert_eq!(parse_stock("list 2"), Ok(Stock::List(2)));
        assert_eq!(parse_stock("add iron_ingot 12"), Ok(Stock::Add { item: "minecraft:iron_ingot".into(), count: Count::Exactly(12) }));
        assert_eq!(parse_stock("add create:cogwheel"), Ok(Stock::Add { item: "create:cogwheel".into(), count: Count::Exactly(1) }));
        assert_eq!(parse_stock("remove stone stack"), Ok(Stock::Remove { item: "minecraft:stone".into(), count: Count::Stack }));
        assert_eq!(parse_stock("address My Farm  1"), Ok(Stock::Address("My Farm  1".into())));
        assert_eq!(parse_stock("address"), Ok(Stock::Address(String::new())));
        assert_eq!(parse_stock("open 1 -2 3"), Ok(Stock::Open([1, -2, 3])));
        assert!(parse_stock("add").is_err());
        assert!(parse_stock("add stone 0").is_err());
        assert!(parse_stock("fly").is_err());
    }

    #[test]
    fn counts_resolve() {
        assert_eq!(Count::Exactly(5).resolve(64, 100), 5);
        assert_eq!(Count::Stack.resolve(16, 100), 16);
        assert_eq!(Count::All.resolve(64, 37), 37);
    }

    #[test]
    fn a_requester_is_configured_one_change_at_a_time() {
        use super::super::redstone_requester::Requester as State;
        let key = |id: &str| id.to_owned();
        let id = |k: &String| k.clone();
        let mut state: State<String> = State::default();
        let set = |item: &str, count| Requester::Set { item: item.into(), count };
        configure(&mut state, set("minecraft:stone", 4), &key, &id).unwrap();
        configure(&mut state, set("minecraft:iron_ingot", 9), &key, &id).unwrap();
        configure(&mut state, set("minecraft:stone", 6), &key, &id).unwrap();
        assert_eq!(state.encoded_request.stacks, vec![("minecraft:stone".to_owned(), 6), ("minecraft:iron_ingot".to_owned(), 9)]);
        configure(&mut state, Requester::Address("Mill".into()), &key, &id).unwrap();
        configure(&mut state, Requester::Partial(true), &key, &id).unwrap();
        assert_eq!(state.encoded_target_address, "Mill");
        assert!(state.allow_partial_requests);
        // An address change keeps the order, a partial switch keeps the address.
        assert_eq!(state.encoded_request.stacks.len(), 2);
        configure(&mut state, Requester::Remove { item: "minecraft:stone".into() }, &key, &id).unwrap();
        assert_eq!(state.encoded_request.stacks, vec![("minecraft:iron_ingot".to_owned(), 9)]);
        assert!(configure(&mut state, Requester::Remove { item: "minecraft:stone".into() }, &key, &id).is_err());
        configure(&mut state, Requester::Clear, &key, &id).unwrap();
        assert!(state.encoded_request.stacks.is_empty());
        assert_eq!(state.encoded_target_address, "Mill");
    }

    #[test]
    fn a_requester_holds_nine_items_at_most() {
        use super::super::redstone_requester::Requester as State;
        let key = |id: &str| id.to_owned();
        let id = |k: &String| k.clone();
        let mut state: State<String> = State::default();
        for n in 0..9 {
            configure(&mut state, Requester::Set { item: format!("minecraft:item_{n}"), count: 1 }, &key, &id).unwrap();
        }
        assert!(configure(&mut state, Requester::Set { item: "minecraft:tenth".into(), count: 1 }, &key, &id).is_err());
        // An item already there is changed, not added.
        configure(&mut state, Requester::Set { item: "minecraft:item_3".into(), count: 20 }, &key, &id).unwrap();
        assert_eq!(state.encoded_request.stacks.len(), 9);
        assert_eq!(state.encoded_request.stacks[3].1, 20);
    }

    #[test]
    fn requester_commands_parse() {
        assert_eq!(parse_requester(""), Ok(Requester::Show));
        assert_eq!(parse_requester("set stone 4"), Ok(Requester::Set { item: "minecraft:stone".into(), count: 4 }));
        assert_eq!(parse_requester("partial on"), Ok(Requester::Partial(true)));
        assert_eq!(parse_requester("address Mill"), Ok(Requester::Address("Mill".into())));
        assert!(parse_requester("partial maybe").is_err());
        assert!(parse_requester("set stone x").is_err());
    }
}
