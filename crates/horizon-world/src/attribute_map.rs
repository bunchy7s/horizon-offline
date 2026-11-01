//! Live AttributeMap instance lookup and update/synchronization queue ownership.
use crate::{
    object_open_map::ObjectOpenMap,
    object_open_set::{ObjectKey, ObjectOpenSet},
};

#[derive(Clone, Copy, Debug)]
pub enum ValueKind {
    Computed,
    Base,
}
#[derive(Debug)]
pub enum ModifierValueError<E> {
    /// Cached instances dereference getModifier's result; suppliers instead
    /// report their own missing-attribute/modifier error.
    MissingInstanceModifier,
    World(E),
}
pub trait QueryWorld<H, I> {
    type Error;
    fn supplier_has_attribute(
        &mut self,
        map: &mut AttributeMap<H, I>,
        key: Option<H>,
    ) -> Result<bool, Self::Error>;
    fn supplier_has_modifier(
        &mut self,
        map: &mut AttributeMap<H, I>,
        key: Option<H>,
        id: &str,
    ) -> Result<bool, Self::Error>;
    fn supplier_value(
        &mut self,
        map: &mut AttributeMap<H, I>,
        key: Option<H>,
        kind: ValueKind,
    ) -> Result<f64, Self::Error>;
    fn supplier_modifier_value(
        &mut self,
        map: &mut AttributeMap<H, I>,
        key: Option<H>,
        id: &str,
    ) -> Result<f64, Self::Error>;
    fn instance_value(
        &mut self,
        map: &mut AttributeMap<H, I>,
        instance: I,
        kind: ValueKind,
    ) -> Result<f64, Self::Error>;
    fn instance_modifier(
        &mut self,
        map: &mut AttributeMap<H, I>,
        instance: I,
        id: &str,
    ) -> Result<Option<f64>, Self::Error>;
    fn set_instance_base(
        &mut self,
        map: &mut AttributeMap<H, I>,
        instance: I,
        value: f64,
    ) -> Result<(), Self::Error>;
}

/// Instance handles retain identity independently of map membership. In
/// particular, a failed creation callback can leave a queued instance that was
/// never inserted into `instances`.
#[derive(Clone)]
pub struct AttributeMap<H, I> {
    pub instances: ObjectOpenMap<H, I>,
    pub updates: ObjectOpenSet<I>,
    pub sync: ObjectOpenSet<I>,
}
impl<H, I> Default for AttributeMap<H, I> {
    fn default() -> Self {
        Self {
            instances: ObjectOpenMap::default(),
            updates: ObjectOpenSet::default(),
            sync: ObjectOpenSet::default(),
        }
    }
}
impl<H: ObjectKey + Clone, I: ObjectKey + Clone> AttributeMap<H, I> {
    pub fn has_attribute<W: QueryWorld<H, I>>(
        &mut self,
        key: Option<H>,
        world: &mut W,
    ) -> Result<bool, W::Error> {
        if self.instances.get(key.as_ref()).is_some() {
            Ok(true)
        } else {
            world.supplier_has_attribute(self, key)
        }
    }
    pub fn has_modifier<W: QueryWorld<H, I>>(
        &mut self,
        key: Option<H>,
        id: &str,
        world: &mut W,
    ) -> Result<bool, W::Error> {
        if let Some(instance) = self.instances.get(key.as_ref()) {
            world
                .instance_modifier(self, instance, id)
                .map(|m| m.is_some())
        } else {
            world.supplier_has_modifier(self, key, id)
        }
    }
    /// Value queries use a cached instance or its supplier prototype. They do
    /// not create an instance or schedule a dirty update.
    pub fn value<W: QueryWorld<H, I>>(
        &mut self,
        key: Option<H>,
        kind: ValueKind,
        world: &mut W,
    ) -> Result<f64, W::Error> {
        if let Some(instance) = self.instances.get(key.as_ref()) {
            world.instance_value(self, instance, kind)
        } else {
            world.supplier_value(self, key, kind)
        }
    }
    pub fn modifier_value<W: QueryWorld<H, I>>(
        &mut self,
        key: Option<H>,
        id: &str,
        world: &mut W,
    ) -> Result<f64, ModifierValueError<W::Error>> {
        if let Some(instance) = self.instances.get(key.as_ref()) {
            world
                .instance_modifier(self, instance, id)
                .map_err(ModifierValueError::World)?
                .ok_or(ModifierValueError::MissingInstanceModifier)
        } else {
            world
                .supplier_modifier_value(self, key, id)
                .map_err(ModifierValueError::World)
        }
    }
    pub fn reset_base<W: QueryWorld<H, I>>(
        &mut self,
        key: Option<H>,
        world: &mut W,
    ) -> Result<bool, W::Error> {
        if !world.supplier_has_attribute(self, key.clone())? {
            return Ok(false);
        }
        if let Some(instance) = self.instances.get(key.as_ref()) {
            let value = world.supplier_value(self, key, ValueKind::Base)?;
            world.set_instance_base(self, instance, value)?;
        }
        Ok(true)
    }
    /// AttributeMap dispatches to Map.computeIfAbsent(Function), not fastutil's
    /// Object2ObjectFunction overload. A null mapping is absent for this call;
    /// the ordinary put re-finds its slot after the supplier callback returns.
    pub fn get_instance<E>(
        &mut self,
        key: Option<H>,
        create: impl FnOnce(&mut Self, Option<H>) -> Result<Option<I>, E>,
    ) -> Result<Option<I>, E> {
        if let Some(instance) = self.instances.get(key.as_ref()) {
            return Ok(Some(instance));
        }
        let instance = create(self, key.clone())?;
        if instance.is_some() {
            self.instances.put(key, instance.clone());
        }
        Ok(instance)
    }
    /// The callback performs instance.getAttribute().value().isClientSyncable().
    /// It runs after adding the update entry, and can inspect or mutate both
    /// queues. Failure preserves the update entry and skips the sync insertion.
    pub fn modified<E>(
        &mut self,
        instance: Option<I>,
        syncable: impl FnOnce(&mut Self, Option<I>) -> Result<bool, E>,
    ) -> Result<(), E> {
        self.updates.add(instance.clone());
        if syncable(self, instance.clone())? {
            self.sync.add(instance);
        }
        Ok(())
    }
}
