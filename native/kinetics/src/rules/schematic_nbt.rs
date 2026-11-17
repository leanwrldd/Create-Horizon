//! Named binary tags (NBT), as vanilla's structure files hold them, in plain Rust: the format under
//! a Schematic (`SchematicExport` writes `NbtIo.writeCompressed`, `SchematicItem.loadSchematic`
//! reads `NbtIo.read`). The glue gunzips and gzips; this reads and writes the bytes inside.
//!
//! Known differences from Create, for the README:
//! - Strings are written as plain UTF-8, not Java's modified UTF-8 (they differ only for the null
//!   character and characters outside the first plane, which block names and sign text rarely hold).
//!   Reading accepts both.

use std::collections::BTreeMap;

/// One tag. Compounds are sorted by key (vanilla's are unordered).
#[derive(Clone, Debug, PartialEq)]
pub enum Nbt {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    String(String),
    /// A list holds one kind of tag; an empty list is written as a list of ends.
    List(Vec<Nbt>),
    Compound(BTreeMap<String, Nbt>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl Nbt {
    pub fn compound() -> Self {
        Nbt::Compound(BTreeMap::new())
    }

    pub fn string(text: &str) -> Self {
        Nbt::String(text.to_owned())
    }

    pub fn ints(values: &[i32]) -> Self {
        Nbt::List(values.iter().map(|v| Nbt::Int(*v)).collect())
    }

    pub fn doubles(values: &[f64]) -> Self {
        Nbt::List(values.iter().map(|v| Nbt::Double(*v)).collect())
    }

    pub fn get(&self, key: &str) -> Option<&Nbt> {
        match self {
            Nbt::Compound(map) => map.get(key),
            _ => None,
        }
    }

    pub fn put(&mut self, key: &str, value: Nbt) {
        if let Nbt::Compound(map) = self {
            map.insert(key.to_owned(), value);
        }
    }

    pub fn remove(&mut self, key: &str) -> Option<Nbt> {
        match self {
            Nbt::Compound(map) => map.remove(key),
            _ => None,
        }
    }

    /// Any whole-number tag as an int (`CompoundTag.getInt` reads every numeric kind).
    pub fn as_int(&self) -> Option<i32> {
        match self {
            Nbt::Byte(v) => Some(i32::from(*v)),
            Nbt::Short(v) => Some(i32::from(*v)),
            Nbt::Int(v) => Some(*v),
            Nbt::Long(v) => Some(*v as i32),
            Nbt::Float(v) => Some(*v as i32),
            Nbt::Double(v) => Some(*v as i32),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Nbt::Double(v) => Some(*v),
            Nbt::Float(v) => Some(f64::from(*v)),
            other => other.as_int().map(f64::from),
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Nbt::String(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Nbt]> {
        match self {
            Nbt::List(items) => Some(items),
            _ => None,
        }
    }

    fn id(&self) -> u8 {
        match self {
            Nbt::Byte(_) => 1,
            Nbt::Short(_) => 2,
            Nbt::Int(_) => 3,
            Nbt::Long(_) => 4,
            Nbt::Float(_) => 5,
            Nbt::Double(_) => 6,
            Nbt::ByteArray(_) => 7,
            Nbt::String(_) => 8,
            Nbt::List(_) => 9,
            Nbt::Compound(_) => 10,
            Nbt::IntArray(_) => 11,
            Nbt::LongArray(_) => 12,
        }
    }
}

// --- writing --------------------------------------------------------------------------------------

/// `NbtIo.write`: the root compound with its name (empty for a structure), uncompressed.
pub fn write(name: &str, tag: &Nbt) -> Vec<u8> {
    let mut out = vec![tag.id()];
    string(&mut out, name);
    payload(&mut out, tag);
    out
}

fn string(out: &mut Vec<u8>, text: &str) {
    let bytes = text.as_bytes();
    let length = bytes.len().min(u16::MAX as usize);
    out.extend_from_slice(&(length as u16).to_be_bytes());
    out.extend_from_slice(&bytes[..length]);
}

fn payload(out: &mut Vec<u8>, tag: &Nbt) {
    match tag {
        Nbt::Byte(v) => out.push(*v as u8),
        Nbt::Short(v) => out.extend_from_slice(&v.to_be_bytes()),
        Nbt::Int(v) => out.extend_from_slice(&v.to_be_bytes()),
        Nbt::Long(v) => out.extend_from_slice(&v.to_be_bytes()),
        Nbt::Float(v) => out.extend_from_slice(&v.to_bits().to_be_bytes()),
        Nbt::Double(v) => out.extend_from_slice(&v.to_bits().to_be_bytes()),
        Nbt::ByteArray(values) => {
            out.extend_from_slice(&(values.len() as i32).to_be_bytes());
            out.extend(values.iter().map(|v| *v as u8));
        }
        Nbt::String(text) => string(out, text),
        Nbt::List(items) => {
            // A list is of one kind: the first element's (a mixed list is written as that kind and
            // its odd elements are written as it too, which can't happen for what this mod builds).
            out.push(items.first().map_or(0, Nbt::id));
            out.extend_from_slice(&(items.len() as i32).to_be_bytes());
            for item in items {
                payload(out, item);
            }
        }
        Nbt::Compound(map) => {
            for (key, value) in map {
                out.push(value.id());
                string(out, key);
                payload(out, value);
            }
            out.push(0);
        }
        Nbt::IntArray(values) => {
            out.extend_from_slice(&(values.len() as i32).to_be_bytes());
            for v in values {
                out.extend_from_slice(&v.to_be_bytes());
            }
        }
        Nbt::LongArray(values) => {
            out.extend_from_slice(&(values.len() as i32).to_be_bytes());
            for v in values {
                out.extend_from_slice(&v.to_be_bytes());
            }
        }
    }
}

// --- reading --------------------------------------------------------------------------------------

/// The deepest nesting read (vanilla's `NbtAccounter` stops at 512).
const MAX_DEPTH: usize = 512;

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self.at.checked_add(n).filter(|end| *end <= self.bytes.len());
        let Some(end) = end else {
            return Err("unexpected end of the data".to_owned());
        };
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn i16(&mut self) -> Result<i16, String> {
        Ok(i16::from_be_bytes(self.take(2)?.try_into().unwrap_or([0; 2])))
    }

    fn i32(&mut self) -> Result<i32, String> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap_or([0; 4])))
    }

