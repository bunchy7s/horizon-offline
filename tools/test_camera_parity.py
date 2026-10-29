#!/usr/bin/env python3
"""Compare the camera with the pinned client: Camera.tick (eye height and
field-of-view easing), alignWithEntity (view rotation quaternion and basis,
interpolated eye, third person back and front, camera distance and scale
attributes with mounts, getMaxZoom clipping against visual shapes, sleeping),
calculateFov/calculateHudFov (option, modifier, death and fluid effects,
panoramic mode) with the world projection and the first-person hand's
(GameRenderer.render3dHud at hudFov, whatever the FOV option),
AbstractClientPlayer.getFieldOfViewModifier (flight, speed attribute against
walking speed, bow draw, spyglass), the walk and bob
history (ClientAvatarState, updateBob) and GameRenderer.bobHurt/bobView,
driven tick by tick and frame by frame with identical scripted states."""
import json
import random
import struct

try:
    from . import lab, oracle_harness as harness
except ImportError:
    import lab, oracle_harness as harness


def f32(v):
    return struct.unpack('>f', struct.pack('>f', v))[0]


WALK = [0.1, 0.0, 0.05, 0.2, 1e-3, -0.1, 0.15]
SPEED = [f32(0.1), f32(0.1) * 1.3, 0.0, 0.03, 0.5, 1024.0, 0.13, f32(0.1) * 0.85, f32(0.1) * 1.4]
TICKS = [0, 1, 5, 10, 19, 20, 21, 72000, -5, 3]


def fov_fields(rng):
    use = rng.choice([None, None, ['bow', rng.choice(TICKS)], ['spyglass', rng.choice(TICKS)], ['other', rng.choice(TICKS)]])
    return dict(flying=rng.random() < 0.3, ws=lab.fbits(rng.choice(WALK + [rng.uniform(-0.3, 0.3)])),
                speed=lab.dbits(rng.choice(SPEED + [rng.uniform(0, 2)])), use=use)


def fov_cases(rng):
    for flying in (False, True):
        for ws in WALK:
            for speed in SPEED:
                for use in [None, ['bow', 0], ['bow', 7], ['bow', 20], ['bow', 40], ['spyglass', 2], ['other', 5]]:
                    for fp in (False, True):
                        for scale in (0.0, 0.5, 1.0):
                            yield dict(t='fov', flying=flying, ws=lab.fbits(ws), speed=lab.dbits(speed), use=use, fp=fp, scale=lab.fbits(scale))
    for _ in range(3000):
        yield dict(t='fov', **fov_fields(rng), fp=rng.random() < 0.6, scale=lab.fbits(rng.choice([0.0, 1.0, rng.random()])))


FULL = [[0, 0, 0, 1, 1, 1]]
SHAPES = [FULL, FULL, [[0, 0, 0, 1, 0.5, 1]], [[0, 0.5, 0, 1, 1, 1]], [[0, 0, 0, 1, 1, 0.125]], [[0.375, 0, 0.375, 0.625, 1, 0.625]],
          [[0, 0, 0, 0.5, 1, 1], [0.5, 0, 0.5, 1, 1, 1]], []]


def world(rng, center):
    cells = {}
    cx, cy, cz = center
    for _ in range(rng.choice([0, 3, 10, 40, 120])):
        p = (cx + rng.randint(-6, 6), cy + rng.randint(-3, 5), cz + rng.randint(-6, 6))
        cells[f'{p[0]} {p[1]} {p[2]}'] = [[lab.dbits(v) for v in box] for box in rng.choice(SHAPES)]
    return cells


ANGLES = [0.0, 90.0, -90.0, 180.0, -180.0, 45.0, 360.0, 720.5, -33.3, 1e-3, 89.9]


def angle(rng):
    return rng.choice(ANGLES + [rng.uniform(-90, 90), rng.uniform(-400, 400)])


def entity_state(rng, center):
    pos = [center[i] + rng.uniform(0, 1) for i in range(3)]
    prev = [p - rng.choice([0, 0, 0.1, 0.3, 1.0, rng.uniform(-1, 1)]) for p in pos]
    kind = 'player' if rng.random() < 0.85 else 'marker'
    xr, yr = angle(rng), angle(rng)
    s = dict(kind=kind, pos=[lab.dbits(v) for v in pos], prev=[lab.dbits(v) for v in prev], xr=lab.fbits(xr), yr=lab.fbits(yr),
             xro=lab.fbits(rng.choice([xr, xr - 5, angle(rng)])), yro=lab.fbits(rng.choice([yr, yr + 10, angle(rng)])),
             eye=lab.fbits(rng.choice([1.62, 1.27, 0.4, 0.2, 0.81, 3.24, rng.uniform(0, 4)])))
    s.update(fov_fields(rng))
    s.update(scale_attr=lab.dbits(rng.choice([1.0, 1.0, 0.5, 2.0, 0.0625, 16.0, rng.uniform(0.0625, 3)])),
             dist=lab.dbits(rng.choice([4.0, 4.0, 0.0, 1.0, 10.0, 32.0, rng.uniform(0, 32)])),
             sleeping=rng.choice([None] * 6 + [[None], [0], [1], [2], [3]]),
             dead=rng.choice([None] * 6 + [0, 1, 5, 19, 20, 25]),
             mount=rng.choice([None] * 4 + [[lab.dbits(rng.choice([1.0, 2.0, 0.5])), lab.dbits(rng.choice([4.0, 8.0, 1.0]))]]))
    return s


