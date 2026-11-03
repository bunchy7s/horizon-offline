//! Registered entity defaults and lazy attribute instances. Cross-attribute bulk
//! mutation/serialization order is deliberately outside this per-attribute API.
use super::{Definition, Instance, Observer};
use crate::{DATA_VERSION, compression::invalid};
use serde_json::Value;
use std::{collections::BTreeMap, convert::Infallible, io, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AttributeId(usize);
impl AttributeId {
    pub(super) fn index(self) -> usize {
        self.0
    }
    /// The attribute's id in the `minecraft:attribute` registry: the catalog
    /// export lists `BuiltInRegistries.ATTRIBUTE` in order.
    pub fn registry_id(self) -> i32 {
        self.0 as i32
    }
}
#[derive(Debug)]
pub struct RegisteredAttribute {
    pub name: String,
    pub definition: Definition,
    pub syncable: bool,
}
#[derive(Debug)]
pub struct Catalog {
    attributes: Vec<RegisteredAttribute>,
    ids: BTreeMap<String, AttributeId>,
    defaults: BTreeMap<String, Arc<[Option<f64>]>>,
}
fn string(v: &Value) -> io::Result<&str> {
    v.as_str()
        .ok_or_else(|| invalid("expected attribute string"))
}
fn number(v: &Value) -> io::Result<f64> {
    let s = string(v)?;
    if s.len() != 16 {
        return Err(invalid("expected 16-digit attribute double bits"));
    }
    Ok(f64::from_bits(
        u64::from_str_radix(s, 16).map_err(|e| invalid(e.to_string()))?,
    ))
}
impl Catalog {
    pub(super) fn default_bases(&self, entity: &str) -> Option<Arc<[Option<f64>]>> {
        self.defaults.get(entity).cloned()
    }
    /// Imports a version-checked export of registered definitions and actual
    /// DefaultAttributes suppliers. Defaults contain base values, not instances.
    pub fn from_json(v: &Value) -> io::Result<Self> {
        if v["data_version"].as_i64() != Some(i64::from(DATA_VERSION)) {
            return Err(invalid("attribute catalog data version mismatch"));
        }
        let mut attributes = Vec::new();
        let mut ids = BTreeMap::new();
        for a in v["attributes"]
            .as_array()
            .ok_or_else(|| invalid("missing attributes"))?
        {
            let name = string(&a["id"])?;
            // Identifier normalization is shared with modifier IDs.
            let canonical = super::Modifier::new(name, 0.0, super::Operation::AddValue)?;
            if canonical.id() != name
                || ids
                    .insert(name.into(), AttributeId(attributes.len()))
                    .is_some()
            {
                return Err(invalid("noncanonical or duplicate attribute identifier"));
            }
            let default = number(&a["default"])?;
            let definition = if a.get("min").is_some() {
                Definition::ranged(default, number(&a["min"])?, number(&a["max"])?)?
            } else {
                Definition::basic(default)
            };
            attributes.push(RegisteredAttribute {
                name: name.into(),
                definition,
                syncable: a["syncable"]
                    .as_bool()
                    .ok_or_else(|| invalid("missing attribute sync flag"))?,
            });
        }
        let mut defaults = BTreeMap::new();
        for (entity, values) in v["entities"]
            .as_object()
            .ok_or_else(|| invalid("missing entity defaults"))?
        {
            let mut bases = vec![None; attributes.len()];
            for (name, base) in values
                .as_object()
                .ok_or_else(|| invalid("invalid entity supplier"))?
            {
                let id = ids
                    .get(name)
                    .ok_or_else(|| invalid("unknown supplier attribute"))?;
                bases[id.0] = Some(number(base)?);
            }
            defaults.insert(entity.clone(), Arc::from(bases));
        }
        Ok(Self {
            attributes,
            ids,
            defaults,
        })
    }
    /// The entity type's supplier base of an attribute, if the type has it.
    pub fn default_base(&self, entity: &str, id: AttributeId) -> Option<f64> {
        self.defaults.get(entity).and_then(|b| b[id.0])
    }
    pub fn id(&self, name: &str) -> Option<AttributeId> {
        self.ids.get(name).copied()
    }
    pub fn attribute(&self, id: AttributeId) -> &RegisteredAttribute {
        &self.attributes[id.0]
    }
    pub fn attributes(&self) -> impl Iterator<Item = (AttributeId, &RegisteredAttribute)> {
        self.attributes
            .iter()
            .enumerate()
            .map(|(i, a)| (AttributeId(i), a))
    }
    pub fn entities(&self) -> impl Iterator<Item = &str> {
        self.defaults.keys().map(String::as_str)
    }
    pub fn create(self: &Arc<Self>, entity: &str) -> Option<EntityAttributes> {
        let bases = self.defaults.get(entity)?.clone();
        let n = bases.len();
        Some(EntityAttributes {
            catalog: self.clone(),
            bases,
            instances: vec![None; n],
            updates: vec![false; n],
            sync: vec![false; n],
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LookupError {
    MissingAttribute,
    MissingModifier,
    InstanceMissingModifier,
}
/// Membership in the native update/sync sets, with no JVM identity-hash order
/// promise. Clearing a flag acknowledges that particular pending update.
pub struct DirtyObserver<'a> {
    update: &'a mut bool,
    sync: &'a mut bool,
    syncable: bool,
}
impl Observer for DirtyObserver<'_> {
    type Error = Infallible;
    fn dirty(&mut self, _: &mut Instance) -> Result<(), Infallible> {
        *self.update = true;
        if self.syncable {
            *self.sync = true;
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct EntityAttributes {
    catalog: Arc<Catalog>,
    bases: Arc<[Option<f64>]>,
    instances: Vec<Option<Box<Instance>>>,
    updates: Vec<bool>,
    sync: Vec<bool>,
}
impl EntityAttributes {
    pub fn catalog(&self) -> &Arc<Catalog> {
        &self.catalog
    }
    pub fn has(&self, id: AttributeId) -> bool {
        self.bases[id.0].is_some()
    }
    pub fn instance(&self, id: AttributeId) -> Option<&Instance> {
        self.instances[id.0].as_deref()
    }
    /// Merely querying a default value must not instantiate or dirty it.
    pub fn value(&mut self, id: AttributeId) -> Result<f64, LookupError> {
        if let Some(a) = self.instances[id.0].as_deref_mut() {
            return Ok(a.value());
        }
        let base = self.bases[id.0].ok_or(LookupError::MissingAttribute)?;
        Ok(self.catalog.attribute(id).definition.sanitize(base))
    }
    pub fn base_value(&self, id: AttributeId) -> Result<f64, LookupError> {
        self.instances[id.0]
            .as_deref()
            .map(Instance::base_value)
            .or(self.bases[id.0])
            .ok_or(LookupError::MissingAttribute)
    }
    pub fn has_modifier(&self, id: AttributeId, modifier: &str) -> bool {
        self.instance(id)
            .and_then(|a| a.modifier(modifier))
            .is_some()
    }
    pub fn modifier_value(&self, id: AttributeId, modifier: &str) -> Result<f64, LookupError> {
        if let Some(a) = self.instance(id) {
            return a
                .modifier(modifier)
                .map(super::Modifier::amount)
                .ok_or(LookupError::InstanceMissingModifier);
        }
        if self.has(id) {
            Err(LookupError::MissingModifier)
        } else {
            Err(LookupError::MissingAttribute)
        }
    }
    /// Native getInstance copies the supplier on first access and immediately
    /// schedules an update, plus a sync when the definition is client-syncable.
    pub fn ensure(&mut self, id: AttributeId) -> bool {
        if self.instances[id.0].is_some() {
            return true;
        }
        let Some(base) = self.bases[id.0] else {
            return false;
        };
        let definition = self.catalog.attribute(id);
        let mut instance = Instance::new(definition.definition);
        let mut observer = DirtyObserver {
            update: &mut self.updates[id.0],
            sync: &mut self.sync[id.0],
            syncable: definition.syncable,
        };
        // Registered default suppliers have no modifiers. replaceFrom always
        // invalidates even when the supplier base equals the registered default.
        let mut source = Instance::new(definition.definition);
        source.base = base;
        instance.replace_from(&source, &mut observer).unwrap();
        self.instances[id.0] = Some(Box::new(instance));
        true
    }
    /// Mutations must use the supplied observer to preserve dirty memberships.
    pub fn edit<R>(
        &mut self,
        id: AttributeId,
        f: impl FnOnce(&mut Instance, &mut DirtyObserver<'_>) -> R,
    ) -> Option<R> {
        if !self.ensure(id) {
            return None;
        }
        let mut observer = DirtyObserver {
            update: &mut self.updates[id.0],
            sync: &mut self.sync[id.0],
            syncable: self.catalog.attribute(id).syncable,
        };
        Some(f(
            self.instances[id.0].as_deref_mut().unwrap(),
            &mut observer,
        ))
    }
    /// Reset uses the entity supplier's base, without creating a missing instance.
    pub fn reset_base(&mut self, id: AttributeId) -> bool {
        let Some(base) = self.bases[id.0] else {
            return false;
        };
        if self.instances[id.0].is_some() {
            self.edit(id, |a, o| a.set_base_value(base, o).unwrap());
        }
        true
    }
    pub fn needs_update(&self, id: AttributeId) -> bool {
        self.updates[id.0]
    }
    pub fn needs_sync(&self, id: AttributeId) -> bool {
        self.sync[id.0]
    }
    pub fn clear_update(&mut self, id: AttributeId) {
        self.updates[id.0] = false;
    }
    pub fn clear_sync(&mut self, id: AttributeId) {
        self.sync[id.0] = false;
    }
}
