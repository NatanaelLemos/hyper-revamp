//! JS-like lenient decoding for config structs.
//!
//! The Electron app never type-validated the config: `fixConfigDefaults`
//! (`app/config.ts`) and the renderer's reducers read each key independently
//! and ignored anything they couldn't use, so one odd value never disturbed
//! the rest of the file. Deriving `Deserialize` gives the opposite behavior —
//! a single mistyped field fails the whole struct — so every config struct
//! decodes through `Fields` instead: unknown keys are kept out of the way,
//! malformed keys are dropped with a warning, and the rest of the document
//! still applies.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// The raw fields of one JSON object, decoded on demand.
pub struct Fields {
    map: Map<String, Value>,
    context: &'static str,
}

impl Fields {
    /// Decode a field. `None` when absent, JSON `null`, or malformed — a
    /// malformed value is reported once and then ignored.
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let value = self.map.get(key)?;
        if value.is_null() {
            return None;
        }
        match T::deserialize(value.clone()) {
            Ok(decoded) => Some(decoded),
            Err(err) => {
                log::warn!(
                    "{}: ignoring malformed `{key}` ({}): {err}",
                    self.context,
                    type_name(value)
                );
                None
            }
        }
    }

    /// Decode a field, falling back to `default` when unusable.
    pub fn get_or<T: DeserializeOwned>(&self, key: &str, default: T) -> T {
        self.get(key).unwrap_or(default)
    }

    /// A number that may be written as a JSON string (`"2222"`), which the
    /// untyped JS path accepted.
    pub fn get_loose_number<T>(&self, key: &str) -> Option<T>
    where
        T: DeserializeOwned + std::str::FromStr,
    {
        if let Some(value) = self.get::<T>(key) {
            return Some(value);
        }
        self.map
            .get(key)?
            .as_str()?
            .trim()
            .parse::<T>()
            .ok()
    }

    pub fn raw(&self, key: &str) -> Option<&Value> {
        self.map.get(key)
    }

    pub fn into_map(self) -> Map<String, Value> {
        self.map
    }
}

/// Pull a struct's fields out of a deserializer. A non-object decodes as an
/// empty set of fields (all defaults) rather than an error, mirroring JS
/// property access on a non-object.
pub fn fields<'de, D>(deserializer: D, context: &'static str) -> Result<Fields, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    match value {
        Value::Object(map) => Ok(Fields { map, context }),
        other => {
            log::warn!("{context}: expected an object, found {}; ignoring", type_name(&other));
            Ok(Fields {
                map: Map::new(),
                context,
            })
        }
    }
}