def window(rng):
    """Window size and Camera.depthFar for the world and hand projections."""
    w, h = rng.choice([(1600, 1000), (854, 480), (1920, 1080), (800, 1200), (1, 1), (rng.randint(1, 4000), rng.randint(1, 3000))])
    return [lab.fbits(w), lab.fbits(h), lab.fbits(rng.choice([768.0, 128.0, 2048.0, 4096.0, rng.uniform(32, 4096)]))]


def camera_cases(rng):
    for n in range(2500):
        center = rng.choice([(0, 64, 0), (-10, -60, 7), (1000, 120, -3000), (0, 0, 0)])
        ops = []
        for _ in range(rng.randrange(1, 40)):
            kind = rng.choice(['entity', 'tick', 'tick', 'tick', 'frame', 'frame', 'frame', 'type'])
            if not ops:
                kind = 'entity'
            if kind == 'entity':
                ops.append(['entity', entity_state(rng, center)])
            elif kind == 'tick':
                ops.append(['tick'])
            elif kind == 'frame':
                pt = rng.choice([0.0, 1.0, 0.5, 0.25, 0.999, rng.random()])
                ops.append(['frame', lab.fbits(pt), rng.choice([0, 0, 0, 1, 2, 3]), rng.random() < 0.05] + window(rng))
            else:
                ops.append(['type', rng.randrange(3)])
        yield dict(t='camera', fov=rng.choice([70, 30, 110, 90, rng.randint(30, 110)]),
                   effect=lab.dbits(rng.choice([1.0, 0.0, 0.5, rng.random()])), type=rng.choice([0, 1, 1, 2]),
                   cells=world(rng, center), ops=ops)
    # The hand's projection at FOV 30, 70 and 110 while walking, sprinting, under
    # speed, drawing a bow, dying and in fluids: GameRenderer.render3dHud uses
    # hudFov, never the option or the modifier.
    for fov in (30, 70, 110):
        for speed in (f32(0.1), f32(0.1) * 1.3, 0.2, 0.05):
            for use in (None, ['bow', 10]):
                for dead in (None, 5):
                    for fluid in (0, 1, 2):
                        s = dict(kind='player', pos=[lab.dbits(0.5), lab.dbits(64.0), lab.dbits(0.5)], prev=[lab.dbits(0.5), lab.dbits(64.0), lab.dbits(0.5)],
                                 xr=lab.fbits(0.0), yr=lab.fbits(0.0), xro=lab.fbits(0.0), yro=lab.fbits(0.0), eye=lab.fbits(1.62), flying=False,
                                 ws=lab.fbits(0.1), speed=lab.dbits(speed), use=use, scale_attr=lab.dbits(1.0), dist=lab.dbits(4.0),
                                 sleeping=None, dead=dead, mount=None)
                        ops = [['entity', s]] + [['tick']] * 8 + [['frame', lab.fbits(pt), fluid, False, lab.fbits(1600.0), lab.fbits(1000.0), lab.fbits(768.0)]
                                                                 for pt in (0.0, 0.5, 1.0)]
                        yield dict(t='camera', fov=fov, effect=lab.dbits(1.0), type=0, cells={}, ops=ops)
    # A walled-in third-person camera stepping through every distance bound.
    for dist in [0.0, 0.5, 1.0, 2.0, 4.0, 8.0, 32.0]:
        for wall in range(0, 6):
            for yaw in [0.0, 90.0, 180.0, 270.0, 45.0]:
                cells = {}
                for dx in range(-6, 7):
                    for dy in range(62, 68):
                        cells[f'{dx} {dy} {-wall - 1}'] = [[lab.dbits(v) for v in box] for box in FULL]
                        cells[f'{dx} {dy} {wall + 1}'] = [[lab.dbits(v) for v in box] for box in FULL]
                s = dict(kind='player', pos=[lab.dbits(0.5), lab.dbits(64.0), lab.dbits(0.5)], prev=[lab.dbits(0.5), lab.dbits(64.0), lab.dbits(0.5)],
                         xr=lab.fbits(15.0), yr=lab.fbits(yaw), xro=lab.fbits(15.0), yro=lab.fbits(yaw), eye=lab.fbits(1.62), flying=False,
                         ws=lab.fbits(0.1), speed=lab.dbits(f32(0.1)), use=None, scale_attr=lab.dbits(1.0), dist=lab.dbits(dist),
                         sleeping=None, dead=None, mount=None)
                ops = [['entity', s]] + [['tick']] * 6 + [['frame', lab.fbits(pt), 0, False] + window(rng) for pt in (0.0, 0.5, 1.0)]
                yield dict(t='camera', fov=70, effect=lab.dbits(1.0), type=1 + (wall % 2), cells=cells, ops=ops)


