// SPDX-License-Identifier: Apache-2.0
//! RFC 8785 JSON Canonicalization Scheme for the independent verifier.
//!
//! This module is a stand-alone implementation. It shares no serializer with
//! the producer. Input is a `serde_json::Value` produced by the strict parser in
//! [`crate::schema`], so duplicate member names, NaN, Infinity and lone
//! surrogates have already been refused before canonicalization.

use serde::Deserialize;
use serde::Deserializer;
use serde::de::Error as DeError;
use serde::de::MapAccess;
use serde::de::SeqAccess;
use serde::de::Visitor;
use serde_json::Map;
use serde_json::Value;
use std::collections::HashSet;
use std::fmt::Write as FmtWrite;

/// A JSON value parsed with rejection of repeated object member names.
///
/// `serde_json::Value` keeps the last value for a repeated member. The verifier
/// must not inherit that behavior, so parsing goes through this wrapper.
#[derive(Debug, Clone)]
pub struct StrictValue(pub Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictValueVisitor).map(Self)
    }
}

struct StrictValueVisitor;

impl<'de> Visitor<'de> for StrictValueVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a JSON value with unique object member names")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(Value::Null)
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(Value::from(value))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(Value::from(value))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(Value::from(value))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Ok(Value::String(value))
    }

    fn visit_seq<A>(self, mut access: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut items = Vec::new();
        while let Some(item) = access.next_element::<StrictValue>()? {
            items.push(item.0);
        }
        Ok(Value::Array(items))
    }

    fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut members = Map::new();
        let mut seen = HashSet::new();
        while let Some(key) = access.next_key::<String>()? {
            if !seen.insert(key.clone()) {
                return Err(A::Error::custom(format!("duplicate JSON member: {}", key)));
            }
            let value = access.next_value::<StrictValue>()?;
            members.insert(key, value.0);
        }
        Ok(Value::Object(members))
    }
}

/// Parse UTF-8 JSON bytes with duplicate-member rejection enabled.
///
/// A parse error is returned verbatim so the caller may classify it as a
/// stage-one schema refusal.
pub fn parse_strict(bytes: &[u8]) -> Result<Value, serde_json::Error> {
    let parsed: StrictValue = serde_json::from_slice(bytes)?;
    Ok(parsed.0)
}

/// Return the RFC 8785 canonical byte sequence for an already parsed value.
pub fn canonical_bytes(value: &Value) -> Vec<u8> {
    let mut output = String::new();
    write_value(&mut output, value);
    output.into_bytes()
}

fn write_value(output: &mut String, value: &Value) {
    match value {
        Value::Null => output.push_str("null"),
        Value::Bool(true) => output.push_str("true"),
        Value::Bool(false) => output.push_str("false"),
        Value::Number(number) => {
            let as_double = number.as_f64().unwrap_or(0.0);
            output.push_str(&canonical_double(as_double));
        }
        Value::String(text) => write_string(output, text),
        Value::Array(items) => {
            output.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                write_value(output, item);
            }
            output.push(']');
        }
        Value::Object(members) => {
            let mut keys: Vec<&String> = members.keys().collect();
            keys.sort_by_key(|left| utf16_key(left));
            output.push('{');
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                write_string(output, key);
                output.push(':');
                write_value(output, &members[*key]);
            }
            output.push('}');
        }
    }
}

fn utf16_key(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn write_string(output: &mut String, text: &str) {
    output.push('"');
    for character in text.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\u{0008}' => output.push_str("\\b"),
            '\u{0009}' => output.push_str("\\t"),
            '\u{000A}' => output.push_str("\\n"),
            '\u{000C}' => output.push_str("\\f"),
            '\u{000D}' => output.push_str("\\r"),
            control if (control as u32) < 0x20 => {
                let _ = write!(output, "\\u{:04x}", control as u32);
            }
            other => output.push(other),
        }
    }
    output.push('"');
}