    fn i64(&mut self) -> Result<i64, String> {
        Ok(i64::from_be_bytes(self.take(8)?.try_into().unwrap_or([0; 8])))
    }

    fn length(&mut self) -> Result<usize, String> {
        let n = self.i32()?;
        usize::try_from(n).map_err(|_| "a negative length".to_owned())
    }

    fn string(&mut self) -> Result<String, String> {
        let n = self.i16()? as u16 as usize;
        Ok(modified_utf8(self.take(n)?))
    }

    fn tag(&mut self, id: u8, depth: usize) -> Result<Nbt, String> {
        if depth > MAX_DEPTH {
            return Err("tags nested too deep".to_owned());
        }
        Ok(match id {
            1 => Nbt::Byte(self.u8()? as i8),
            2 => Nbt::Short(self.i16()?),
            3 => Nbt::Int(self.i32()?),
            4 => Nbt::Long(self.i64()?),
            5 => Nbt::Float(f32::from_bits(self.i32()? as u32)),
            6 => Nbt::Double(f64::from_bits(self.i64()? as u64)),
            7 => {
                let n = self.length()?;
                Nbt::ByteArray(self.take(n)?.iter().map(|b| *b as i8).collect())
            }
            8 => Nbt::String(self.string()?),
            9 => {
                let kind = self.u8()?;
                let n = self.length()?;
                if kind == 0 && n > 0 {
                    return Err("a list of ends".to_owned());
                }
                // Each element takes at least a byte, except for ends: don't trust a huge count.
                if n > self.bytes.len() {
                    return Err("a list longer than the data".to_owned());
                }
                let mut items = Vec::with_capacity(n);
                for _ in 0..n {
                    items.push(self.tag(kind, depth + 1)?);
                }
                Nbt::List(items)
            }
            10 => {
                let mut map = BTreeMap::new();
                loop {
                    let kind = self.u8()?;
                    if kind == 0 {
                        break;
                    }
                    let key = self.string()?;
                    let value = self.tag(kind, depth + 1)?;
                    map.insert(key, value);
                }
                Nbt::Compound(map)
            }
            11 => {
                let n = self.length()?;
                if n.saturating_mul(4) > self.bytes.len() {
                    return Err("an array longer than the data".to_owned());
                }
                Nbt::IntArray((0..n).map(|_| self.i32()).collect::<Result<_, _>>()?)
            }
            12 => {
                let n = self.length()?;
                if n.saturating_mul(8) > self.bytes.len() {
                    return Err("an array longer than the data".to_owned());
                }
                Nbt::LongArray((0..n).map(|_| self.i64()).collect::<Result<_, _>>()?)
            }
            other => return Err(format!("an unknown tag kind {other}")),
        })
    }
}

