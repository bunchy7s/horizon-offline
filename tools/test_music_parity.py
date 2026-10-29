#!/usr/bin/env python3
"""Compare the music manager with the vanilla 26.3 client.

The oracle runs the real `MusicManager` over the real `SoundEngine` (OpenAL
Soft "No Output" device, see VanillaSoundRig). The situation
(`getSituationalMusic`, `getMusicVolume`), the loading screen, pauses, the
end of a track, `stopPlaying`, `setMinutesBetweenSongs`, the music category
volume and `showNowPlayingToastIfNeeded` are script inputs; every step prints
the current track, `nextSongDelay`, `currentGain`, `toastShown`, the toast
calls, the MUSIC category gain and the music channel's AL source. Horizon's
`MusicManager` and `SoundEngine` replay the scripts. Tracks use events whose
variants share one volume, since `forMusic` resolves with an unseeded random.
"""
import random

try:
    from . import audio_harness as h
except ImportError:
    import audio_harness as h

TRACKS = ['minecraft:music.credits', 'minecraft:music.dragon', 'minecraft:music.end', 'minecraft:music.under_water',
          'minecraft:music.overworld.swamp', 'minecraft:music_disc.cat', 'minecraft:music_disc.13']
ODD = ['minecraft:music.nether.warped_forest', 'minecraft:not.a.track']


class Script:
    def __init__(self):
        self.lines = []

    def add(self, *parts):
        self.lines.append(' '.join(map(str, parts)))

    def situation(self, sound=None, lo=0, hi=0, replace=0, volume=1.0):
        self.add('situation', '-' if sound is None else f'{sound},{lo},{hi},{replace}', h.bits(volume))

    def tick(self, n=1, loading=0, paused=0):
        for _ in range(n):
            self.add('tick', loading, paused)


def scripted(s):
    end, dragon, water, credits, cat = TRACKS[2], TRACKS[1], TRACKS[3], TRACKS[0], TRACKS[5]
    s.add('start', 1, 'DEFAULT')
    # Menu-like music: 20..600 ticks, replacing.
    s.situation(end, 20, 600, 1)
    s.tick(3, loading=1)
    s.tick(120)
    s.add('finish')
    s.tick(650)
    # Game music does not replace, and the next delay comes from its range.
    s.situation(water, 12000, 24000, 0)
    s.tick(5)
    s.add('finish')
    s.tick(30)
    s.add('freq', 'FREQUENT')
    s.tick(3)
    s.add('freq', 'CONSTANT')
    s.tick(110)
    s.add('freq', 'DEFAULT')
    s.tick(2)
    # A replacing track stops the current one over the following ticks.
    s.situation(dragon, 0, 0, 1)
    s.tick(30)
    s.situation(credits, 0, 0, 1)
    s.tick(30)
    s.situation(None)
    s.tick(5)
    s.add('finish')
    s.tick(5)
    s.situation(end, 6000, 24000, 1)
    s.tick(130)
    # Music volume fades: down to silence, then up from a partial volume.
    s.situation(end, 6000, 24000, 1, volume=0.0)
    s.tick(320)
    s.situation(cat, 0, 0, 1, volume=0.25)
    s.tick(5)
    s.situation(cat, 0, 0, 1, volume=1.0)
    s.tick(400)
    s.situation(cat, 0, 0, 1, volume=0.6)
    s.tick(80)
    # Silent starts and the toast, stops, pauses.
    s.add('volume', 1, h.bits(0.0))
    s.add('stop')
    s.situation(dragon, 0, 0, 1)
    s.tick(105)
    s.add('toast')
    s.add('volume', 1, h.bits(0.5))
    s.add('toast')
    s.tick(2)
    s.add('stopmusic', f'{end},0,0,1')
    s.add('stopmusic', f'{dragon},0,0,1')
    s.tick(105, paused=1)
    s.add('finish')
    s.tick(3, paused=1)
    s.tick(5)
    s.add('stopall')
    s.tick(5)
    # Joining a world: the menu-like track plays, Minecraft.setLevel stops every
    # sound, and the game music (not replacing) waits its own delay; a respawn
    # into another dimension then stops the music manager's track too.
    s.add('start', 11, 'DEFAULT')
    s.situation(end, 20, 600, 1)
    s.tick(110)
    s.add('stopall')
    s.situation(water, 12000, 24000, 0)
    s.tick(3, loading=1)
    s.tick(5)
    s.add('stopall')
    s.add('stop')
    s.tick(5)
    # Events that cannot play.
    for event in ODD:
        s.add('start', 7, 'DEFAULT')
        s.situation(event, 0, 40, 1)
        s.tick(150)
    # Real Musics delays over a long session.
    s.add('start', 99, 'DEFAULT')
    s.situation(water, 12000, 24000, 0)
    s.tick(130)
    s.add('finish')
    s.tick(24100)


def fuzz(s, rng, steps):
    s.add('start', rng.getrandbits(63), rng.choice(['DEFAULT', 'FREQUENT', 'CONSTANT']))
    for _ in range(steps):
        op = rng.random()
        if op < 0.08:
            sound = rng.choice(TRACKS + ODD[:1]) if rng.random() < 0.9 else None
            lo = rng.choice([0, 20, 100, 300])
            s.situation(sound, lo, lo + rng.choice([0, 50, 280, 600]), int(rng.random() < 0.5),
                        rng.choice([1.0, 1.0, 1.0, 0.0, 0.5, rng.random()]))
        elif op < 0.13:
            s.add('finish')
        elif op < 0.15:
            s.add('stop')
        elif op < 0.16:
            s.add('stopmusic', f'{rng.choice(TRACKS)},0,0,1')
        elif op < 0.17:
            s.add('freq', rng.choice(['DEFAULT', 'FREQUENT', 'CONSTANT']))
        elif op < 0.18:
            s.add('toast')
        elif op < 0.20:
            s.add('volume', rng.choice([0, 1]), h.bits(rng.choice([0.0, 1.0, 0.5])))
        elif op < 0.21:
            s.add('stopall')
        elif op < 0.25:
            s.tick(rng.randrange(1, 30), loading=1)
        elif op < 0.28:
            s.tick(rng.randrange(1, 30), paused=1)
        else:
            s.tick(rng.randrange(1, 60))


def fixtures():
    s = Script()
    scripted(s)
    rng = random.Random(0x4d55534943)
    for _ in range(60):
        fuzz(s, rng, 120)
    return s.lines


def main():
    lines = fixtures()
    matched, report = h.run_harness(
        'music', 'VanillaMusicOracle', 'music_fixture', ''.join(l + '\n' for l in lines),
        extra_sources=[h.lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaSoundRig.java',
                       h.lab.ROOT / 'crates/horizon-client/src/music.rs',
                       h.lab.ROOT / 'crates/horizon-client/examples/probe/audio_common/mod.rs'],
        java_args=[h.RESOURCES], rust_args=[h.RESOURCES],
        scope='Real MusicManager.tick/startPlaying/stopPlaying/fadePlaying/setMinutesBetweenSongs/showNowPlayingToastIfNeeded with a seeded '
              'random over the real SoundEngine: track starts and replacements, nextSongDelay draws, the three music frequencies, the loading '
              'screen hold, fades and the MUSIC category gain, silent starts, toasts, pauses, track ends and unplayable events. The situation '
              '(getSituationalMusic/getMusicVolume) is an input here; see test_audio_attributes_parity.py for the attribute values.')
    print(f'{matched:,} music lines match the pinned client ({len(lines):,} steps): {report}', flush=True)


if __name__ == '__main__':
    main()
