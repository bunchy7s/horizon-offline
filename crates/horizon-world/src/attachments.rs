//! Typed native-mod data attached to server-owned holders.
use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use crate::nbt::{Compound, JavaString, Tag};

/// An attachment operation could not be completed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AttachmentError {
    /// The holder does not exist or is unavailable on this side.
    MissingHolder,
    /// The key was used with a different Rust type.
    TypeMismatch,
    /// The saved value could not be decoded.
    Decode,
    /// The value could not be encoded.
    Encode,
    /// A namespace in the saved compound is not a compound.
    InvalidNamespace,
    /// Copy-on-death requires a serializer.
    InvalidCopyPolicy,
    /// Syncing to clients requires a serializer.
    InvalidSyncPolicy,
}

/// NBT key reserved for server-owned attachment maps.
pub const ATTACHMENTS_KEY: &str = "horizon:attachments";

/// Read the reserved compound without replacing an invalid saved value.
pub fn attachment_compound(tag: Option<&Tag>) -> Result<Option<&Compound>, AttachmentError> {
    match tag {
        None => Ok(None),
        Some(Tag::Compound(compound)) => Ok(Some(compound)),
        Some(_) => Err(AttachmentError::InvalidNamespace),
    }
}

/// A server holder in the host's current dimension.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Holder {
    Entity(u32),
    Player(u128),
    BlockEntity([i32; 3]),
    Chunk([i32; 2]),
    Level,
}

/// Object-safe access to the current server's holders.
pub trait AttachmentHost {
    fn attachments(&mut self, holder: Holder) -> Result<&mut Attachments, AttachmentError>;
}

/// The optional on-disk representation of an attachment.
pub struct AttachmentCodec<T> {
    pub encode: fn(&T) -> Result<Compound, AttachmentError>,
    pub decode: fn(&Compound) -> Result<T, AttachmentError>,
}

/// A mod-defined attachment key and its value's behavior.
pub struct AttachmentType<T: 'static> {
    pub namespace: &'static str,
    pub name: &'static str,
    pub default: fn() -> T,
    pub codec: Option<AttachmentCodec<T>>,
    pub copy_on_death: bool,
    /// Send the value to the clients that see the holder; needs a codec.
    pub sync: bool,
}

impl<T> AttachmentType<T> {
    fn validate_policy(&self) -> Result<(), AttachmentError> {
        if self.copy_on_death && self.codec.is_none() {
            return Err(AttachmentError::InvalidCopyPolicy);
        }
        if self.sync && self.codec.is_none() {
            return Err(AttachmentError::InvalidSyncPolicy);
        }
        Ok(())
    }
}

/// One attachment's new state for a client; `None` means removed.
#[derive(Clone, Debug, PartialEq)]
pub struct AttachmentSync {
    pub namespace: JavaString,
    pub name: JavaString,
    pub value: Option<Compound>,
}

impl<T: Clone + Send + Sync + 'static> AttachmentType<T> {
    /// Read a value, inserting its default if absent; see `get_existing`.
    pub fn get<H: AttachmentHost + ?Sized>(
        &self,
        host: &mut H,
        holder: Holder,
    ) -> Result<T, AttachmentError> {
        host.attachments(holder)?.get(self)
    }
    /// Whether a holder carries a value, like NeoForge's `hasData`; never inserts the default.
    /// API 317.
    pub fn has<H: AttachmentHost + ?Sized>(
        &self,
        host: &mut H,
        holder: Holder,
    ) -> Result<bool, AttachmentError> {
        Ok(host.attachments(holder)?.has(self))
    }
    /// Read a value only if present, like `getExistingData`. API 317.
    pub fn get_existing<H: AttachmentHost + ?Sized>(
        &self,
        host: &mut H,
        holder: Holder,
    ) -> Result<Option<T>, AttachmentError> {
        host.attachments(holder)?.get_existing(self)
    }
    /// Replace a value on an existing holder.
    pub fn set<H: AttachmentHost + ?Sized>(
        &self,
        host: &mut H,
        holder: Holder,
        value: T,
    ) -> Result<(), AttachmentError> {
        host.attachments(holder)?.set(self, value)
    }
    /// Remove a value from an existing holder.
    pub fn remove<H: AttachmentHost + ?Sized>(
        &self,
        host: &mut H,
        holder: Holder,
    ) -> Result<Option<T>, AttachmentError> {
        host.attachments(holder)?.remove(self)
    }
    /// Send the current value to the clients that see the holder.
    pub fn sync_data<H: AttachmentHost + ?Sized>(
        &self,
        host: &mut H,
        holder: Holder,
    ) -> Result<(), AttachmentError> {
        host.attachments(holder)?.mark_sync(self)
    }
}

