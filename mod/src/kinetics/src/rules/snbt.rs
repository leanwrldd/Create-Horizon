//! SNBT, the text form of NBT (`/data merge`, schematic text files): a parser into a plain value tree. Horizon's mod API
//! hands a mod no SNBT reader, so the compat layer parses here and builds its `Tag` from the tree.
//!
//! Grammar as Minecraft's `TagParser` reads it: compounds `{key:value,...}` (keys bare or quoted), lists `[a,b]`,
//! typed arrays `[B;1b,2b]` `[I;1,2]` `[L;1l]`, strings in `"` or `'` with `\` escapes or bare
//! (`[0-9A-Za-z_.+-]`), numbers with a suffix `b s l f d` (any case), a bare integer is an int, a bare decimal a double,
//! `true` and `false` are the bytes 1 and 0. Whitespace is allowed between tokens.

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    Str(String),
    ByteArray(Vec<i8>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
    List(Vec<Value>),
    Compound(Vec<(String, Value)>),
}

/// Parses one value from the start of `text` (leading whitespace skipped) and says how many bytes it read.
pub fn parse(text: &str) -> Result<(Value, usize), String> {
    let mut reader = Reader { text, at: 0 };
    let value = reader.value()?;
    Ok((value, reader.at))
}

struct Reader<'a> {
    text: &'a str,
    at: usize,
}

fn bare(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '+' | '-')
}

impl Reader<'_> {
    fn peek(&self) -> Option<char> {
        self.text[self.at..].chars().next()
    }

    fn skip(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.at += self.peek().map_or(0, char::len_utf8);
        }
    }

    fn eat(&mut self, c: char) -> bool {
        self.skip();
        if self.peek() == Some(c) {
            self.at += c.len_utf8();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, c: char) -> Result<(), String> {
        if self.eat(c) { Ok(()) } else { Err(format!("expected '{c}' at {}", self.at)) }
    }

    fn quoted(&mut self, quote: char) -> Result<String, String> {
        self.at += quote.len_utf8();
        let mut out = String::new();
        loop {
            let c = self.peek().ok_or("unterminated string")?;
            self.at += c.len_utf8();
            match c {
                '\\' => {
                    let escaped = self.peek().ok_or("unterminated escape")?;
                    self.at += escaped.len_utf8();
                    out.push(match escaped {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        'b' => '\u{8}',
                        'f' => '\u{c}',
                        other => other,
                    });
                }
                c if c == quote => return Ok(out),
                c => out.push(c),
            }
        }
    }

    fn word(&mut self) -> String {
        let start = self.at;
        while self.peek().is_some_and(bare) {
            self.at += 1;
        }
        self.text[start..self.at].to_owned()
    }

    fn key(&mut self) -> Result<String, String> {
        self.skip();
        match self.peek() {
            Some(q @ ('"' | '\'')) => self.quoted(q),
            Some(c) if bare(c) => Ok(self.word()),
            _ => Err(format!("expected a key at {}", self.at)),
        }
    }

    fn value(&mut self) -> Result<Value, String> {
        self.skip();
        match self.peek().ok_or("expected a value, found the end")? {
            '{' => self.compound(),
            '[' => self.list(),
            q @ ('"' | '\'') => Ok(Value::Str(self.quoted(q)?)),
            c if bare(c) => Ok(scalar(&self.word())),
            c => Err(format!("unexpected '{c}' at {}", self.at)),
        }
    }

    fn compound(&mut self) -> Result<Value, String> {
        self.expect('{')?;
        let mut fields = Vec::new();
        if self.eat('}') {
            return Ok(Value::Compound(fields));
        }
        loop {
            let key = self.key()?;
            self.expect(':')?;
            let value = self.value()?;
            fields.push((key, value));
            if self.eat(',') {
                // a trailing comma is allowed before the brace
                if self.eat('}') {
                    break;
                }
                continue;
            }
            self.expect('}')?;
            break;
        }
        Ok(Value::Compound(fields))
    }

    fn list(&mut self) -> Result<Value, String> {
        self.expect('[')?;
        // `[B;` `[I;` `[L;`
        self.skip();
        let rest = &self.text[self.at..];
        let kind = rest.chars().next().filter(|c| matches!(c, 'B' | 'I' | 'L') && rest[1..].trim_start().starts_with(';'));
        if let Some(kind) = kind {
            self.at += 1;
            self.expect(';')?;
            let items = self.items()?;
            return typed_array(kind, items);
        }
        Ok(Value::List(self.items()?))
    }

    /// The values up to the closing `]`, which it reads.
    fn items(&mut self) -> Result<Vec<Value>, String> {
        let mut out = Vec::new();
        if self.eat(']') {
            return Ok(out);
        }
        loop {
            out.push(self.value()?);
            if self.eat(',') {
                if self.eat(']') {
                    break;
                }
                continue;
            }
            self.expect(']')?;
            break;
        }
        Ok(out)
    }
}

fn typed_array(kind: char, items: Vec<Value>) -> Result<Value, String> {
    let bad = || format!("a '{kind}' array holds only its own numbers");
    Ok(match kind {
        'B' => Value::ByteArray(
            items.into_iter().map(|v| if let Value::Byte(b) = v { Ok(b) } else { Err(bad()) }).collect::<Result<_, _>>()?,
        ),
        'I' => Value::IntArray(
            items
                .into_iter()
                .map(|v| match v {
                    Value::Int(i) => Ok(i),
                    _ => Err(bad()),
                })
                .collect::<Result<_, _>>()?,
        ),
        _ => Value::LongArray(
            items
                .into_iter()
                .map(|v| match v {
                    Value::Long(i) => Ok(i),
                    Value::Int(i) => Ok(i64::from(i)),
                    _ => Err(bad()),
                })
                .collect::<Result<_, _>>()?,
        ),
    })
}

