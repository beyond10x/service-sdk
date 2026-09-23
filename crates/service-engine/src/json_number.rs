//! JSON number semantics that do not depend on `serde_json`'s `arbitrary_precision` feature.
//!
//! Entity Runtime turns that feature on through Cargo feature unification. With it, a
//! `serde_json::Number` keeps its authored token, so `12.50 != 12.5` and `to_string` returns the
//! token. Without it, a number is an `i64`, a `u64` or an `f64`. The SDK compares and prints
//! numbers the way the build without the feature does, so behavior does not change with the
//! dependency graph.

use serde_json::{Number, Value};

/// Equality of two JSON values as a `serde_json` build without `arbitrary_precision` decides it.
pub fn json_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => {
            FeatureIndependent::of(left) == FeatureIndependent::of(right)
        }
        (Value::Array(left), Value::Array(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| json_equal(left, right))
        }
        (Value::Object(left), Value::Object(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .all(|(key, left)| right.get(key).is_some_and(|right| json_equal(left, right)))
        }
        (left, right) => left == right,
    }
}

/// The three number representations of a build without the feature; distinct kinds never equal.
#[derive(PartialEq)]
enum FeatureIndependent {
    Integer(i128),
    Binary(f64),
    Unrepresentable(String),
}

impl FeatureIndependent {
    fn of(value: &Number) -> Self {
        if let Some(integer) = value.as_i64() {
            Self::Integer(i128::from(integer))
        } else if let Some(integer) = value.as_u64() {
            Self::Integer(i128::from(integer))
        } else {
            value
                .as_f64()
                .filter(|binary| binary.is_finite())
                .map_or_else(|| Self::Unrepresentable(value.to_string()), Self::Binary)
        }
    }
}

/// This value with every number rewritten as a `serde_json` build without `arbitrary_precision`
/// holds it, so its serialized bytes are the same in both builds.
pub(crate) fn feature_independent_value(value: &Value) -> Value {
    match value {
        Value::Number(number) => Value::Number(match FeatureIndependent::of(number) {
            FeatureIndependent::Integer(_) => number
                .as_i64()
                .map_or_else(
                    || number.as_u64().map(Number::from),
                    |integer| Some(Number::from(integer)),
                )
                .unwrap_or_else(|| number.clone()),
            FeatureIndependent::Binary(binary) => {
                Number::from_f64(binary).unwrap_or_else(|| number.clone())
            }
            FeatureIndependent::Unrepresentable(_) => number.clone(),
        }),
        Value::Array(items) => Value::Array(items.iter().map(feature_independent_value).collect()),
        Value::Object(members) => Value::Object(
            members
                .iter()
                .map(|(key, item)| (key.clone(), feature_independent_value(item)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// The text a `serde_json` build without `arbitrary_precision` prints for this number.
pub(crate) fn feature_independent_number(value: &Number) -> String {
    if let Some(integer) = value.as_i64() {
        return integer.to_string();
    }
    if let Some(integer) = value.as_u64() {
        return integer.to_string();
    }
    value
        .as_f64()
        .and_then(Number::from_f64)
        .map_or_else(|| value.to_string(), |binary| binary.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn number_equality_is_the_same_in_both_serde_json_builds() {
        for (left, right) in [
            ("12.50", "12.5"),
            ("1e2", "100.0"),
            ("1.0E2", "100.0"),
            ("123456789012345678901", "123456789012345680000"),
            ("0.1000000000000000055511151231257827", "0.1"),
            (
                r#"{"total": {"amount": 12.50}}"#,
                r#"{"total": {"amount": 12.5}}"#,
            ),
            ("[1e2, 7]", "[100.0, 7]"),
        ] {
            assert!(json_equal(&parse(left), &parse(right)), "{left} == {right}");
        }
        for (left, right) in [
            ("100", "100.0"),
            ("1e2", "100"),
            ("7", "8"),
            ("-3", "3"),
            (r#"{"a": 1}"#, r#"{"a": 1, "b": 2}"#),
            ("12.5", r#""12.5""#),
        ] {
            assert!(
                !json_equal(&parse(left), &parse(right)),
                "{left} != {right}"
            );
        }
    }
}