trait Value: Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn clone_box(&self) -> Box<dyn Value>;
    fn encoded(&self) -> Option<&Compound>;
    fn copy_on_death(&self) -> bool;
    fn sync(&self) -> bool;
}
struct TypedValue<T> {
    value: T,
    encoded: Option<Compound>,
    copy_on_death: bool,
    sync: bool,
}
impl<T: Clone + Send + Sync + 'static> Value for TypedValue<T> {
    fn as_any(&self) -> &dyn Any {
        &self.value
    }
    fn clone_box(&self) -> Box<dyn Value> {
        Box::new(Self {
            value: self.value.clone(),
            encoded: self.encoded.clone(),
            copy_on_death: self.copy_on_death,
            sync: self.sync,
        })
    }
    fn encoded(&self) -> Option<&Compound> {
        self.encoded.as_ref()
    }
    fn copy_on_death(&self) -> bool {
        self.copy_on_death
    }
    fn sync(&self) -> bool {
        self.sync
    }
}
/// The saved form of a decoded value: its encoding and the policies it was
/// declared with. `None` when the type has no codec.
fn envelope(value: &dyn Value) -> Option<Compound> {
    value.encoded().map(|encoded| {
        let mut envelope = Compound::new();
        envelope.insert("value".into(), Tag::Compound(encoded.clone()));
        if value.copy_on_death() {
            envelope.insert("copy_on_death".into(), Tag::Byte(1));
        }
        if value.sync() {
            envelope.insert("sync".into(), Tag::Byte(1));
        }
        envelope
    })
}
fn raw_value(raw: &Compound) -> Option<&Compound> {
    match raw.get(&"value".into()) {
        Some(Tag::Compound(value)) => Some(value),
        _ => None,
    }
}

enum Entry {
    Raw(Compound),
    Decoded(Box<dyn Value>),
}
impl Entry {
    /// The encoded value, as clients receive it.
    fn synced_value(&self) -> Option<&Compound> {
        match self {
            Self::Raw(raw) => raw_value(raw),
            Self::Decoded(value) => value.encoded(),
        }
    }
    fn is_synced(&self) -> bool {
        match self {
            Self::Raw(raw) => matches!(raw.get(&"sync".into()), Some(Tag::Byte(1))),
            Self::Decoded(value) => value.sync(),
        }
    }
}
impl Clone for Entry {
    fn clone(&self) -> Self {
        match self {
            Self::Raw(value) => Self::Raw(value.clone()),
            Self::Decoded(value) => Self::Decoded(value.clone_box()),
        }
    }
}

/// The data held by one entity, block entity, chunk or level.
#[derive(Clone, Default)]
pub struct Attachments {
    entries: BTreeMap<(JavaString, JavaString), Entry>,
    // Keep malformed or otherwise unrecognized NBT exactly as it was read.
    raw: Compound,
    // Keys to send to clients at the next flush; never saved.
    pending: BTreeSet<(JavaString, JavaString)>,
}
impl fmt::Debug for Attachments {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Attachments")
            .field("keys", &self.entries.keys())
            .finish()
    }
}

