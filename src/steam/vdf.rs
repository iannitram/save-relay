//! Valve's KeyValues text format (`.vdf`, `.acf`), as Steam writes `libraryfolders.vdf`,
//! `appmanifest_<id>.acf` and `localconfig.vdf`: quoted keys and values, `{ }` blocks, `//`
//! comments, `\"` `\\` `\n` `\t` escapes. Order and repeated keys are kept, so a file read and
//! written again changes only what was set. Pure.
//!
//! ```text
//! "libraryfolders"
//! {
//!     "0"
//!     {
//!         "path"      "/home/user/.local/share/Steam"
//!         "apps"      { "1659420"  "51539607552" }
//!     }
//! }
//! ```

use crate::error::{Error, Result};

/// One value: a string or a block of key-value pairs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Vdf {
    Str(String),
    /// In file order; keys may repeat.
    Map(Vec<(String, Vdf)>),
}

impl Vdf {
    /// The first value under `key` in a `Map`, keys compared without ASCII case (Steam writes
    /// both `apps` and `Apps`). `None` for a `Str`.
    pub fn get(&self, key: &str) -> Option<&Vdf> {
        match self {
            Vdf::Map(pairs) => pairs
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v),
            Vdf::Str(_) => None,
        }
    }

    /// `get` along a path of keys.
    pub fn get_path(&self, keys: &[&str]) -> Option<&Vdf> {
        let mut value = self;
        for key in keys {
            value = value.get(key)?;
        }
        Some(value)
    }

    /// The string, for a `Str`.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Vdf::Str(s) => Some(s),
            Vdf::Map(_) => None,
        }
    }

    /// Set the value at a path of keys, creating `Map`s on the way (matching keys without
    /// case, keeping the first match's spelling); `None` removes that key. A `Str` met on the
    /// way is replaced by a `Map`.
    pub fn set_path(&mut self, keys: &[&str], value: Option<Vdf>) {
        if keys.is_empty() {
            if let Some(value) = value {
                *self = value;
            }
            return;
        }
        match value {
            Some(value) => set_value(self, keys, value),
            None => remove_key(self, keys),
        }
    }
}

/// The pair list of `value`, turning a `Str` into an empty `Map` first.
fn map_mut(value: &mut Vdf) -> &mut Vec<(String, Vdf)> {
    if !matches!(value, Vdf::Map(_)) {
        *value = Vdf::Map(Vec::new());
    }
    if let Vdf::Map(pairs) = value {
        pairs
    } else {
        unreachable!("the value was just turned into a Map")
    }
}

fn set_value(value: &mut Vdf, keys: &[&str], new: Vdf) {
    let key = keys[0];
    let pairs = map_mut(value);
    let found = pairs
        .iter()
        .position(|(existing, _)| existing.eq_ignore_ascii_case(key));
    if keys.len() == 1 {
        match found {
            Some(index) => pairs[index].1 = new,
            None => pairs.push((key.to_string(), new)),
        }
        return;
    }
    let index = match found {
        Some(index) => index,
        None => {
            pairs.push((key.to_string(), Vdf::Map(Vec::new())));
            pairs.len() - 1
        }
    };
    set_value(&mut pairs[index].1, &keys[1..], new);
}

fn remove_key(value: &mut Vdf, keys: &[&str]) {
    let Vdf::Map(pairs) = value else {
        return;
    };
    let key = keys[0];
    let Some(index) = pairs
        .iter()
        .position(|(existing, _)| existing.eq_ignore_ascii_case(key))
    else {
        return;
    };
    if keys.len() == 1 {
        pairs.remove(index);
    } else {
        remove_key(&mut pairs[index].1, &keys[1..]);
    }
}

/// Parse a whole file: its top-level pairs as one `Map`. An unclosed quote or block, or a key
/// with no value, is `Error::Io("vdf: line <n>: ...")`.
pub fn parse(text: &str) -> Result<Vdf> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        pos: 0,
        line: 1,
    };
    Ok(Vdf::Map(parser.pairs(None)?))
}

/// Write a top-level `Map` the way Steam does: one pair per line, tab-indented blocks, values
/// escaped, `\n` line ends, a final newline. `parse(&to_text(v)) == v` for every `Map`.
pub fn to_text(root: &Vdf) -> String {
    let mut out = String::new();
    match root {
        Vdf::Map(pairs) => write_pairs(&mut out, pairs, 0),
        Vdf::Str(s) => {
            write_quoted(&mut out, s);
            out.push('\n');
        }
    }
    out
}

