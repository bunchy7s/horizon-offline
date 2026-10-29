#!/usr/bin/env python3
"""Compare options.txt loading and saving, and the key mapping table, with the
pinned client: every file is loaded through the real Options.load and written
back with Options.save. The Rust probe normalizes Horizon's intentional 33..128
render-distance extension to vanilla's default; a Rust slider/persistence
regression covers the extended range."""
from pathlib import Path
import random
import shutil

try:
    from . import lab, oracle_harness as harness
except ImportError:
    import lab, oracle_harness as harness

VERSION = lab.version_info()['world_version']

# Raw JSON-ish value texts: valid, boundary, wrong-typed, lenient-parser and
# syntax-error forms. Syntax errors abort the rest of a load, as in vanilla.
TOKENS = [
    'true', 'false', 'TRUE', 'False', '"true"', '0', '1', '2', '3', '4', '5', '-1', '-3', '7', '10', '11', '12', '13',
    '16', '27', '29', '30', '31', '32', '33', '60', '65', '69', '70', '71', '100', '109', '110', '111', '120', '128',
    '129', '130', '255', '256', '260', '261', '1000', '2147483646', '2147483647', '2147483648', '-2147483648',
    '0.0', '0.1', '0.25', '0.3', '0.5', '0.75', '0.77', '0.99', '1.0', '1.5', '2.0', '2.5', '5.0', '5.5', '6.0',
    '10.0', '10.5', '-0.5', '-0.0', '1e0', '1e1', '1E-2', '1e2', '2.5e-1', '0.0001', '1e30', '12.7', '4.99',
    '"fast"', '"fancy"', '"fabulous"', '"custom"', '"afk"', '"minimized"', '"left"', '"right"', '"default"',
    '"opengl"', '"all"', '"none"', '"limited"', '"never"', '"pause"', '"pause_and_toast"', '"DEFAULT"', '"FREQUENT"',
    '"CONSTANT"', '"false"', '"movement"', '"Fast"', "'fast'", 'fast', 'afk', 'left', 'abc', '', ' ', '""', 'null',
    '[]', '[1,2]', '["vanilla"]', '["vanilla","file/x.zip"]', '{}', '[vanilla,a]', '["a<b>&=\'"]', '0.5f', '.5',
    '007', '+1', 'NaN', 'Infinity', '-Infinity', '0x10', '1 2', '"unterminated', '{', ']', '1,2', '0.5 # note',
    '0.5 // note', '/* c */ 0.5', '0.5;', ')]}\'\n0.5', 'tRuE', 'nul', 'true false',
]
KEY_TOKENS = [
    'key.keyboard.w', 'key.keyboard.unknown', 'key.mouse.left', 'key.mouse.9', 'key.mouse.99', 'key.keyboard.200',
    'key.keyboard.007', 'key.keyboard.-5', 'key.keyboard.+4', 'key.keyboard.', 'key.keyboard', 'key.mouse', 'key.mousefoo',
    'key.keyboardX5', 'key.keyboard.abc', 'key.keyboard.f13', 'key.keyboard.keypad.0', 'key.keyboard.right.shift',
    '', 'w', 'key.w', 'scancode.17', 'key.keyboard.4294967296', 'key.keyboard.2147483647', 'key.keyboard.2147483648',
    'KEY.KEYBOARD.W', 'key.keyboard.menu', 'key.keyboard.f24',
]
PLAIN_TOKENS = ['true', 'false', 'True', '1', '0', 'yes', '', ' true', 'abc', '5', '-1', '2147483648', '+3', '007', '1.0']
STRING_TOKENS = ['en_us', 'FR_fr', '', ' a:b ', 'a:b:c', 'Ünïcode', '"quoted"', 'x' * 300]
TAIL = 'musicFrequency:"FREQUENT"\nmodelPart_hat:false\n'  # shows whether a load was aborted


def option_keys():
    """Option-instance keys, read from the Rust table through the probe-independent
    list below (generated from VanillaOptionsTableExporter output)."""
    return ALL_KEYS


