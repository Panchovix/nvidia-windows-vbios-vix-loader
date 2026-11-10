//! A small JSON value with insertion-ordered objects, printed exactly like Python's
//! `json.dumps(value, indent=2)` (ensure_ascii: every character outside ' '..'~' is escaped as \uXXXX, with
//! lowercase hex and surrogate pairs above U+FFFF).
//!
//! The reports of `operations` are these values, built in the same key order as the Python dicts, so
//! `--json` prints the same bytes as the Python loader did. Assigning an existing key keeps its position, as
//! assigning to a Python dict does.

use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Str(String),
    Array(Vec<Value>),
    Object(Map),
}

/// An object: keys in insertion order.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Map {
    entries: Vec<(String, Value)>,
}

impl Map {
    pub fn new() -> Self {
        Map { entries: Vec::new() }
    }

    /// `map[key] = value`: replaces the value in place when the key exists, else appends.
    pub fn insert(&mut self, key: &str, value: impl Into<Value>) {
        let value = value.into();
        match self.entries.iter_mut().find(|(k, _)| k == key) {
            Some(entry) => entry.1 = value,
            None => self.entries.push((key.to_string(), value)),
        }
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.entries.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Removes a key; the others keep their order.
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        let position = self.entries.iter().position(|(k, _)| k == key)?;
        Some(self.entries.remove(position).1)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(k, _)| k.as_str())
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }
}

impl Value {
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Vec<Value>> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&Map> {
        match self {
            Value::Object(m) => Some(m),
            _ => None,
        }
    }

    pub fn as_object_mut(&mut self) -> Option<&mut Map> {
        match self {
            Value::Object(m) => Some(m),
            _ => None,
        }
    }

    /// `value[key]` of an object; None for a missing key or a value that is not an object.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_object().and_then(|m| m.get(key))
    }

    /// Python truthiness, as `if report.get('message'):` uses it: null, false, 0, "", [] and {} are false.
    pub fn truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Str(s) => !s.is_empty(),
            Value::Array(a) => !a.is_empty(),
            Value::Object(m) => !m.is_empty(),
        }
    }

    /// `json.dumps(self, indent=2)`, without a trailing newline.
    pub fn to_json_pretty(&self) -> String {
        let mut out = String::new();
        write_value(&mut out, self, 0);
        out
    }
}

fn write_indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push_str("  ");
    }
}

fn write_value(out: &mut String, value: &Value, level: usize) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Int(i) => {
            let _ = write!(out, "{i}");
        }
        Value::Str(s) => write_string(out, s),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                out.push_str(if i == 0 { "\n" } else { ",\n" });
                write_indent(out, level + 1);
                write_value(out, item, level + 1);
            }
            out.push('\n');
            write_indent(out, level);
            out.push(']');
        }
        Value::Object(map) => {
            if map.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            for (i, (key, item)) in map.entries.iter().enumerate() {
                out.push_str(if i == 0 { "\n" } else { ",\n" });
                write_indent(out, level + 1);
                write_string(out, key);
                out.push_str(": ");
                write_value(out, item, level + 1);
            }
            out.push('\n');
            write_indent(out, level);
            out.push('}');
        }
    }
}

/// A JSON string as Python's ensure_ascii encoder writes it.
fn write_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ' '..='~' => out.push(c),
            _ => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    let _ = write!(out, "\\u{unit:04x}");
                }
            }
        }
    }
    out.push('"');
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::Int(v)
    }
}

impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Value::Int(v as i64)
    }
}

impl From<u32> for Value {
    fn from(v: u32) -> Self {
        Value::Int(v as i64)
    }
}

impl From<usize> for Value {
    fn from(v: usize) -> Self {
        Value::Int(v as i64)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::Str(v.to_string())
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::Str(v)
    }
}

impl From<&String> for Value {
    fn from(v: &String) -> Self {
        Value::Str(v.clone())
    }
}

impl From<Map> for Value {
    fn from(v: Map) -> Self {
        Value::Object(v)
    }
}

impl From<Vec<Value>> for Value {
    fn from(v: Vec<Value>) -> Self {
        Value::Array(v)
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Self {
        v.map_or(Value::Null, Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_indent_2_layout() {
        let mut inner = Map::new();
        inner.insert("size", 3usize);
        inner.insert("list", Value::Array(vec![]));
        inner.insert("map", Map::new());
        let mut map = Map::new();
        map.insert("a", Value::Null);
        map.insert("b", true);
        map.insert("c", inner);
        map.insert("d", Value::Array(vec![1i64.into(), "x".into()]));
        let expected = "{\n  \"a\": null,\n  \"b\": true,\n  \"c\": {\n    \"size\": 3,\n    \"list\": [],\n    \
                        \"map\": {}\n  },\n  \"d\": [\n    1,\n    \"x\"\n  ]\n}";
        assert_eq!(Value::Object(map).to_json_pretty(), expected);
        assert_eq!(Value::Array(vec![]).to_json_pretty(), "[]");
    }

    #[test]
    fn ensure_ascii_escapes() {
        let text = "a\"b\\c\n\r\t\u{08}\u{0c}\u{01}\u{7f}é€😀~ /";
        assert_eq!(
            Value::from(text).to_json_pretty(),
            "\"a\\\"b\\\\c\\n\\r\\t\\b\\f\\u0001\\u007f\\u00e9\\u20ac\\ud83d\\ude00~ /\""
        );
    }

    #[test]
    fn assignment_keeps_the_position() {
        let mut map = Map::new();
        map.insert("x", 1i64);
        map.insert("y", 2i64);
        map.insert("x", 3i64);
        assert_eq!(map.keys().collect::<Vec<_>>(), ["x", "y"]);
        assert_eq!(map.get("x"), Some(&Value::Int(3)));
        assert_eq!(map.remove("x"), Some(Value::Int(3)));
        assert_eq!(map.keys().collect::<Vec<_>>(), ["y"]);
    }
}
