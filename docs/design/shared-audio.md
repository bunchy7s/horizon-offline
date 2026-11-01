# Shared audio output

Part 2 of the splitscreen series (#3541). This adds the mixer API; the game still uses one
listener until the player-session integration lands. No launcher changes or settings are added.

`Audio::share(owner)` creates another sound engine on the existing output and decoded sample
cache. Give each live engine its own nonzero owner ID. Call `set_world` with the same world or
dimension identity for engines that can hear each other's world sounds, then update each one's
camera with `set_listener`. Existing channels keep their original world when a listener travels.

A world sound is heard from the nearest camera in its world. Copies heard by different engines
at nearly the same position within 150 ms are played once; relative sounds and looping sources
keep their own channels. Numeric suffixes identify vanilla's sound-file variants. Shared stereo
panning is reduced while multiple listeners share the speakers.

Dropping a shared engine removes its sources and listener. Replacement engines get fresh channel
IDs even when they reuse an owner ID, so a late decoder reply cannot attach to a replacement's
sound. Keep the original `Audio` alive: it owns the output stream.

Run the deterministic mixer tests without an audio device or game resources:

```sh
cargo test --profile fast -p horizon-client-render --lib audio::tests
```