# Every key the 26.3 Options writes, in processing order (verified by the
# defaults case: a mismatch in either name or order fails the comparison).
ALL_KEYS = [
    'ao', 'biomeBlendRadius', 'chunkSectionFadeInTime', 'cutoutLeaves', 'enableVsync', 'entityDistanceScaling',
    'entityShadows', 'forceUnicodeFont', 'japaneseGlyphVariants', 'fov', 'fovEffectScale', 'darknessEffectScale',
    'glintSpeed', 'glintStrength', 'preferredGraphicsBackend', 'graphicsPreset', 'prioritizeChunkUpdates',
    'fullscreen', 'exclusiveFullscreen', 'macFullscreenMenuVisibility', 'gamma', 'guiScale', 'debugGuiScale',
    'maxAnisotropyBit', 'textureFiltering', 'maxFps', 'improvedTransparency', 'inactivityFpsLimit', 'mipmapLevels',
    'narrator', 'particles', 'reducedDebugInfo', 'renderClouds', 'cloudRange', 'renderDistance',
    'simulationDistance', 'screenEffectScale', 'soundDevice', 'vignette', 'weatherRadius', 'autoJump',
    'rotateWithMinecart', 'operatorItemsTab', 'autoSuggestions', 'chatColors', 'chatLinks', 'chatLinksPrompt',
    'discrete_mouse_scroll', 'invertXMouse', 'invertYMouse', 'realmsNotifications', 'showSubtitles',
    'directionalAudio', 'bobView', 'toggleCrouch', 'toggleSprint', 'toggleAttack', 'toggleUse', 'sprintWindow',
    'darkMojangStudiosBackground', 'hideLightningFlashes', 'hideSplashTexts', 'mouseSensitivity',
    'damageTiltStrength', 'highContrast', 'highContrastBlockOutline', 'narratorHotkey', 'resourcePacks',
    'incompatibleResourcePacks', 'lastServer', 'lang', 'chatVisibility', 'chatOpacity', 'chatLineSpacing',
    'textBackgroundOpacity', 'backgroundForChatOnly', 'hideServerAddress', 'advancedItemTooltips',
    'pauseOnLostFocus', 'overrideWidth', 'overrideHeight', 'chatHeightFocused', 'chatDelay', 'chatHeightUnfocused',
    'chatScale', 'chatWidth', 'notificationDisplayTime', 'useNativeTransport', 'mainHand', 'attackIndicator',
    'tutorialStep', 'mouseWheelSensitivity', 'allowCursorChanges', 'quitShortcuts', 'ctrlClickEmulatesRightClick',
    'glDebugVerbosity', 'skipMultiplayerWarning', 'hideMatchedNames', 'joinedFirstServer', 'syncChunkWrites',
    'showAutosaveIndicator', 'allowServerListing', 'inGameNotification', 'sharePresence', 'onlyShowSecureChat',
    'saveChatDrafts', 'panoramaScrollSpeed', 'telemetryOptInExtra', 'onboardAccessibility',
    'menuBackgroundBlurriness', 'startedCleanly', 'musicToast', 'musicFrequency',
]
PLAIN = {'lastServer', 'lang', 'hideServerAddress', 'advancedItemTooltips', 'pauseOnLostFocus', 'overrideWidth',
         'overrideHeight', 'useNativeTransport', 'glDebugVerbosity', 'skipMultiplayerWarning', 'joinedFirstServer',
         'syncChunkWrites', 'onboardAccessibility', 'startedCleanly'}
PLAIN_STRINGS = {'lastServer', 'lang'}
MAPPINGS = None
SOUNDS = ['master', 'music', 'record', 'weather', 'block', 'hostile', 'neutral', 'player', 'ambient', 'voice', 'ui']
PARTS = ['cape', 'jacket', 'left_sleeve', 'right_sleeve', 'left_pants_leg', 'right_pants_leg', 'hat']


def write(directory, index, text, newline='\n'):
    (directory / f'{index:05d}.txt').write_bytes(text.replace('\n', newline).encode('utf-8'))