impl Attachments {
    /// Read the namespaced attachment compound, leaving unknown data intact.
    pub fn from_nbt(raw: Compound) -> Self {
        let mut result = Self {
            entries: BTreeMap::new(),
            raw,
            pending: BTreeSet::new(),
        };
        for (namespace, tag) in &result.raw {
            if let Tag::Compound(names) = tag {
                for (name, value) in names {
                    if let Tag::Compound(value) = value {
                        result
                            .entries
                            .insert((namespace.clone(), name.clone()), Entry::Raw(value.clone()));
                    }
                }
            }
        }
        result
    }
    /// Encode changed values, preserving raw entries and unrecognized tags.
    pub fn to_nbt(&self) -> Compound {
        let mut raw = self.raw.clone();
        for ((namespace, name), entry) in &self.entries {
            let value = match entry {
                Entry::Raw(value) => Some(value.clone()),
                Entry::Decoded(value) => envelope(value.as_ref()),
            };
            if let Some(value) = value {
                let tag = raw
                    .entry(namespace.clone())
                    .or_insert_with(|| Tag::Compound(Compound::new()));
                let Tag::Compound(names) = tag else {
                    unreachable!("attachment namespace checked before mutation");
                };
                names.insert(name.clone(), Tag::Compound(value));
            }
        }
        raw
    }
    /// Copy opted-in values through their disk encoding for a player respawn.
    pub fn copy_on_death(&self) -> Result<Self, AttachmentError> {
        let mut copy = Self::default();
        for (key, entry) in &self.entries {
            let encoded = match entry {
                Entry::Raw(raw)
                    if matches!(raw.get(&"copy_on_death".into()), Some(Tag::Byte(1))) =>
                {
                    Some(raw.clone())
                }
                Entry::Decoded(value) if value.copy_on_death() => {
                    Some(envelope(value.as_ref()).ok_or(AttachmentError::InvalidCopyPolicy)?)
                }
                _ => None,
            };
            if let Some(encoded) = encoded {
                copy.entries.insert(key.clone(), Entry::Raw(encoded));
            }
        }
        Ok(copy)
    }
    /// Whether a value is stored, saved or set, without decoding or inserting it.
    pub fn has<T: Clone + Send + Sync + 'static>(&self, ty: &AttachmentType<T>) -> bool {
        self.entries
            .contains_key(&(ty.namespace.into(), ty.name.into()))
    }
    /// Get a stored value without inserting the default; saved values decode lazily.
    pub fn get_existing<T: Clone + Send + Sync + 'static>(
        &mut self,
        ty: &AttachmentType<T>,
    ) -> Result<Option<T>, AttachmentError> {
        ty.validate_policy()?;
        if self
            .raw
            .get(&ty.namespace.into())
            .is_some_and(|tag| !matches!(tag, Tag::Compound(_)))
        {
            return Err(AttachmentError::InvalidNamespace);
        }
        let value = match self.entries.get(&(ty.namespace.into(), ty.name.into())) {
            Some(Entry::Raw(raw)) => {
                let value = raw_value(raw).ok_or(AttachmentError::Decode)?;
                (ty.codec.as_ref().ok_or(AttachmentError::Decode)?.decode)(value)?
            }
            Some(Entry::Decoded(value)) => {
                return value
                    .as_any()
                    .downcast_ref::<T>()
                    .cloned()
                    .map(Some)
                    .ok_or(AttachmentError::TypeMismatch);
            }
            None => return Ok(None),
        };
        self.insert_decoded(ty, value.clone())?;
        Ok(Some(value))
    }
    /// Get an existing value or insert its default; saved values decode lazily.
    pub fn get<T: Clone + Send + Sync + 'static>(
        &mut self,
        ty: &AttachmentType<T>,
    ) -> Result<T, AttachmentError> {
        if let Some(value) = self.get_existing(ty)? {
            return Ok(value);
        }
        let value = (ty.default)();
        self.insert_decoded(ty, value.clone())?;
        if ty.sync {
            self.pending.insert((ty.namespace.into(), ty.name.into()));
        }
        Ok(value)
    }
    /// Replace a value. A key already decoded as a different type is rejected.
    pub fn set<T: Clone + Send + Sync + 'static>(
        &mut self,
        ty: &AttachmentType<T>,
        value: T,
    ) -> Result<(), AttachmentError> {
        ty.validate_policy()?;
        if self
            .raw
            .get(&ty.namespace.into())
            .is_some_and(|tag| !matches!(tag, Tag::Compound(_)))
        {
            return Err(AttachmentError::InvalidNamespace);
        }
        if let Some(Entry::Decoded(old)) = self.entries.get(&(ty.namespace.into(), ty.name.into()))
            && !old.as_any().is::<T>()
        {
            return Err(AttachmentError::TypeMismatch);
        }
        self.insert_decoded(ty, value)?;
        if ty.sync {
            self.pending.insert((ty.namespace.into(), ty.name.into()));
        }
        if let Some(Tag::Compound(names)) = self.raw.get_mut(&ty.namespace.into()) {
            names.remove(&ty.name.into());
        }
        Ok(())
    }
    /// Remove a value, retaining all other namespaces and names. A saved value
    /// that does not decode is removed too, and its error returned.
    pub fn remove<T: Clone + Send + Sync + 'static>(
        &mut self,
        ty: &AttachmentType<T>,
    ) -> Result<Option<T>, AttachmentError> {
        if !self.has(ty) {
            return Ok(None);
        }
        if ty.sync {
            self.pending.insert((ty.namespace.into(), ty.name.into()));
        }
        let value = self.get_existing(ty);
        self.entries.remove(&(ty.namespace.into(), ty.name.into()));
        if let Some(Tag::Compound(names)) = self.raw.get_mut(&ty.namespace.into()) {
            names.remove(&ty.name.into());
        }
        value
    }
    fn insert_decoded<T: Clone + Send + Sync + 'static>(
        &mut self,
        ty: &AttachmentType<T>,
        value: T,
    ) -> Result<(), AttachmentError> {
        let encoded = ty
            .codec
            .as_ref()
            .map(|codec| (codec.encode)(&value))
            .transpose()?;
        self.entries.insert(
            (ty.namespace.into(), ty.name.into()),
            Entry::Decoded(Box::new(TypedValue {
                value,
                encoded,
                copy_on_death: ty.copy_on_death,
                sync: ty.sync,
            })),
        );
        Ok(())
    }
    /// Queue a value to be sent to clients again; nothing for an unsynced type.
    pub fn mark_sync<T: 'static>(&mut self, ty: &AttachmentType<T>) -> Result<(), AttachmentError> {
        ty.validate_policy()?;
        if ty.sync {
            self.pending.insert((ty.namespace.into(), ty.name.into()));
        }
        Ok(())
    }
    /// The changes to send to clients since the last call: the current value
    /// of each queued key, or `None` for one that is gone.
    pub fn take_sync_changes(&mut self) -> Vec<AttachmentSync> {
        std::mem::take(&mut self.pending)
            .into_iter()
            .map(|(namespace, name)| {
                let value = self
                    .entries
                    .get(&(namespace.clone(), name.clone()))
                    .and_then(Entry::synced_value)
                    .cloned();
                AttachmentSync {
                    namespace,
                    name,
                    value,
                }
            })
            .collect()
    }
    /// Every synced value, for a client that starts seeing the holder.
    pub fn synced(&self) -> Vec<AttachmentSync> {
        self.entries
            .iter()
            .filter(|(_, entry)| entry.is_synced())
            .filter_map(|((namespace, name), entry)| {
                Some(AttachmentSync {
                    namespace: namespace.clone(),
                    name: name.clone(),
                    value: Some(entry.synced_value()?.clone()),
                })
            })
            .collect()
    }
    /// Client side: keep what the server sent, decoded by the next `get`.
    pub fn apply_sync(&mut self, change: AttachmentSync) {
        let key = (change.namespace, change.name);
        match change.value {
            Some(value) => {
                let raw = [("value".into(), Tag::Compound(value))].into();
                self.entries.insert(key, Entry::Raw(raw));
            }
            None => {
                self.entries.remove(&key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(n: &i32) -> Result<Compound, AttachmentError> {
        Ok([(JavaString::from("value"), Tag::Int(*n))].into())
    }
    fn decode(c: &Compound) -> Result<i32, AttachmentError> {
        match c.get(&"value".into()) {
            Some(Tag::Int(n)) => Ok(*n),
            _ => Err(AttachmentError::Decode),
        }
    }
    static COUNTER: AttachmentType<i32> = AttachmentType {
        namespace: "test",
        name: "counter",
        default: || 0,
        codec: Some(AttachmentCodec { encode, decode }),
        copy_on_death: false,
        sync: false,
    };
    static SYNCED: AttachmentType<i32> = AttachmentType {
        namespace: "test",
        name: "synced",
        default: || 7,
        codec: Some(AttachmentCodec { encode, decode }),
        copy_on_death: false,
        sync: true,
    };
    fn change(name: &str, value: Option<i32>) -> AttachmentSync {
        AttachmentSync {
            namespace: "test".into(),
            name: name.into(),
            value: value.map(|n| encode(&n).unwrap()),
        }
    }
    static MEMORY: AttachmentType<i32> = AttachmentType {
        namespace: "test",
        name: "memory",
        default: || 3,
        codec: None,
        copy_on_death: false,
        sync: false,
    };
    #[test]
    fn saved_values_round_trip_and_memory_values_do_not() {
        let mut map = Attachments::default();
        map.set(&COUNTER, 42).unwrap();
        map.set(&MEMORY, 8).unwrap();
        let mut loaded = Attachments::from_nbt(map.to_nbt());
        assert_eq!(loaded.get(&COUNTER), Ok(42));
        assert_eq!(loaded.get(&MEMORY), Ok(3));
        assert_eq!(loaded.remove(&COUNTER), Ok(Some(42)));
        let mut reopened = Attachments::from_nbt(loaded.to_nbt());
        assert_eq!(reopened.get(&COUNTER), Ok(0));
    }
    #[test]
    fn untouched_saved_value_copies_on_respawn() {
        let keep = AttachmentType {
            namespace: "test",
            name: "keep",
            default: || 0,
            codec: Some(AttachmentCodec { encode, decode }),
            copy_on_death: true,
            sync: false,
        };
        let mut original = Attachments::default();
        original.set(&keep, 42).unwrap();
        original.set(&COUNTER, 5).unwrap();
        let loaded = Attachments::from_nbt(original.to_nbt());
        let mut respawned = loaded.copy_on_death().unwrap();
        assert_eq!(respawned.get(&keep), Ok(42));
        assert_eq!(respawned.get(&COUNTER), Ok(0));
    }
    #[test]
    fn existing_reads_leave_an_absent_value_absent() {
        let mut map = Attachments::default();
        assert!(!map.has(&COUNTER));
        assert_eq!(map.get_existing(&COUNTER), Ok(None));
        assert!(!map.has(&COUNTER));
        assert_eq!(map.to_nbt(), Compound::new());
        assert_eq!(map.get(&COUNTER), Ok(0));
        assert!(map.has(&COUNTER));
        assert_ne!(map.to_nbt(), Compound::new());
        map.set(&COUNTER, 5).unwrap();
        assert_eq!(map.get_existing(&COUNTER), Ok(Some(5)));
    }
    #[test]
    fn existing_read_decodes_a_saved_value_without_changing_it() {
        let mut map = Attachments::default();
        map.set(&COUNTER, 42).unwrap();
        map.set(&MEMORY, 8).unwrap();
        let saved = map.to_nbt();
        let mut loaded = Attachments::from_nbt(saved.clone());
        assert!(loaded.has(&COUNTER));
        assert_eq!(loaded.get_existing(&COUNTER), Ok(Some(42)));
        assert_eq!(loaded.to_nbt(), saved);
        assert!(!loaded.has(&MEMORY));
        assert_eq!(loaded.get_existing(&MEMORY), Ok(None));
    }
    #[test]
    fn unknown_and_invalid_values_survive_a_failed_decode() {
        let saved: Compound = [(
            JavaString::from("test"),
            Tag::Compound(
                [
                    (
                        JavaString::from("counter"),
                        Tag::Compound(
                            [(JavaString::from("value"), Tag::String("bad".into()))].into(),
                        ),
                    ),
                    (JavaString(vec![0xd800]), Tag::Compound(Compound::new())),
                ]
                .into(),
            ),
        )]
        .into();
        let mut map = Attachments::from_nbt(saved.clone());
        assert!(map.has(&COUNTER));
        assert_eq!(map.get_existing(&COUNTER), Err(AttachmentError::Decode));
        assert_eq!(map.get(&COUNTER), Err(AttachmentError::Decode));
        assert_eq!(map.to_nbt(), saved);
        assert_eq!(map.remove(&COUNTER), Err(AttachmentError::Decode));
        assert_eq!(map.remove(&COUNTER), Ok(None), "removed all the same");
    }
    #[test]
    fn synced_set_and_remove_report_the_last_state_once() {
        let mut map = Attachments::default();
        map.set(&SYNCED, 1).unwrap();
        map.set(&SYNCED, 2).unwrap();
        map.remove(&SYNCED).unwrap();
        assert_eq!(map.take_sync_changes(), [change("synced", None)]);
        assert!(map.take_sync_changes().is_empty());
    }
    #[test]
    fn default_creation_is_synced_and_a_later_read_is_not() {
        let mut map = Attachments::default();
        assert_eq!(map.get(&SYNCED), Ok(7));
        assert_eq!(map.take_sync_changes(), [change("synced", Some(7))]);
        assert_eq!(map.get(&SYNCED), Ok(7));
        assert!(map.take_sync_changes().is_empty());
    }
    #[test]
    fn unsynced_types_report_no_changes() {
        let mut map = Attachments::default();
        map.set(&COUNTER, 1).unwrap();
        map.remove(&COUNTER).unwrap();
        map.mark_sync(&COUNTER).unwrap();
        assert!(map.take_sync_changes().is_empty());
    }
    #[test]
    fn sync_without_a_codec_is_refused() {
        let broken = AttachmentType {
            namespace: "test",
            name: "broken",
            default: || 0,
            codec: None,
            copy_on_death: false,
            sync: true,
        };
        let mut map = Attachments::default();
        assert_eq!(map.get(&broken), Err(AttachmentError::InvalidSyncPolicy));
        assert_eq!(map.set(&broken, 1), Err(AttachmentError::InvalidSyncPolicy));
        assert_eq!(
            map.mark_sync(&broken),
            Err(AttachmentError::InvalidSyncPolicy)
        );
        assert!(map.take_sync_changes().is_empty());
    }
    #[test]
    fn a_saved_sync_flag_puts_undecoded_values_in_the_snapshot() {
        let mut map = Attachments::default();
        map.set(&SYNCED, 5).unwrap();
        map.set(&COUNTER, 6).unwrap();
        let saved = map.to_nbt();
        let Some(Tag::Compound(names)) = saved.get(&"test".into()) else {
            panic!("namespace saved");
        };
        let Some(Tag::Compound(envelope)) = names.get(&"synced".into()) else {
            panic!("entry saved");
        };
        assert_eq!(envelope.get(&"sync".into()), Some(&Tag::Byte(1)));
        let loaded = Attachments::from_nbt(saved);
        assert_eq!(loaded.synced(), [change("synced", Some(5))]);
    }
    #[test]
    fn applied_changes_decode_through_the_codec_and_removals_clear() {
        let mut map = Attachments::default();
        map.apply_sync(change("synced", Some(9)));
        assert_eq!(map.get(&SYNCED), Ok(9));
        map.apply_sync(change("synced", None));
        assert_eq!(map.get(&SYNCED), Ok(7));
    }
    #[test]
    fn sync_data_reports_the_current_value() {
        struct One(Attachments);
        impl AttachmentHost for One {
            fn attachments(&mut self, _: Holder) -> Result<&mut Attachments, AttachmentError> {
                Ok(&mut self.0)
            }
        }
        let mut host = One(Attachments::default());
        SYNCED.set(&mut host, Holder::Level, 4).unwrap();
        host.0.take_sync_changes();
        SYNCED.sync_data(&mut host, Holder::Level).unwrap();
        assert_eq!(host.0.take_sync_changes(), [change("synced", Some(4))]);
    }
}
