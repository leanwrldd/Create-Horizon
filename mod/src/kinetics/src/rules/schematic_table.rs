//! The Schematic Table's decisions: which clicks a block takes before a Schematic or a Schematic And
//! Quill gets them, and which file a name or a number picks (`SchematicTableBlock.useWithoutItem`,
//! `SchematicTableMenu`; the screen's upload button's choice of file). No game types.

/// `create:schematic_table`.
pub const TABLE: &str = "create:schematic_table";
/// `create:schematicannon`.
pub const CANNON: &str = "create:schematicannon";

/// Whether the block clicked takes the click itself (opens its screen) rather than the Schematic
/// or the Schematic And Quill in hand using it on the block: Create's table and cannon have
/// `useItemOn`/`useWithoutItem` of their own, and a block's use comes before the item's.
pub fn block_takes_click(target: &str) -> bool {
    target == TABLE || target == CANNON
}

/// The file a name or a number picks among `names`.
pub fn pick(names: &[String], wanted: &str) -> Result<String, String> {
    let wanted = wanted.trim();
    if wanted.is_empty() {
        return Err("Give a file name or a number from /schematic list".to_owned());
    }
    if let Ok(n) = wanted.parse::<usize>()
        && n >= 1
        && n <= names.len()
    {
        return Ok(names[n - 1].clone());
    }
    let lower = wanted.to_lowercase();
    let with_ext = if lower.ends_with(".nbt") { lower.clone() } else { format!("{lower}.nbt") };
    if let Some(name) = names.iter().find(|n| n.to_lowercase() == with_ext) {
        return Ok(name.clone());
    }
    let starting: Vec<&String> = names.iter().filter(|n| n.to_lowercase().starts_with(&lower)).collect();
    match starting.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(format!("No schematic called {wanted} (see /schematic list)")),
        _ => Err(format!("{wanted} fits more than one file (see /schematic list)")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        ["castle.nbt", "castle_keep.nbt", "House.nbt"].map(str::to_owned).to_vec()
    }

    /// The in-game report: with the Quill in hand a click on the table set a selection corner
    /// instead of opening the table.
    #[test]
    fn the_table_and_the_cannon_take_the_click_before_the_quill() {
        assert!(block_takes_click(TABLE));
        assert!(block_takes_click(CANNON));
        assert!(!block_takes_click("minecraft:stone"));
    }

    #[test]
    fn a_file_is_picked_by_number_or_name() {
        assert_eq!(pick(&names(), "3").unwrap(), "House.nbt");
        assert_eq!(pick(&names(), "house").unwrap(), "House.nbt");
        assert_eq!(pick(&names(), "castle.nbt").unwrap(), "castle.nbt");
        // An exact name wins over the files it is the start of.
        assert_eq!(pick(&names(), "castle").unwrap(), "castle.nbt");
        assert!(pick(&names(), "cast").unwrap_err().contains("more than one"));
        assert!(pick(&names(), "moat").is_err());
        assert!(pick(&names(), "").is_err());
        assert!(pick(&names(), "9").is_err());
    }
}