def make_cases(build):
    directory = build / 'cases'
    shutil.rmtree(directory, ignore_errors=True)
    directory.mkdir()
    rng = random.Random(0x4f5054)
    n = 0
    # The no-file case is named absent.txt so the oracle skips the copy.
    (directory / 'absent.txt').write_text('')
    n += 1
    header = f'version:{VERSION}\n'
    # One option per file with each token: isolates every codec path.
    for key in ALL_KEYS:
        tokens = PLAIN_TOKENS if key in PLAIN and key not in PLAIN_STRINGS else (
            STRING_TOKENS if key in PLAIN_STRINGS else TOKENS)
        for token in tokens:
            write(directory, n, f'{header}{key}:{token}\n{TAIL}')
            n += 1
    # Round trips of values the game itself would save.
    for key in ALL_KEYS:
        write(directory, n, f'{header}{key}:{rng.choice(TOKENS)}\n{key}:{rng.choice(TOKENS)}\n')
        n += 1
    # Key mappings, sound volumes and model parts.
    keys = ['key.attack', 'key.use', 'key.forward', 'key.sneak', 'key.sprint', 'key.hotbar.1', 'key.swapOffhand',
            'key.debug.modifier', 'key.debug.profilingChart', 'key.spectatorHotbar', 'key.nonexistent']
    for key in keys:
        for token in KEY_TOKENS:
            write(directory, n, f'{header}key_{key}:{token}\n{TAIL}')
            n += 1
    for source in SOUNDS + ['nonexistent']:
        for token in TOKENS[:60]:
            write(directory, n, f'{header}soundCategory_{source}:{token}\n{TAIL}')
            n += 1
    for part in PARTS + ['nonexistent']:
        for token in PLAIN_TOKENS:
            write(directory, n, f'{header}modelPart_{part}:{token}\n')
            n += 1
    # Random multi-option files, some with CRLF/CR line ends, noise and duplicates.
    syntax_errors = {'1 2', '"unterminated', '{', ']', '1,2', 'true false', '0.5 // note'}
    safe = [t for t in TOKENS if t not in syntax_errors]
    for _ in range(1500):
        lines = [] if rng.random() < 0.1 else [f'version:{VERSION}']
        for _ in range(rng.randrange(1, 40)):
            key = rng.choice(ALL_KEYS)
            pool = safe if rng.random() < 0.9 else TOKENS
            lines.append(f'{key}:{rng.choice(PLAIN_TOKENS if key in PLAIN and key not in PLAIN_STRINGS else pool)}')
            if rng.random() < 0.05:
                lines.append(rng.choice(['', 'garbage', ':', 'novalue:', ':novalue', 'a:b:c', 'fullscreenResolution:1920x1080@60:24']))
        newline = rng.choice(['\n', '\n', '\r\n', '\r'])
        write(directory, n, '\n'.join(lines) + rng.choice(['', '\n']), newline)
        n += 1
    # Older files exercise the data fixers; each version boundary of the OPTIONS fixers.
    legacy = [
        'fancyGraphics:true\nao:2\nlang:EN_US\nchatOpacity:0.4\nmenuBackgroundBlurriness:0.5\nkey_key.forward:17\n'
        'key_key.attack:-100\nkey_key.use:-99\nkey_key.pickItem:-98\nkey_key.jump:-95\nresourcePacks:["programer_art"]\n'
        'showNowPlayingToast:true\n',
        'fancyGraphics:false\nao:0\nkey_key.forward:key.w\nkey_key.swapHands:key.f\nmouseSensitivity:0.5\n',
        'graphicsMode:2\nao:1\nmenuBackgroundBlurriness:abc\nchatOpacity:abc\nshowNowPlayingToast:false\n',
        'graphicsMode:0\nmenuBackgroundBlurriness:0.35\nchatOpacity:1e-3\n',
        'graphicsMode:1\nlang:Fr_Ca\nresourcePacks:["a","programer_art"]\nincompatibleResourcePacks:["programer_art"]\n',
        'key_key.forward:abc\n', 'key_key.forward:\n', 'key_key.attack:k\n', 'chatOpacity:Infinity\n',
        'chatOpacity: 0.5d\n', 'menuBackgroundBlurriness:0.25\n', 'menuBackgroundBlurriness:1.04999\n',
        # OptionsForceVBOFix writes a key nothing reads; the lower-casing, key and graphics fixers on odd values.
        'useVbo:false\nao:true\nao:3\n', 'ao:\n', 'graphicsMode:3\nao:2\n', 'graphicsMode:abc\n', 'fancyGraphics:abc\n',
        'fancyGraphics:TRUE\n', 'lang:\u039f\u0394\u03a5\u03a3\u03a3\u0395\u03a5\u03a3\n', 'lang:\u0130_TR\n', 'lang:\u00df_\u00dc\n',
        'key_key.attack:-101\nkey_key.use:-92\nkey_key.forward:2147483647\nkey_key.back:99999\nkey_key.left:-2147483648\n',
        'key_key.hotbar.1:2\nkey_key.hotbar.9:10\nkey_key.swapHands:33\nkey_key.drop:16\nkey_key.debug.modifier:292\n',
        'showNowPlayingToast:maybe\n', 'showNowPlayingToast:true\nmusicToast:never\n',
        'resourcePacks:["programer_art","x"]\nincompatibleResourcePacks:[]\n', 'resourcePacks:[\'programer_art\']\n',
        'menuBackgroundBlurriness:-3\n', 'menuBackgroundBlurriness:1e3\n', 'menuBackgroundBlurriness:\n', 'chatOpacity:0\n',
        'chatOpacity:1\ntextBackgroundOpacity:0.2\n', 'preferredGraphicsBackend:vulkan\n', 'onboardAccessibility:true\n',
    ]
    versions = [-1, 0, 100, 504, 505, 815, 816, 1343, 1344, 1445, 1446, 1935, 1936, 2555, 2556, 2557, 2558, 3200, 3201,
                3213, 3214, 3318, 3319, 3942, 3943, 4650, 4651, 4660, 4661, 4891, 4892, 5022, 5023, 5024, 9999]
    for version in versions:
        for body in legacy:
            write(directory, n, f'version:{version}\n{body}')
            n += 1
    for body in legacy:
        write(directory, n, body)
        n += 1
        write(directory, n, f'version:abc\n{body}')
        n += 1
        write(directory, n, f'version: 5023\n{body}')
        n += 1
    for text in ['fullscreenResolution:1920x1080@60:24\n', 'fullscreenResolution:\n', 'fov:abc\nfullscreenResolution:x\n',
                 'fov:1 2\nfullscreenResolution:x\n', '﻿version:5023\nfov:0.5\n', 'FOV:0.5\n', ' fov:0.5\n', 'fov :0.5\n']:
        write(directory, n, f'version:{VERSION}\n{text}')
        n += 1
    return directory, n