/// A bare word: a number with its suffix, `true`/`false`, or a string.
fn scalar(word: &str) -> Value {
    match word {
        "true" => return Value::Byte(1),
        "false" => return Value::Byte(0),
        _ => {}
    }
    let (digits, suffix) = match word.char_indices().last() {
        Some((at, c)) if c.is_ascii_alphabetic() => (&word[..at], Some(c.to_ascii_lowercase())),
        _ => (word, None),
    };
    let integer = {
        let body = digits.strip_prefix(['-', '+']).unwrap_or(digits);
        !body.is_empty() && body.bytes().all(|b| b.is_ascii_digit())
    };
    let decimal = digits.parse::<f64>().is_ok()
        && digits.bytes().any(|b| b.is_ascii_digit())
        && digits.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'-' | b'+' | b'.' | b'e' | b'E'));
    let parsed = match suffix {
        Some('b') if integer => digits.parse().ok().map(Value::Byte),
        Some('s') if integer => digits.parse().ok().map(Value::Short),
        Some('l') if integer => digits.parse().ok().map(Value::Long),
        Some('f') if decimal => digits.parse().ok().map(Value::Float),
        Some('d') if decimal => digits.parse().ok().map(Value::Double),
        None if integer => digits.parse().ok().map(Value::Int),
        None if decimal => digits.parse().ok().map(Value::Double),
        _ => None,
    };
    parsed.unwrap_or_else(|| Value::Str(word.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(text: &str) -> Value {
        parse(text).unwrap().0
    }

    #[test]
    fn numbers_carry_their_suffix() {
        assert_eq!(ok("3"), Value::Int(3));
        assert_eq!(ok("-3"), Value::Int(-3));
        assert_eq!(ok("3b"), Value::Byte(3));
        assert_eq!(ok("3B"), Value::Byte(3));
        assert_eq!(ok("3s"), Value::Short(3));
        assert_eq!(ok("3L"), Value::Long(3));
        assert_eq!(ok("1.5f"), Value::Float(1.5));
        assert_eq!(ok("1.5d"), Value::Double(1.5));
        assert_eq!(ok("1.5"), Value::Double(1.5));
        assert_eq!(ok("true"), Value::Byte(1));
        assert_eq!(ok("false"), Value::Byte(0));
    }

    #[test]
    fn a_word_that_is_not_a_number_is_a_string() {
        assert_eq!(ok("abc"), Value::Str("abc".into()));
        assert_eq!(ok("minecraft:chest"[..9].trim_end_matches(':')), Value::Str("minecraft".into()));
        assert_eq!(ok("1x"), Value::Str("1x".into()));
        assert_eq!(ok("300b"), Value::Str("300b".into()));
    }

    #[test]
    fn strings_unescape() {
        assert_eq!(ok(r#""a \"b\" \\ c""#), Value::Str("a \"b\" \\ c".into()));
        assert_eq!(ok("'it\\'s'"), Value::Str("it's".into()));
        assert_eq!(ok(r#""""#), Value::Str(String::new()));
    }

    #[test]
    fn compounds_and_lists_nest() {
        let v = ok(r#"{"id":"minecraft:sign", x: 1, messages:["a","b"], empty:{}, at:[I;1,2,3], b:[B;1b,2b], l:[L;1L]}"#);
        let Value::Compound(fields) = v else { panic!("compound") };
        let get = |k: &str| fields.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone()).unwrap();
        assert_eq!(get("id"), Value::Str("minecraft:sign".into()));
        assert_eq!(get("x"), Value::Int(1));
        assert_eq!(get("messages"), Value::List(vec![Value::Str("a".into()), Value::Str("b".into())]));
        assert_eq!(get("empty"), Value::Compound(vec![]));
        assert_eq!(get("at"), Value::IntArray(vec![1, 2, 3]));
        assert_eq!(get("b"), Value::ByteArray(vec![1, 2]));
        assert_eq!(get("l"), Value::LongArray(vec![1]));
    }

    #[test]
    fn it_reports_what_it_consumed_and_stops_there() {
        let (v, n) = parse("  {a:1} tail").unwrap();
        assert_eq!(v, Value::Compound(vec![("a".into(), Value::Int(1))]));
        assert_eq!(&"  {a:1} tail"[n..], " tail");
    }

    #[test]
    fn broken_text_is_an_error() {
        assert!(parse("{a:1").is_err());
        assert!(parse("[1,2").is_err());
        assert!(parse("{a 1}").is_err());
        assert!(parse("\"open").is_err());
        assert!(parse("").is_err());
        assert!(parse("[I;1b]").is_err());
    }

    #[test]
    fn a_trailing_comma_and_an_empty_list_are_fine() {
        assert_eq!(ok("[]"), Value::List(vec![]));
        assert_eq!(ok("[1,]"), Value::List(vec![Value::Int(1)]));
        assert_eq!(ok("{a:1,}"), Value::Compound(vec![("a".into(), Value::Int(1))]));
    }

    #[test]
    fn what_the_writer_prints_reads_back() {
        // the shape machines::display_link_target::to_snbt writes
        let text = r#"{"messages":["a \"b\"",""],"is_waxed":0b,"n":3,"at":[I;1,2,3],"f":1.5f,"d":2.0d,"l":7L}"#;
        let Value::Compound(fields) = ok(text) else { panic!("compound") };
        assert_eq!(fields.len(), 7);
        assert_eq!(fields[1], ("is_waxed".into(), Value::Byte(0)));
        assert_eq!(fields[4], ("f".into(), Value::Float(1.5)));
        assert_eq!(fields[5], ("d".into(), Value::Double(2.0)));
    }
}
