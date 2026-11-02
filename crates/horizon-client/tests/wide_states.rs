//! Block states past vanilla's 16-bit global palette (docs/modpacks.md):
//! content mods can take the registry past 65,536 states. Its own test binary,
//! since the palette width is set once for the whole process.

extern crate horizon_client_lib as horizon_client;
use horizon_client::chunk::{Chunk, Section, block_strategy, set_block_states};

#[test]
fn sections_hold_states_past_sixteen_bits() {
    // Vanilla's width until the registry needs more (one test: the width is
    // global to the process).
    assert_eq!(block_strategy().global_bits, 16);
    set_block_states(35_723);
    assert_eq!(block_strategy().global_bits, 16);
    set_block_states(100_000);
    assert_eq!(block_strategy().global_bits, 17);

    // A few states past 65,536 stay in the section's own palette.
    let mut chunk = Chunk::empty(0, 0);
    chunk.set(1, 70, 2, 70_000);
    chunk.set(3, 70, 4, 99_999);
    assert_eq!(chunk.get(1, 70, 2), 70_000);
    assert_eq!(chunk.get(3, 70, 4), 99_999);
    assert_eq!(chunk.get(0, 70, 0), 0);

    // More than 256 distinct states take the global palette at 17 bits.
    let mut values = [0u32; 4096];
    for (i, v) in values.iter_mut().enumerate() {
        *v = 65_000 + (i as u32 % 1000) * 30;
    }
    let mut section = Section {
        blocks: Section::from_blocks(&values),
        ..Section::default()
    };
    let mut out = [0u32; 4096];
    section.unpack(&mut out);
    assert_eq!(out, values);
    section.set(17, 99_998);
    assert_eq!(section.get(17), 99_998);
    assert_eq!(section.get(18), values[18]);
}
