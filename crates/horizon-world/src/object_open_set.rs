//! Default fastutil ObjectOpenHashSet storage and live iterator semantics.
//! Key hashing/equality must be pure; null is represented by None.
pub trait ObjectKey: Eq {
    fn object_hash(&self) -> i32;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorError {
    NoSuchElement,
    NoCurrentEntry,
    MissingWrapped,
    IndexOutOfBounds,
}
pub struct Cursor<K> {
    position: i32,
    last: i32,
    remaining: i32,
    null: bool,
    wrapped: Option<Vec<Option<K>>>,
}
impl<K> Cursor<K> {
    pub fn has_next(&self) -> bool {
        self.remaining != 0
    }
}
#[derive(Clone)]
pub struct ObjectOpenSet<K> {
    slots: Vec<Option<K>>,
    size: i32,
    null: bool,
}
impl<K> Default for ObjectOpenSet<K> {
    fn default() -> Self {
        Self {
            slots: std::iter::repeat_with(|| None).take(33).collect(),
            size: 0,
            null: false,
        }
    }
}
fn mixed(hash: i32) -> usize {
    let h = (hash as u32).wrapping_mul(0x9e3779b9);
    (h ^ (h >> 16)) as usize
}
impl<K: ObjectKey + Clone> ObjectOpenSet<K> {
    pub fn len(&self) -> i32 {
        self.size
    }
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }
    fn n(&self) -> usize {
        self.slots.len() - 1
    }
    fn max_fill(&self) -> i32 {
        (self.n() * 3 / 4) as i32
    }
    fn find(&self, key: &K) -> usize {
        let mask = self.n() - 1;
        let mut p = mixed(key.object_hash()) & mask;
        while self.slots[p].as_ref().is_some_and(|old| old != key) {
            p = (p + 1) & mask;
        }
        p
    }
    pub fn contains(&self, key: Option<&K>) -> bool {
        key.map_or(self.null, |key| self.slots[self.find(key)].is_some())
    }
    pub fn get(&self, key: &K) -> Option<&K> {
        self.slots[self.find(key)].as_ref()
    }
    pub fn values(&self) -> impl Iterator<Item = Option<&K>> {
        self.null.then_some(None).into_iter().chain(
            self.slots[..self.n()]
                .iter()
                .rev()
                .filter_map(|v| v.as_ref().map(Some)),
        )
    }
    pub fn add(&mut self, key: Option<K>) -> bool {
        if let Some(key) = key {
            let p = self.find(&key);
            if self.slots[p].is_some() {
                return false;
            }
            self.slots[p] = Some(key);
        } else {
            if self.null {
                return false;
            }
            self.null = true;
        }
        let old = self.size;
        self.size = self.size.wrapping_add(1);
        if old >= self.max_fill() {
            self.rehash(
                (((self.size + 1) as f64 / 0.75).ceil() as usize)
                    .max(2)
                    .next_power_of_two(),
            );
        }
        true
    }
    fn rehash(&mut self, capacity: usize) {
        let old = std::mem::replace(&mut self.slots, vec![None; capacity + 1]);
        for key in old.into_iter().rev().flatten() {
            let p = self.find(&key);
            self.slots[p] = Some(key);
        }
    }
    fn shift(
        &mut self,
        mut hole: usize,
        mut cursor: Option<&mut Cursor<K>>,
    ) -> Result<(), CursorError> {
        let mask = self.n() - 1;
        loop {
            let mut p = (hole + 1) & mask;
            loop {
                let Some(key) = &self.slots[p] else {
                    *self
                        .slots
                        .get_mut(hole)
                        .ok_or(CursorError::IndexOutOfBounds)? = None;
                    return Ok(());
                };
                let home = mixed(key.object_hash()) & mask;
                if if hole <= p {
                    hole >= home || home > p
                } else {
                    hole >= home && home > p
                } {
                    break;
                }
                p = (p + 1) & mask;
            }
            if p < hole {
                if let Some(cursor) = &mut cursor {
                    cursor
                        .wrapped
                        .get_or_insert_with(Vec::new)
                        .push(self.slots[p].clone());
                }
            }
            let key = self.slots[p].clone();
            *self
                .slots
                .get_mut(hole)
                .ok_or(CursorError::IndexOutOfBounds)? = key;
            hole = p;
        }
    }
    pub fn remove(&mut self, key: Option<&K>) -> bool {
        if let Some(key) = key {
            let p = self.find(key);
            if self.slots[p].is_none() {
                return false;
            }
            self.size = self.size.wrapping_sub(1);
            self.shift(p, None).unwrap();
        } else {
            if !self.null {
                return false;
            }
            self.null = false;
            self.size = self.size.wrapping_sub(1);
        }
        if self.n() > 32 && self.size < self.max_fill() / 4 {
            self.rehash(self.n() / 2);
        }
        true
    }
    pub fn clear(&mut self) {
        if self.size == 0 {
            return;
        }
        self.size = 0;
        self.null = false;
        self.slots.fill(None);
    }
    pub fn cursor(&self) -> Cursor<K> {
        Cursor {
            position: self.n() as i32,
            last: -1,
            remaining: self.size,
            null: self.null,
            wrapped: None,
        }
    }
    /// The cursor reads the live slot array. Unlike HashMap it has no
    /// modification counter, including when callbacks clear or resize the set.
    pub fn next(&self, cursor: &mut Cursor<K>) -> Result<Option<K>, CursorError> {
        if !cursor.has_next() {
            return Err(CursorError::NoSuchElement);
        }
        cursor.remaining = cursor.remaining.wrapping_sub(1);
        if cursor.null {
            cursor.null = false;
            cursor.last = self.n() as i32;
            return Ok(None);
        }
        loop {
            cursor.position = cursor.position.wrapping_sub(1);
            if cursor.position < 0 {
                cursor.last = i32::MIN;
                let wrapped = cursor.wrapped.as_ref().ok_or(CursorError::MissingWrapped)?;
                return wrapped
                    .get(cursor.position.wrapping_neg().wrapping_sub(1) as usize)
                    .cloned()
                    .ok_or(CursorError::IndexOutOfBounds);
            }
            let key = self
                .slots
                .get(cursor.position as usize)
                .ok_or(CursorError::IndexOutOfBounds)?;
            if key.is_some() {
                cursor.last = cursor.position;
                return Ok(key.clone());
            }
        }
    }
    pub fn remove_current(&mut self, cursor: &mut Cursor<K>) -> Result<(), CursorError> {
        if cursor.last == -1 {
            return Err(CursorError::NoCurrentEntry);
        }
        if cursor.last == self.n() as i32 {
            self.null = false;
        } else if cursor.position >= 0 {
            self.shift(cursor.last as usize, Some(cursor))?;
        } else {
            let wrapped = cursor.wrapped.as_mut().ok_or(CursorError::MissingWrapped)?;
            let key = wrapped
                .get_mut(cursor.position.wrapping_neg().wrapping_sub(1) as usize)
                .ok_or(CursorError::IndexOutOfBounds)?
                .take();
            self.remove(key.as_ref());
            cursor.last = -1;
            return Ok(());
        }
        self.size = self.size.wrapping_sub(1);
        cursor.last = -1;
        Ok(())
    }
}
