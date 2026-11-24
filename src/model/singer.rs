use serde_json::Value;

/// A group with its own singer overrides the track; an unbound group inherits.
pub fn singer_database<'a>(track: &'a Value, instance: &'a Value) -> &'a Value {
    if ["id", "name"].iter().any(|key| {
        instance
            .get(key)
            .is_some_and(|value| value.as_str().is_none_or(|text| !text.is_empty()))
    }) {
        instance
    } else {
        track
    }
}

pub fn inherits_singer_modes(track: &Value, instance: &Value) -> bool {
    let selected = singer_database(track, instance);
    if track == selected {
        return true;
    }
    match (text(track, "id"), text(selected, "id")) {
        (Some(left), Some(right)) => left == right,
        _ => text(track, "name").is_some_and(|name| text(selected, "name") == Some(name)),
    }
}

fn text<'a>(database: &'a Value, key: &str) -> Option<&'a str> {
    database
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}
