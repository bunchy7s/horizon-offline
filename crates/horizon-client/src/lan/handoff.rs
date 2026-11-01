//! Sections a single-player server thread sends its own window, shared
//! instead of copied. The server and the window each held every column they
//! both see: the server its level, the window what the chunk packets decoded
//! to, the same blocks, biomes and light. When the server sends a column to a
//! player on its memory connection it offers the column's sections here, as
//! `Weak` references; the window's decoder takes each one whose blocks,
//! biomes and light are the same as what it decoded, and drops its own copy.
//!
//! Nothing changes what either side sees. The packet is still sent and
//! decoded, and a section is taken only when it equals the decoded one. A
//! `Weak` keeps no section alive, and a server edit to an offered section
//! moves it (`Arc::make_mut`), which the window's `Weak` no longer reaches;
//! once taken, a section either side edits is copied for that side alone.
use crate::chunk::{Chunk, Section};
use crate::fast_hash::FxHashMap;
use std::sync::{Arc, Mutex, Weak};

/// Columns offered and not taken yet before the dead ones are swept.
const SWEEP: usize = 1 << 13;

struct Offer {
    /// The minimum block Y of `sections[0]`.
    min_y: i32,
    sections: Vec<Weak<Section>>,
}

static OFFERED: Mutex<Option<FxHashMap<(i32, i32), Offer>>> = Mutex::new(None);

/// The server sends `chunk` to a player on its memory connection.
pub fn offer(chunk: &Chunk) {
    let Ok(mut offered) = OFFERED.lock() else {
        return;
    };
    let offered = offered.get_or_insert_with(Default::default);
    if offered.len() >= SWEEP {
        offered.retain(|_, o| o.sections.iter().any(|w| w.strong_count() > 0));
        if offered.len() >= SWEEP / 2 {
            offered.clear();
        }
    }
    offered.insert(
        (chunk.x, chunk.z),
        Offer {
            min_y: chunk.min_y(),
            sections: chunk.sections.iter().map(Arc::downgrade).collect(),
        },
    );
}

/// The window decoded `chunk` from a packet: each of its sections the same
/// as the server's offered one at that height becomes the server's. Returns
/// how many did.
pub fn take(chunk: &mut Chunk) -> usize {
    let offer = match OFFERED.lock() {
        Ok(mut offered) => offered.as_mut().and_then(|o| o.remove(&(chunk.x, chunk.z))),
        Err(_) => None,
    };
    let Some(offer) = offer else {
        return 0;
    };
    let offset = (chunk.min_y() - offer.min_y) >> 4;
    let mut taken = 0;
    for (i, section) in chunk.sections.iter_mut().enumerate() {
        let Some(theirs) = usize::try_from(i as i32 + offset)
            .ok()
            .and_then(|j| offer.sections.get(j))
            .and_then(Weak::upgrade)
        else {
            continue;
        };
        if Arc::ptr_eq(section, &theirs) {
            continue;
        }
        if theirs.same_as(section) {
            *section = theirs;
            taken += 1;
        }
    }
    taken
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::{Light, MIN_Y};

    fn column(x: i32) -> Chunk {
        let mut chunk = Chunk::empty(x, 7);
        chunk.set(1, MIN_Y + 3, 2, 5);
        chunk.set(4, 70, 4, 9);
        Arc::make_mut(&mut chunk.sections[2]).light =
            Light::packed(Box::new(std::array::from_fn(|i| {
                if i % 3 == 0 { 0xF2 } else { 0x70 }
            })));
        chunk
    }

    #[test]
    fn equal_sections_are_shared_and_edits_stay_apart() {
        let server = column(901);
        offer(&server);
        let mut window = column(901);
        let taken = take(&mut window);
        // The two edited sections and the lit one; the uniform ones were
        // already the same shared allocation.
        assert_eq!(taken, 3);
        for (a, b) in server.sections.iter().zip(&window.sections) {
            assert!(Arc::ptr_eq(a, b));
        }
        let mut edited = window.clone();
        edited.set(1, MIN_Y + 3, 2, 6);
        assert_eq!(server.get(1, MIN_Y + 3, 2), 5);
        assert_eq!(edited.get(1, MIN_Y + 3, 2), 6);
        // Taken once: a second decode of the column has nothing to take.
        assert_eq!(take(&mut column(901)), 0);
    }

    #[test]
    fn differing_or_edited_sections_are_kept() {
        let mut server = column(902);
        offer(&server);
        // Edited after the offer: the server's section moved.
        server.set(1, MIN_Y + 3, 2, 8);
        let mut window = column(902);
        window.set(4, 70, 4, 10);
        let taken = take(&mut window);
        // Only the lit section is still the same on both sides.
        assert_eq!(taken, 1);
        assert_eq!(window.get(1, MIN_Y + 3, 2), 5);
        assert_eq!(window.get(4, 70, 4), 10);
    }
}
