//! The GUI atlas's fixed slots for runtime images (`horizon/skin{N}`,
//! `horizon/map{N}`): which image each slot holds, least recently shown
//! first. A new atlas (a resource pack reload) has every slot blank, so the
//! table is forgotten with it and each image uploads again.

pub(super) struct AtlasSlots<K> {
    count: usize,
    /// (image, slot), most recently shown last.
    held: Vec<(K, usize)>,
}

impl<K: PartialEq> AtlasSlots<K> {
    pub(super) fn new(count: usize) -> Self {
        Self {
            count,
            held: Vec::new(),
        }
    }
    /// The slot holding `key`, marking it recently shown.
    pub(super) fn show<Q: PartialEq + ?Sized>(&mut self, key: &Q) -> Option<usize>
    where
        K: std::borrow::Borrow<Q>,
    {
        let i = self.held.iter().position(|e| e.0.borrow() == key)?;
        let e = self.held.remove(i);
        let slot = e.1;
        self.held.push(e);
        Some(slot)
    }
    /// A slot for `key`: a free one, else the least recently shown, returned
    /// with the image it held.
    pub(super) fn claim(&mut self, key: K) -> (usize, Option<K>) {
        let (slot, evicted) = if self.held.len() < self.count {
            (self.held.len(), None)
        } else {
            let (old, slot) = self.held.remove(0);
            (slot, Some(old))
        };
        self.held.push((key, slot));
        (slot, evicted)
    }
    /// The atlas was replaced: every slot is blank.
    pub(super) fn forget(&mut self) {
        self.held.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_least_recently_shown_image_gives_up_its_slot() {
        let mut slots = AtlasSlots::new(2);
        assert_eq!(slots.claim("a"), (0, None));
        assert_eq!(slots.claim("b"), (1, None));
        assert_eq!(slots.show(&"a"), Some(0));
        assert_eq!(slots.claim("c"), (1, Some("b")));
        assert_eq!(slots.show(&"b"), None);
    }

    #[test]
    fn a_new_atlas_shows_no_stored_skin() {
        // #3385: a skin stored before a pack reload must upload again, not
        // name a sprite the new atlas does not have.
        let mut slots = AtlasSlots::new(4);
        slots.claim("skins/abc".to_string());
        assert_eq!(slots.show("skins/abc"), Some(0));
        slots.forget();
        assert_eq!(slots.show("skins/abc"), None);
        assert_eq!(slots.claim("skins/abc".to_string()), (0, None));
    }
}
