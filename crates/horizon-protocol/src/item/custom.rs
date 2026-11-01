//! Internal wire IDs for native data components. Installed once before networking.
use super::{Component, Stack, Template, component};
use crate::buf::{Codec, Reader};
use std::sync::OnceLock;

struct Types {
    first: i32,
    names: Vec<String>,
}

static TYPES: OnceLock<Types> = OnceLock::new();

#[derive(Debug, PartialEq, Eq)]
pub enum InstallError {
    InvalidRange,
    DuplicateName,
    InvalidName,
    AlreadyInstalled,
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidRange => "invalid component ID range",
            Self::DuplicateName => "duplicate component name",
            Self::InvalidName => "invalid component name",
            Self::AlreadyInstalled => "component types already installed",
        })
    }
}
impl std::error::Error for InstallError {}

/// Install sorted component names after the built-in component IDs.
/// This is an internal transport seam, not a native-mod API.
#[doc(hidden)]
pub fn install(first: i32, mut names: Vec<String>) -> Result<(), InstallError> {
    if first < 0
        || i32::try_from(names.len())
            .ok()
            .and_then(|n| first.checked_add(n))
            .is_none()
    {
        return Err(InstallError::InvalidRange);
    }
    if names.iter().any(|name| {
        horizon_world::identifier::Identifier::parse_namespaced(name)
            .is_none_or(|id| id.namespace() == "minecraft")
    }) {
        return Err(InstallError::InvalidName);
    }
    names.sort();
    if names.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(InstallError::DuplicateName);
    }
    TYPES
        .set(Types { first, names })
        .map_err(|_| InstallError::AlreadyInstalled)
}

/// Validate an opaque value against the same limits used by the network reader.
pub fn validate(value: &horizon_world::nbt::Tag) -> crate::Result<()> {
    if matches!(value, horizon_world::nbt::Tag::End) {
        return Err(crate::Error::new("a component value cannot be NBT End"));
    }
    let bytes = horizon_world::nbt::write_network(
        value,
        horizon_world::nbt::Limits {
            max_bytes: crate::buf::NBT_QUOTA,
            ..Default::default()
        },
    )
    .map_err(|error| crate::Error::new(error.to_string()))?;
    Reader::new(&bytes).nbt()?;
    // Disk NBT canonicalizes NaNs, and its reader normalizes negative zero.
    // Reject values that would change when saved and reloaded.
    let limits = horizon_world::nbt::Limits {
        max_bytes: crate::buf::NBT_QUOTA + 2,
        ..Default::default()
    };
    let named = horizon_world::nbt::NamedTag {
        name: "".into(),
        tag: value.clone(),
    };
    let saved = horizon_world::nbt::write_named(&named, limits)
        .map_err(|error| crate::Error::new(error.to_string()))?;
    let (loaded, _) = horizon_world::nbt::read_named(&saved, limits)
        .map_err(|error| crate::Error::new(error.to_string()))?;
    if loaded.tag != *value {
        return Err(crate::Error::new(
            "component value changes on save and reload",
        ));
    }
    Ok(())
}

pub(crate) fn contains(id: i32) -> bool {
    name(id).is_some()
}

/// Check a decoded stack before converting it on an unverified connection.
#[doc(hidden)]
pub fn permitted(stack: &Stack, negotiated: bool) -> crate::Result<bool> {
    if negotiated || entries().next().is_none() {
        return Ok(true);
    }
    fn contains_custom(stack: &Stack) -> crate::Result<bool> {
        if stack.removed.iter().any(|id| contains(*id)) {
            return Ok(true);
        }
        for (id, value) in &stack.added {
            if contains(*id) {
                return Ok(true);
            }
            match value {
                Component::Stacks(stacks) => {
                    for stack in stacks {
                        if contains_custom(stack)? {
                            return Ok(true);
                        }
                    }
                }
                Component::TooltipDisplay { hidden, .. }
                    if hidden.iter().any(|id| contains(*id)) =>
                {
                    return Ok(true);
                }
                Component::Raw(bytes)
                    if matches!(
                        *id,
                        component::USE_REMAINDER
                            | component::POT_DECORATIONS
                            | component::CONTAINER
                            | component::SULFUR_CUBE_CONTENT
                    ) =>
                {
                    let mut reader = Reader::new(bytes);
                    let count = match *id {
                        component::USE_REMAINDER | component::SULFUR_CUBE_CONTENT => 1,
                        component::POT_DECORATIONS => 4,
                        _ => reader.count(256)?,
                    };
                    for _ in 0..count {
                        if (matches!(
                            *id,
                            component::USE_REMAINDER | component::SULFUR_CUBE_CONTENT
                        ) || reader.bool()?)
                            && contains_custom(&Template::read(&mut reader)?)?
                        {
                            return Ok(true);
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(false)
    }
    Ok(!contains_custom(stack)?)
}

#[doc(hidden)]
pub fn name(id: i32) -> Option<&'static str> {
    let types = TYPES.get()?;
    let index = usize::try_from(id.checked_sub(types.first)?).ok()?;
    types.names.get(index).map(String::as_str)
}

#[doc(hidden)]
pub fn id(name: &str) -> Option<i32> {
    let types = TYPES.get()?;
    let index = types
        .names
        .binary_search_by(|n| n.as_str().cmp(name))
        .ok()?;
    types.first.checked_add(i32::try_from(index).ok()?)
}

#[doc(hidden)]
pub fn entries() -> impl Iterator<Item = (i32, &'static str)> {
    TYPES.get().into_iter().flat_map(|types| {
        types.names.iter().enumerate().filter_map(|(index, name)| {
            Some((
                types.first.checked_add(i32::try_from(index).ok()?)?,
                name.as_str(),
            ))
        })
    })
}