/// Java's modified UTF-8 as text: the null character as two bytes, a character past the first
/// plane as two surrogates of three bytes each. Plain UTF-8 reads as itself.
fn modified_utf8(bytes: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_owned();
    }
    let mut units: Vec<u16> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b & 0x80 == 0 {
            units.push(u16::from(b));
            i += 1;
        } else if b & 0xE0 == 0xC0 && i + 1 < bytes.len() {
            units.push((u16::from(b & 0x1F) << 6) | u16::from(bytes[i + 1] & 0x3F));
            i += 2;
        } else if b & 0xF0 == 0xE0 && i + 2 < bytes.len() {
            units.push(
                (u16::from(b & 0x0F) << 12)
                    | (u16::from(bytes[i + 1] & 0x3F) << 6)
                    | u16::from(bytes[i + 2] & 0x3F),
            );
            i += 3;
        } else {
            units.push(0xFFFD);
            i += 1;
        }
    }
    String::from_utf16_lossy(&units)
}

/// `NbtIo.read`: the root tag and its name, from uncompressed bytes.
pub fn read(bytes: &[u8]) -> Result<(String, Nbt), String> {
    let mut reader = Reader { bytes, at: 0 };
    let id = reader.u8()?;
    if id == 0 {
        return Err("the data holds no tag".to_owned());
    }
    let name = reader.string()?;
    let tag = reader.tag(id, 0)?;
    Ok((name, tag))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Nbt {
        let mut inner = Nbt::compound();
        inner.put("Name", Nbt::string("minecraft:oak_stairs"));
        inner.put("x", Nbt::Byte(-3));
        let mut root = Nbt::compound();
        root.put("size", Nbt::ints(&[3, 2, 1]));
        root.put("DataVersion", Nbt::Int(5023));
        root.put("palette", Nbt::List(vec![inner]));
        root.put("pos", Nbt::doubles(&[0.5, 64.0, -2.25]));
        root.put("empty", Nbt::List(Vec::new()));
        root.put("longs", Nbt::LongArray(vec![1, -2, i64::MAX]));
        root.put("bytes", Nbt::ByteArray(vec![1, 2, -1]));
        root.put("ints", Nbt::IntArray(vec![7, -7]));
        root.put("f", Nbt::Float(1.5));
        root.put("s", Nbt::Short(-9));
        root.put("l", Nbt::Long(1 << 40));
        root
    }

    #[test]
    fn a_tree_round_trips() {
        let tree = sample();
        let bytes = write("", &tree);
        let (name, back) = read(&bytes).unwrap();
        assert_eq!(name, "");
        assert_eq!(back, tree);
    }

    #[test]
    fn the_bytes_are_vanillas() {
        // 0x0A (compound), name length 0, then a byte tag "a" = 1, then the end.
        let mut root = Nbt::compound();
        root.put("a", Nbt::Byte(1));
        assert_eq!(write("", &root), [0x0A, 0, 0, 0x01, 0, 1, b'a', 1, 0]);
    }

    #[test]
    fn truncated_and_lying_data_is_refused() {
        let bytes = write("", &sample());
        assert!(read(&bytes[..bytes.len() - 3]).is_err());
        // A list that claims four billion elements.
        let lying = [0x0A, 0, 0, 0x09, 0, 1, b'l', 3, 0x7F, 0xFF, 0xFF, 0xFF, 0];
        assert!(read(&lying).is_err());
        assert!(read(&[]).is_err());
        assert!(read(&[0]).is_err());
    }

    #[test]
    fn java_s_modified_utf8_reads() {
        // "a", the null character (C0 80) and U+1F600 as a surrogate pair (ED A0 BD ED B8 80).
        let java = [b'a', 0xC0, 0x80, 0xED, 0xA0, 0xBD, 0xED, 0xB8, 0x80];
        assert_eq!(modified_utf8(&java), "a\u{0}\u{1F600}");
    }
}