def bob_cases(rng):
    for n in range(6000):
        living = None
        if rng.random() < 0.8:
            living = dict(hurt=lab.fbits(rng.choice([-1.0, -0.5, 0.0, 10.0, 9.5, 5.25, 1.0, rng.uniform(-2, 11)])),
                          duration=rng.choice([10, 10, 0, 5, 20]), dir=lab.fbits(rng.choice([0.0, 90.0, -45.0, 180.0, rng.uniform(-360, 360)])),
                          death=rng.choice([None, None, None, lab.fbits(rng.choice([0.0, 0.5, 19.5, 20.0, 25.0, rng.uniform(0, 30)]))]))
        player = None
        if rng.random() < 0.8:
            player = [lab.fbits(rng.choice([0.0, -0.3, -1.7, -100.25, rng.uniform(-1000, 0)])),
                      lab.fbits(rng.choice([0.0, 0.1, 0.05, 0.0999, rng.uniform(0, 0.1)]))]
        yield dict(t='bob', living=living, player=player, tilt=lab.dbits(rng.choice([1.0, 0.0, 0.5, rng.random()])),
                   view=rng.random() < 0.8)


def avatar_cases(rng):
    for n in range(1500):
        ops = []
        for _ in range(rng.randrange(1, 60)):
            kind = rng.choice(['tick', 'move', 'move', 'bob', 'bob', 'frame', 'frame', 'reset'])
            if kind == 'tick':
                ops.append(['tick'])
            elif kind == 'move':
                ops.append(['move', lab.fbits(rng.choice([0.0, 0.2158, -0.1, rng.uniform(-1, 1)])), lab.fbits(rng.choice([0.0, 0.2806, rng.uniform(-1, 1)]))])
            elif kind == 'bob':
                v = [rng.choice([0.0, 0.05, -0.2, rng.uniform(-0.5, 0.5)]), rng.uniform(-1, 1), rng.choice([0.0, 0.08, rng.uniform(-0.5, 0.5)])]
                ops.append(['bob', rng.random() < 0.7, rng.random() < 0.1, rng.random() < 0.1, [lab.dbits(x) for x in v]])
            elif kind == 'frame':
                ops.append(['frame', lab.fbits(rng.choice([0.0, 1.0, 0.5, rng.random()]))])
            else:
                ops.append(['reset'])
        yield dict(t='avatar', ops=ops)


def make_cases(build):
    rng = random.Random(0x43414d45)
    path = build / 'cases.jsonl'
    cases = build / 'cases'
    cases.mkdir(exist_ok=True)
    count = 0
    with path.open('w') as out:
        for generator in (fov_cases, camera_cases, bob_cases, avatar_cases):
            for case in generator(rng):
                text = json.dumps(case, separators=(',', ':'))
                out.write(text + '\n')
                if count < 20000:
                    (cases / str(count)).write_text(text)
                count += 1
    return path, count


def main():
    scope = ('Real Camera.tick, tickFov, alignWithEntity (setRotation quaternion and forwards/up/left basis, move, '
             'interpolated eye with eye-height easing, first person, third person back and mirrored front, scale and '
             'camera_distance attributes including a living mount, getMaxZoom over the actual Level.clip with '
             'ClipContext.Block.VISUAL against controlled visual shapes, sleeping bed orientation), calculateFov and '
             'calculateHudFov (option, eased modifier, death, water/lava with the effect scale, panoramic), the world projection '
             '(Camera.setupPerspective) and the hand projection GameRenderer.render3dHud sets up at hudFov, as JOML setPerspective '
             'matrices, at FOV options 30, 70 and 110 among others, '
             'AbstractClientPlayer.getFieldOfViewModifier with real Abilities, attribute maps and item stacks, '
             'ClientAvatarState walk/bob history with AbstractClientPlayer.updateBob, and GameRenderer.bobHurt/bobView '
             'as the full JOML pose matrix with its property flags. The camera entity is a shell with controlled '
             'queries; getFluidInCamera is supplied (the fog harnesses cover fluid detection) and the new-minecart '
             'interpolation branch is not exercised.')
    harness.differential(
        'rust-camera-parity', 'VanillaCameraOracle', 'camera_probe', make_cases, scope,
        sources=[lab.ROOT / 'crates/horizon-client-render/src/camera.rs'])


if __name__ == '__main__':
    main()