def main():
    scope = ('Real Options.load and Options.save (26.3) on generated options.txt files: the full option table in '
             'processing order with defaults, every codec/validator path (int ranges, quantised doubles, unit doubles with '
             'boolean alternative, string and id enums with wrapping, clouds, fov, frame limit, scroll sensitivity), plain '
             'fields, lenient Gson parsing and syntax-error aborts, key mappings (names, scancode-number forms, unbinding), '
             'sound volumes, model parts, line endings, the OPTIONS data fixers across version boundaries, and the '
             'KeyMapping table (order, defaults, categories, toggle mappings). Does not cover option screens or '
             'runtime side effects of setting an option. Horizon render distances 33..128 are normalized to vanilla\'s '
             'default by options_probe; the slider/persistence unit regression checks that intentional extension.')
    harness.differential(
        'rust-options-parity', 'VanillaOptionsOracle', 'options_probe', make_cases, scope,
        sources=[lab.ROOT / 'crates/horizon-client-render/src/options_file.rs', lab.ROOT / 'crates/horizon-client-render/src/options_fixes.rs',
                 lab.ROOT / 'crates/horizon-client-render/src/java_text.rs', lab.ROOT / 'crates/horizon-client-input/src/key_names.rs',
                 lab.ROOT / 'crates/horizon-client-input/src/keybinds.rs'])


if __name__ == '__main__':
    main()
