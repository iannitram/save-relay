//! Pure parsing for Steam's public appdetails answer (ADR 0004). Network access and caching
//! belong to F73; this module only builds the request URL and interprets one answer.

use crate::error::{Error, Result};
use serde_json::Value;

/// Steam's public appdetails endpoint for one app, requesting the fields this parser needs.
pub fn appdetails_url(app_id: u32) -> String {
    format!(
        "https://store.steampowered.com/api/appdetails?appids={app_id}&filters=basic,categories"
    )
}

/// What the public store says about one app id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreAnswer {
    /// A store game, and whether its categories include Steam Cloud (23).
    Game { cloud: bool },
    /// A successful answer for a product whose type is not `game`.
    NotAGame,
    /// Steam returned `success: false`; the app type is unknown.
    Unknown,
}

/// Parse one appdetails response. Invalid JSON or a missing app id is a state error.
pub fn parse_appdetails(text: &str, app_id: u32) -> Result<StoreAnswer> {
    let invalid = |reason: &str| Error::StateInvalid(format!("steam store: {reason}"));
    let root: Value = serde_json::from_str(text).map_err(|_| invalid("invalid JSON"))?;
    let answer = root
        .get(app_id.to_string())
        .ok_or_else(|| invalid("missing app id"))?;
    if answer.get("success").and_then(Value::as_bool) == Some(false) {
        return Ok(StoreAnswer::Unknown);
    }
    if answer.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(invalid("missing success value"));
    }
    let data = answer.get("data").ok_or_else(|| invalid("missing data"))?;
    let kind = data
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("missing or invalid type"))?;
    if kind != "game" {
        return Ok(StoreAnswer::NotAGame);
    }
    let categories = match data.get("categories") {
        None => None,
        Some(value) => Some(
            value
                .as_array()
                .ok_or_else(|| invalid("invalid categories"))?,
        ),
    };
    let cloud = categories.is_some_and(|categories| {
        categories
            .iter()
            .any(|category| category.get("id").and_then(Value::as_u64) == Some(23))
    });
    Ok(StoreAnswer::Game { cloud })
}
