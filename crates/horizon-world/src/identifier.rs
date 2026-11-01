//! Validated resource identifiers, with Java hashing and path-first ordering.
use crate::java_hash_map::JavaKey;
use std::{cmp::Ordering, fmt, sync::Arc};
#[derive(Debug, PartialEq, Eq)]
struct Data {
    namespace: Box<str>,
    path: Box<str>,
    hash: i32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identifier(Arc<Data>);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentifierError {
    Namespace,
    Path,
}
impl fmt::Display for IdentifierError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Namespace => "invalid identifier namespace",
            Self::Path => "invalid identifier path",
        })
    }
}
impl std::error::Error for IdentifierError {}
fn valid(c: u8) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c)
}
fn hash(s: &str) -> i32 {
    s.bytes()
        .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(i32::from(c)))
}
impl Identifier {
    pub fn from_parts(namespace: &str, path: &str) -> Result<Self, IdentifierError> {
        if !namespace.bytes().all(valid) {
            return Err(IdentifierError::Namespace);
        }
        if !path.bytes().all(|c| valid(c) || c == b'/') {
            return Err(IdentifierError::Path);
        }
        Ok(Self(Arc::new(Data {
            namespace: namespace.into(),
            path: path.into(),
            hash: hash(namespace).wrapping_mul(31).wrapping_add(hash(path)),
        })))
    }
    /// A fully qualified identifier with nonempty namespace and path.
    pub fn parse_namespaced(text: &str) -> Option<Self> {
        let (namespace, path) = text.split_once(':')?;
        if namespace.is_empty() || path.is_empty() {
            return None;
        }
        Self::from_parts(namespace, path).ok()
    }
    pub fn parse(text: &str) -> Result<Self, IdentifierError> {
        let (namespace, path) = text.split_once(':').unwrap_or(("minecraft", text));
        Self::from_parts(
            if namespace.is_empty() {
                "minecraft"
            } else {
                namespace
            },
            path,
        )
    }
    pub fn namespace(&self) -> &str {
        &self.0.namespace
    }
    pub fn path(&self) -> &str {
        &self.0.path
    }
    pub fn hash_code(&self) -> i32 {
        self.0.hash
    }
}
impl Ord for Identifier {
    fn cmp(&self, other: &Self) -> Ordering {
        self.path()
            .cmp(other.path())
            .then_with(|| self.namespace().cmp(other.namespace()))
    }
}
impl PartialOrd for Identifier {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl fmt::Display for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.namespace(), self.path())
    }
}
impl JavaKey for Identifier {
    fn java_hash(&self) -> i32 {
        self.hash_code()
    }
    fn comparable_compare(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
    fn tie_break(&self, other: &Self) -> std::cmp::Ordering {
        self.cmp(other)
    }
}