/// Format a finite double with the ECMAScript `Number::toString` rules used by
/// RFC 8785 section 3.2.2.3.
///
/// Rust's lower-exponent formatter emits the shortest round-trip digits. Those
/// digits and the decimal exponent are rearranged into the four ECMAScript
/// notation cases without allocating a buffer sized from untrusted input.
pub fn canonical_double(value: f64) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    let negative = value < 0.0;
    let magnitude = value.abs();
    let exponent_form = format!("{:e}", magnitude);
    let (significand, exponent_text) = match exponent_form.split_once('e') {
        Some(parts) => parts,
        None => (exponent_form.as_str(), "0"),
    };
    let digits: String = significand.chars().filter(|item| *item != '.').collect();
    let exponent: i32 = exponent_text.parse().unwrap_or_default();
    let count = digits.len() as i32;
    let n = exponent + 1;
    let mut body = String::new();
    if n > 0 && n <= 21 && count <= n {
        body.push_str(&digits);
        for _ in 0..(n - count) {
            body.push('0');
        }
    } else if n > 0 && n <= 21 {
        let split = n as usize;
        body.push_str(&digits[..split]);
        body.push('.');
        body.push_str(&digits[split..]);
    } else if n > -6 && n <= 0 {
        body.push_str("0.");
        for _ in 0..(-n) {
            body.push('0');
        }
        body.push_str(&digits);
    } else {
        body.push_str(&digits[..1]);
        if count > 1 {
            body.push('.');
            body.push_str(&digits[1..]);
        }
        body.push('e');
        if exponent >= 0 {
            body.push('+');
        }
        body.push_str(&exponent.to_string());
    }
    if negative { format!("-{}", body) } else { body }
}

/// Return the maximum container nesting depth of a parsed value.
///
/// A scalar at the document root has depth one. Each nested array or object
/// adds one level.
pub fn json_depth(value: &Value) -> u32 {
    match value {
        Value::Array(items) => 1 + items.iter().map(json_depth).max().unwrap_or(0),
        Value::Object(members) => 1 + members.values().map(json_depth).max().unwrap_or(0),
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::canonical_bytes;
    use super::canonical_double;
    use super::json_depth;
    use super::parse_strict;
    use serde_json::Value;
    use serde_json::json;

    #[test]
    fn rfc8785_number_vector() -> Result<(), Box<dyn std::error::Error>> {
        let awkward: f64 = "333333333.33333329".parse()?;
        let input = json!({"numbers": [awkward, 1E30, 4.50, 2e-3, 0.000000000000000000000000001]});
        let expected = r#"{"numbers":[333333333.3333333,1e+30,4.5,0.002,1e-27]}"#;
        let actual = String::from_utf8(canonical_bytes(&input))?;
        assert_eq!(actual, expected, "RFC 8785 Appendix B number vector");
        Ok(())
    }

    #[test]
    fn rfc8785_integer_and_negative_zero_vectors() {
        assert_eq!(canonical_double(0.0), "0", "positive zero canonical form");
        assert_eq!(canonical_double(-0.0), "0", "negative zero canonical form");
        assert_eq!(canonical_double(1e21), "1e+21", "upper exponent boundary");
        assert_eq!(
            canonical_double(1e20),
            "100000000000000000000",
            "integral boundary"
        );
        assert_eq!(
            canonical_double(9007199254740992.0),
            "9007199254740992",
            "exact integer"
        );
        assert_eq!(canonical_double(-1.5), "-1.5", "negative fraction");
    }

    #[test]
    fn duplicate_member_is_refused() {
        let bytes = br#"{"a":1,"a":2}"#;
        let result = parse_strict(bytes);
        assert!(result.is_err(), "duplicate member must fail strict parsing");
    }

    #[test]
    fn member_order_is_utf16_not_utf8() {
        let input = json!({"\u{e000}": 1, "\u{10000}": 2});
        let actual = String::from_utf8(canonical_bytes(&input)).unwrap_or_default();
        let expected = "{\"𐀀\":2,\"\":1}";
        assert_eq!(
            actual, expected,
            "UTF-16 order places surrogates before U+E000"
        );
    }

    #[test]
    fn json_depth_counts_nested_containers() {
        let value: Value = json!({"a": [1, {"b": 2}]});
        assert_eq!(
            json_depth(&value),
            4,
            "root object, array, inner object, scalar"
        );
    }
}
