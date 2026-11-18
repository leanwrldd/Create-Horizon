//! Seats: what a click on one does, where the rider sits, and when the invisible seat a rider
//! sits on goes away. Ported from Create `mc1.21.1/dev` at 4148fd8 (MIT, the Creators of Create):
//! `SeatBlock` (`useItemOn`, `sitDown`), `SeatEntity` (`positionRider`, `tick`), `AllBlocks.SEATS`.
//! No game types.

/// `DyeColor` in its ordinal order, which is `AllBlocks.SEATS`' (`DyedBlockList`).
pub const COLOURS: [&str; 16] = [
    "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan",
    "purple", "blue", "brown", "green", "red", "black",
];

/// `SeatEntity.setPos` is given the block's corner at its centre and `positionRider` puts a player
/// `1/16 + (0.35 - 0.6)` above it (the entity is 0.35 high, a player's vehicle attachment is 0.6).
/// Horizon seats a rider 0.6 below the vehicle's own position when that has no height (a marker),
/// so the marker stands at `0.35 + 1/16` above the block's bottom: the rider ends where Create's.
pub const MARKER_HEIGHT: f64 = 0.35 + 1.0 / 16.0;

/// Game ticks a new seat is left alone while its rider is still being put on it.
pub const GRACE_TICKS: i64 = 20;

/// `create:<colour>_seat`.
pub fn block_of(colour: &str) -> String {
    format!("create:{colour}_seat")
}

/// The colour of the seat block `block` is, if it is one.
pub fn colour_of_block(block: &str) -> Option<&'static str> {
    let name = block.strip_prefix("create:")?.strip_suffix("_seat")?;
    COLOURS.into_iter().find(|c| *c == name)
}

/// `DyeColor.getColor(stack)`: the colour of a dye item.
pub fn dye_colour(item: &str) -> Option<&'static str> {
    let name = item.strip_prefix("minecraft:")?.strip_suffix("_dye")?;
    COLOURS.into_iter().find(|c| *c == name)
}

/// What a click on a seat does (`SeatBlock.useItemOn`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Click {
    /// Not the seat's business (a sneaking player).
    Pass,
    /// A dye of another colour: the seat becomes this block.
    Recolour(String),
    /// Somebody is sitting there.
    Occupied,
    /// Sit the player down.
    Sit,
}

/// `held` is the item in the clicking hand, if any.
pub fn click(sneaking: bool, held: Option<&str>, seat_colour: &str, occupied: bool) -> Click {
    if sneaking {
        return Click::Pass;
    }
    if let Some(colour) = held.and_then(dye_colour)
        && colour != seat_colour
    {
        return Click::Recolour(block_of(colour));
    }
    if occupied {
        return Click::Occupied;
    }
    Click::Sit
}

/// The seat's place on `pos`: the centre of the block, `MARKER_HEIGHT` up.
pub fn marker_position(pos: [i32; 3]) -> [f64; 3] {
    [f64::from(pos[0]) + 0.5, f64::from(pos[1]) + MARKER_HEIGHT, f64::from(pos[2]) + 0.5]
}

/// Whether the one on a seat still is, as far as the server can tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rider {
    Riding,
    Off,
    /// The server could not say (the player isn't in its list).
    Unknown,
}

/// `SeatEntity.tick`: the seat stays while it carries a rider and its block is a seat. A rider
/// that can't be seen keeps it (it is replaced by the next click); a new seat is left alone for
/// [`GRACE_TICKS`].
pub fn keep(age: i64, block_is_seat: bool, rider: Rider) -> bool {
    if !block_is_seat {
        return false;
    }
    age < GRACE_TICKS || rider != Rider::Off
}

/// The UUID of the `n`th seat marker this mod made.
pub fn marker_uuid(n: u64) -> u128 {
    0x4352_4541_5445_5345_4154_0000_0000_0000_u128 | u128::from(n)
}

/// A UUID as commands write it.
pub fn uuid_text(uuid: u128) -> String {
    let h = format!("{uuid:032x}");
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// `startRiding` the seat: Horizon seats a player on a marker through `/ride`.
pub fn ride_command(dimension: &str, rider: u128, vehicle: u128) -> String {
    format!("execute in {dimension} run ride {} mount {}", uuid_text(rider), uuid_text(vehicle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn there_are_sixteen_seats_named_for_dyes() {
        assert_eq!(COLOURS.len(), 16);
        assert_eq!(block_of("light_blue"), "create:light_blue_seat");
        assert_eq!(colour_of_block("create:red_seat"), Some("red"));
        assert_eq!(colour_of_block("create:seat"), None);
        assert_eq!(colour_of_block("minecraft:red_seat"), None);
        assert_eq!(dye_colour("minecraft:lime_dye"), Some("lime"));
        assert_eq!(dye_colour("minecraft:bone_meal"), None);
    }

    #[test]
    fn a_dye_recolours_and_anything_else_sits() {
        assert_eq!(click(false, Some("minecraft:blue_dye"), "red", false), Click::Recolour("create:blue_seat".into()));
        // The same colour, or any other item, or nothing: sit.
        assert_eq!(click(false, Some("minecraft:red_dye"), "red", false), Click::Sit);
        assert_eq!(click(false, Some("minecraft:stick"), "red", false), Click::Sit);
        assert_eq!(click(false, None, "red", false), Click::Sit);
        // A taken seat can still be dyed, but nobody else sits.
        assert_eq!(click(false, None, "red", true), Click::Occupied);
        assert!(matches!(click(false, Some("minecraft:blue_dye"), "red", true), Click::Recolour(_)));
        // Sneaking leaves the click to the block placed or the item used.
        assert_eq!(click(true, Some("minecraft:blue_dye"), "red", false), Click::Pass);
    }

    #[test]
    fn the_seat_stands_where_create_puts_its_entity() {
        let at = marker_position([3, 64, -2]);
        assert_eq!(at[0], 3.5);
        assert_eq!(at[2], -1.5);
        assert!((at[1] - 64.4125).abs() < 1e-9);
    }

    #[test]
    fn a_seat_goes_when_its_block_goes_or_its_rider_gets_off() {
        assert!(!keep(100, false, Rider::Riding));
        assert!(keep(100, true, Rider::Riding));
        assert!(!keep(100, true, Rider::Off));
        assert!(keep(100, true, Rider::Unknown));
        // A fresh one isn't judged yet.
        assert!(keep(1, true, Rider::Off));
    }

    #[test]
    fn uuids_and_commands_are_written_as_the_game_reads_them() {
        let uuid = marker_uuid(5);
        assert_eq!(uuid_text(uuid), "43524541-5445-5345-4154-000000000005");
        assert_eq!(
            ride_command("minecraft:overworld", 0x1, uuid),
            "execute in minecraft:overworld run ride 00000000-0000-0000-0000-000000000001 mount 43524541-5445-5345-4154-000000000005"
        );
    }
}
