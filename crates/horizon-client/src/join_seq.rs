//! PURPOSE: the join sequence: when each entity joined the level, kept in one
//! counter that rises with every join, whatever the kind.
//! WHY: ids do not say it (primed TNT, falling blocks, projectiles, tridents and
//! rockets take theirs late, in kind-list order), and vanilla ticks entities in
//! the order they were added, so a walk in that order needs a number that does
//! (#6509, tick order slice).
//! REFS: vanilla `EntityLookup` / `EntitySectionStorage` insertion order,
//! `PersistentEntitySectionManager.addEntity`; issue #6509; PR #7808.
//! LOGS: none. READS-BY: `game/entity.rs` (the join-order walk, the save order).
//! ENDS-WHEN: kinds share one stored entity list, which is its own order.

use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(1);

/// When an entity joined: a number above every earlier join's. Zero means it
/// has not joined a list yet. One counter for the process, so entities of
/// different levels compare too; only the order of the numbers means anything.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JoinSeq(u64);

impl JoinSeq {
    /// Not joined yet.
    pub const UNJOINED: Self = Self(0);
    /// The next number: one relaxed add, and only when something joins.
    fn next() -> Self {
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
    pub fn is_joined(self) -> bool {
        self.0 != 0
    }
    pub fn get(self) -> u64 {
        self.0
    }
}

/// A kind that stores when it joined.
pub trait Joined {
    fn join_seq(&self) -> JoinSeq;
    fn set_join_seq(&mut self, seq: JoinSeq);
}

/// Implements [`Joined`] for a struct with a `join: JoinSeq` field.
#[macro_export]
macro_rules! impl_joined {
    ($($ty:ty),* $(,)?) => {$(
        impl $crate::join_seq::Joined for $ty {
            fn join_seq(&self) -> $crate::join_seq::JoinSeq {
                self.join
            }
            fn set_join_seq(&mut self, seq: $crate::join_seq::JoinSeq) {
                self.join = seq;
            }
        }
    )*};
}

/// One kind's entity list. It is a `Vec` in every way but one: an entity that
/// is put in without a join number (`push`, `insert`, `extend`) gets the next
/// one, so a join cannot be missed by a spawn site. One already numbered (taken
/// out and put back, or a list rebuilt) keeps its number.
#[derive(Clone, Debug)]
pub struct JoinList<T>(Vec<T>);

impl<T> Default for JoinList<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<T> From<Vec<T>> for JoinList<T> {
    /// A list put back after a tick took it: the same storage, nothing copied
    /// and nothing renumbered. An entity in it that never joined stays
    /// unnumbered, which the join-order walk reports.
    fn from(list: Vec<T>) -> Self {
        Self(list)
    }
}

impl<T> From<JoinList<T>> for Vec<T> {
    fn from(list: JoinList<T>) -> Self {
        list.0
    }
}

impl<T> Deref for JoinList<T> {
    type Target = Vec<T>;
    fn deref(&self) -> &Vec<T> {
        &self.0
    }
}
impl<T> DerefMut for JoinList<T> {
    fn deref_mut(&mut self) -> &mut Vec<T> {
        &mut self.0
    }
}

impl<T: Joined> JoinList<T> {
    fn number(entity: &mut T) {
        if !entity.join_seq().is_joined() {
            entity.set_join_seq(JoinSeq::next());
        }
    }
    pub fn push(&mut self, mut entity: T) {
        Self::number(&mut entity);
        self.0.push(entity);
    }
    pub fn insert(&mut self, index: usize, mut entity: T) {
        Self::number(&mut entity);
        self.0.insert(index, entity);
    }
    pub fn extend(&mut self, entities: impl IntoIterator<Item = T>) {
        for entity in entities {
            self.push(entity);
        }
    }
}

impl<'a, T> IntoIterator for &'a JoinList<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}
impl<'a, T> IntoIterator for &'a mut JoinList<T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter_mut()
    }
}
impl<T> IntoIterator for JoinList<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}
