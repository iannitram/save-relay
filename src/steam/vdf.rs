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

use crate::error::{not_built, Result};

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
        crate::not_built!("steam::vdf::Vdf::get")
    }

    /// `get` along a path of keys.
    pub fn get_path(&self, keys: &[&str]) -> Option<&Vdf> {
        crate::not_built!("steam::vdf::Vdf::get_path")
    }

    /// The string, for a `Str`.
    pub fn as_str(&self) -> Option<&str> {
        crate::not_built!("steam::vdf::Vdf::as_str")
    }

    /// Set the value at a path of keys, creating `Map`s on the way (matching keys without
    /// case, keeping the first match's spelling); `None` removes that key. A `Str` met on the
    /// way is replaced by a `Map`.
    pub fn set_path(&mut self, keys: &[&str], value: Option<Vdf>) {
        crate::not_built!("steam::vdf::Vdf::set_path")
    }
}

/// Parse a whole file: its top-level pairs as one `Map`. An unclosed quote or block, or a key
/// with no value, is `Error::Io("vdf: line <n>: ...")`.
pub fn parse(text: &str) -> Result<Vdf> {
    not_built("steam::vdf::parse")
}

/// Write a top-level `Map` the way Steam does: one pair per line, tab-indented blocks, values
/// escaped, `\n` line ends, a final newline. `parse(&to_text(v)) == v` for every `Map`.
pub fn to_text(root: &Vdf) -> String {
    crate::not_built!("steam::vdf::to_text")
}