fn write_pairs(out: &mut String, pairs: &[(String, Vdf)], depth: usize) {
    for (key, value) in pairs {
        write_indent(out, depth);
        write_quoted(out, key);
        match value {
            Vdf::Str(s) => {
                out.push_str("\t\t");
                write_quoted(out, s);
                out.push('\n');
            }
            Vdf::Map(inner) => {
                out.push('\n');
                write_indent(out, depth);
                out.push_str("{\n");
                write_pairs(out, inner, depth + 1);
                write_indent(out, depth);
                out.push_str("}\n");
            }
        }
    }
}

fn write_indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push('\t');
    }
}

fn write_quoted(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
}

enum Kind {
    Str(String),
    Open,
    Close,
}

struct Token {
    kind: Kind,
    line: usize,
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
    line: usize,
}

impl Parser<'_> {
    fn pairs(&mut self, open_line: Option<usize>) -> Result<Vec<(String, Vdf)>> {
        let mut pairs = Vec::new();
        loop {
            let Some(token) = self.token()? else {
                return match open_line {
                    Some(line) => Err(Error::Io(format!("vdf: line {line}: unclosed block"))),
                    None => Ok(pairs),
                };
            };
            match token.kind {
                Kind::Close => {
                    return match open_line {
                        Some(_) => Ok(pairs),
                        None => Err(Error::Io(format!(
                            "vdf: line {}: '}}' with no block",
                            token.line
                        ))),
                    };
                }
                Kind::Open => {
                    return Err(Error::Io(format!(
                        "vdf: line {}: '{{' with no key",
                        token.line
                    )));
                }
                Kind::Str(key) => {
                    let Some(next) = self.token()? else {
                        return Err(Error::Io(format!(
                            "vdf: line {}: key with no value",
                            token.line
                        )));
                    };
                    match next.kind {
                        Kind::Str(value) => pairs.push((key, Vdf::Str(value))),
                        Kind::Open => {
                            let inner = self.pairs(Some(next.line))?;
                            pairs.push((key, Vdf::Map(inner)));
                        }
                        Kind::Close => {
                            return Err(Error::Io(format!(
                                "vdf: line {}: key with no value",
                                token.line
                            )));
                        }
                    }
                }
            }
        }
    }

    fn token(&mut self) -> Result<Option<Token>> {
        self.skip();
        let Some(byte) = self.peek() else {
            return Ok(None);
        };
        let line = self.line;
        match byte {
            b'{' => {
                self.pos += 1;
                Ok(Some(Token {
                    kind: Kind::Open,
                    line,
                }))
            }
            b'}' => {
                self.pos += 1;
                Ok(Some(Token {
                    kind: Kind::Close,
                    line,
                }))
            }
            b'"' => {
                self.pos += 1;
                let mut out = Vec::new();
                loop {
                    let Some(byte) = self.peek() else {
                        return Err(Error::Io(format!("vdf: line {line}: unclosed quote")));
                    };
                    self.pos += 1;
                    match byte {
                        b'"' => break,
                        b'\\' => {
                            let Some(escaped) = self.peek() else {
                                return Err(Error::Io(format!("vdf: line {line}: unclosed quote")));
                            };
                            self.pos += 1;
                            match escaped {
                                b'"' => out.push(b'"'),
                                b'\\' => out.push(b'\\'),
                                b'n' => out.push(b'\n'),
                                b't' => out.push(b'\t'),
                                other => {
                                    if other == b'\n' {
                                        self.line += 1;
                                    }
                                    out.push(b'\\');
                                    out.push(other);
                                }
                            }
                        }
                        other => {
                            if other == b'\n' {
                                self.line += 1;
                            }
                            out.push(other);
                        }
                    }
                }
                Ok(Some(Token {
                    kind: Kind::Str(String::from_utf8(out).expect("vdf text is UTF-8")),
                    line,
                }))
            }
            _ => {
                let mut out = Vec::new();
                while let Some(byte) = self.peek() {
                    if byte.is_ascii_whitespace() || byte == b'{' || byte == b'}' {
                        break;
                    }
                    out.push(byte);
                    self.pos += 1;
                }
                Ok(Some(Token {
                    kind: Kind::Str(String::from_utf8(out).expect("vdf text is UTF-8")),
                    line,
                }))
            }
        }
    }

    fn skip(&mut self) {
        loop {
            while let Some(byte) = self.peek() {
                if !byte.is_ascii_whitespace() {
                    break;
                }
                if byte == b'\n' {
                    self.line += 1;
                }
                self.pos += 1;
            }
            if !self.bytes[self.pos..].starts_with(b"//") {
                return;
            }
            while let Some(byte) = self.peek() {
                self.pos += 1;
                if byte == b'\n' {
                    self.line += 1;
                    break;
                }
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }
}
