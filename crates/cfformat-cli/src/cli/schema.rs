//! `cfformat settings --schema`: a JSON Schema for `.cfformat.json`,
//! generated from the settings reference (`options::reference()`).

use cfformat::options::{reference, Kind};
use cfformat::Options;
use serde_json::{json, Map, Value};

/// Integer keys that validation requires to be at least 1.
const AT_LEAST_ONE: &[&str] = &["indent_size", "max_columns"];

/// `struct.separator`: `:` or `=` with at most one space on each side.
const SEPARATOR_PATTERN: &str = "^ ?[:=] ?$";

/// The schema (draft 2020-12): one property per key with its description,
/// type, domain and default; unknown keys are not allowed, so a removed or
/// renamed key (which the CLI still migrates, with a warning) is flagged.
pub fn schema() -> Value {
    let defaults = match serde_json::to_value(Options::default()) {
        Ok(Value::Object(map)) => map,
        _ => unreachable!("options serialise to an object"),
    };
    let mut properties = Map::new();
    for info in reference() {
        let mut property = match info.kind {
            Kind::Bool => json!({ "type": "boolean" }),
            Kind::Integer => json!({
                "type": "integer",
                "minimum": if AT_LEAST_ONE.contains(&info.key) { 1 } else { 0 },
            }),
            Kind::Enum(values) => json!({ "type": "string", "enum": values }),
            Kind::Separator => json!({ "type": "string", "pattern": SEPARATOR_PATTERN }),
        };
        let object = property.as_object_mut().expect("an object");
        object.insert("description".into(), info.description.into());
        if let Some(default) = defaults.get(info.key) {
            object.insert("default".into(), default.clone());
        }
        properties.insert(info.key.into(), property);
    }
    sorted(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": ".cfformat.json",
        "description": "cfformat settings",
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
    }))
}

/// `value` with every object's keys in order (`serde_json` preserves
/// insertion order in this workspace).
fn sorted(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(entries.into_iter().map(|(k, v)| (k, sorted(v))).collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}
