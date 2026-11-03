//! JSON data kept as its text and read into a [`Value`] the first time it is
//! used. A `Value` holds a B-tree node (~0.5 KB) per object: data packs'
//! small, many files (loot tables, advancement criteria) took ~10× their
//! JSON's size, most of it never read.
use serde_json::Value;
use std::sync::OnceLock;

pub struct LazyJson {
    json: Box<str>,
    value: OnceLock<Value>,
}
impl LazyJson {
    /// `value`'s JSON, read back on first use.
    pub fn new(value: &Value) -> Self {
        Self {
            json: value.to_string().into_boxed_str(),
            value: OnceLock::new(),
        }
    }
    /// The value, read once and kept.
    pub fn get(&self) -> &Value {
        self.value
            .get_or_init(|| serde_json::from_str(&self.json).expect("written from a Value"))
    }
    /// The value without keeping it: for a pass over every entry (an
    /// index) that would otherwise keep them all.
    pub fn read(&self) -> std::borrow::Cow<'_, Value> {
        match self.value.get() {
            Some(value) => std::borrow::Cow::Borrowed(value),
            None => std::borrow::Cow::Owned(
                serde_json::from_str(&self.json).expect("written from a Value"),
            ),
        }
    }
}
impl std::ops::Deref for LazyJson {
    type Target = Value;
    fn deref(&self) -> &Value {
        self.get()
    }
}
impl From<Value> for LazyJson {
    fn from(value: Value) -> Self {
        let lazy = Self::new(&value);
        // Already read: keep it rather than read it again.
        let _ = lazy.value.set(value);
        lazy
    }
}
impl Clone for LazyJson {
    fn clone(&self) -> Self {
        Self {
            json: self.json.clone(),
            value: self.value.clone(),
        }
    }
}
impl std::fmt::Debug for LazyJson {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read back as written: floats, integers past 2^53, escapes, nesting,
    /// and `read` gives the same value without keeping it.
    #[test]
    fn reads_back_as_written() {
        let value: Value = serde_json::from_str(
            r#"{"rolls": 1.5, "min": -2.25e-7, "max": 1e300, "weight": 9007199254740993,
            "name": "é\n\"", "z": null, "a": [true, false, {"b": []}]}"#,
        )
        .unwrap();
        let lazy = LazyJson::new(&value);
        assert_eq!(*lazy.read(), value);
        assert!(lazy.value.get().is_none());
        assert_eq!(lazy.get(), &value);
        assert_eq!(lazy["a"][2]["b"], Value::Array(vec![]));
        assert_eq!(*lazy.clone(), value);
    }
}
