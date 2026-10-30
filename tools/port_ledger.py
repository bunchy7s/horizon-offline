#!/usr/bin/env python3
"""The port ledger: every vanilla method next to the Horizon functions that port it, both ways, and the gaps.

vanilla_map.py maps vanilla classes to the files that name them. The ledger goes one level down and both
ways: each vanilla method (from the game jar's bytecode and the decompiled sources) to the Rust functions,
with their line spans, that port it, and each Rust function back to the vanilla methods it ports. A port is
found by its doc citing the method (`Class.method`, `Class::method`, `Class#method`), by a same-named fn in
a file or impl named for the class, by the call graph (a callee of a port with a callee of the vanilla
method), by the state and calls a fn shares with the method in the class's home files, or from what both
sides share that is hard to share by chance: rare numbers (0.458, 72000) and game ids
(`SoundEvents.ENTITY_SHULKER_BULLET_HIT` against "entity.shulker_bullet.hit"). What none of that finds,
the verdict map (tools/port_map/*.tsv) answers: a fn, the fn it is folded into, missing, or why it needs
no port.

Both sides' call graphs then show where a port exists but is not wired like vanilla, the kind of bug a
method-by-method reading misses:

- seam: a vanilla method ported on both paths (the in-frame game and the server-client side,
  game/remote*.rs, or the server, game/lan*.rs), one of which reaches the port of a callee and the other
  doesn't;
- callee: a ported vanilla method whose ported callee no Rust counterpart reaches;
- client: a vanilla effect only the client's copy draws (`addParticle`, `playLocalSound`,
  `itemUsed`, a `isClientSide()` branch) whose port nothing on the server-client side (game/remote*.rs)
  reaches;
- class: a whole vanilla class nothing ports while most of its package is ported (a renderer, a sound
  instance, a goal: the game reaches those by registration, so no call graph shows them);
- drift: a port that never names what its vanilla method uses: a sound, particle, item, block, effect or
  game event, or a rare number (a missing sound or a changed constant);
- stale: a citation whose class or method doesn't exist in the pinned version;
- unported: vanilla methods with no port, ranked by how many vanilla methods call them.

    python3 tools/port_ledger.py build                         # index into .local/port-ledger/ledger.json
    python3 tools/port_ledger.py where KineticWeapon.damageEntities
    python3 tools/port_ledger.py where ShulkerBulletRenderer   # a whole class
    python3 tools/port_ledger.py what crates/horizon-client/src/game/spear.rs:150
    python3 tools/port_ledger.py class Shulker                 # every method: ported where, or not
    python3 tools/port_ledger.py issue 3123                    # the classes, code and gaps a report points at
    python3 tools/port_ledger.py check [--base origin/main]    # what a branch touches, gaps it opens and closes
    python3 tools/port_ledger.py coverage [--package world/entity] [--depth 3]
    python3 tools/port_ledger.py gaps [--kind seam] [--package world/entity] [--limit 50]
    python3 tools/port_ledger.py report                        # .local/port-ledger/report.md and report.html
    python3 tools/port_ledger.py diff OTHER/ledger.json        # against another build
    python3 tools/port_ledger.py todo [--package world/entity] # the next methods with no answer

Add --json to any query for machine output. `build` reads the game jar (the lab's or the launcher's; --jar,
--no-jar) and the lab's decompiled sources (.local/minecraft/<version>/sources/{client,server},
tools/lab.py decompile; --sources takes any directory of decompiled Java). Queries read the last build;
`build` again after pulling. docs/port-ledger.md has the details.
"""

import argparse
import bisect
from collections import Counter, defaultdict
from concurrent.futures import ProcessPoolExecutor
import json
import math
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(os.environ.get("PORT_LEDGER_ROOT") or Path(__file__).resolve().parents[1])
OUT = Path(os.environ.get("PORT_LEDGER_OUT") or ROOT / ".local" / "port-ledger")
LEDGER = OUT / "ledger.json"

JAVA_KEYWORDS = {
    "if", "for", "while", "switch", "catch", "synchronized", "return", "new", "throw", "else", "do", "try",
    "case", "assert", "super", "this", "instanceof", "yield",
}
RUST_KEYWORDS = {
    "if", "while", "match", "for", "loop", "return", "fn", "let", "move", "in", "as", "else", "unsafe",
    "where", "impl", "dyn", "ref", "mut", "async", "await", "break", "continue", "use", "pub", "crate",
    "self", "super", "struct", "enum", "type", "const", "static", "trait", "mod",
}
# Calls too generic to mean anything on their own (collections, math, builders, logging).
GENERIC_CALLS = {
    "get", "set", "add", "put", "remove", "contains", "size", "len", "is_empty", "isEmpty", "map", "filter",
    "iter", "into_iter", "collect", "unwrap", "unwrap_or", "clone", "to_string", "from", "into", "new",
    "push", "pop", "insert", "min", "max", "abs", "floor", "ceil", "sqrt", "sin", "cos", "clamp", "lerp",
    "equals", "hashCode", "toString", "of", "copy", "apply", "accept", "test", "run", "call", "value",
    "values", "keySet", "entrySet", "stream", "forEach", "orElse", "isPresent", "ifPresent", "getValue",
    "getKey", "length", "append", "format", "is_some", "is_none", "unwrap_or_default", "as_ref", "as_mut",
    "extend", "retain", "find", "any", "all", "position", "first", "last", "sort", "dedup", "default",
}
# Numbers that say nothing: indices, common scales, angles, tick rates.
TRIVIAL_NUMBERS = {
    "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "15", "16", "20", "24", "32", "40",
    "50", "60", "64", "100", "127", "128", "180", "200", "255", "256", "360", "1000", "0.0", "0.5", "1.0",
    "2.0", "0.1", "0.2", "0.25", "0.75", "0.3", "0.4", "0.6", "0.8", "0.9", "1.5", "3.0", "4.0", "10.0",
    "16.0", "20.0", "90", "90.0", "180.0", "360.0", "0.01", "0.001", "100.0", "-1", "1.0e-5", "1.0e-7",
}
# Vanilla classes whose UPPER_CASE fields name game ids (SoundEvents.ELYTRA_FLYING is "item.elytra.flying").
ID_HOLDERS = {
    "SoundEvents", "Items", "Blocks", "ParticleTypes", "EntityType", "MobEffects", "Enchantments",
    "Attributes", "DamageTypes", "BlockTags", "ItemTags", "EntityTypeTags", "DamageTypeTags", "GameRules",
    "CriteriaTriggers", "Stats", "MemoryModuleType", "SensorType", "Activity", "PoiTypes", "VillagerProfession",
    "BlockEntityType", "MenuType", "RecipeType", "DataComponents", "Fluids", "Biomes", "Structures",
    "LootTables", "BuiltInLootTables", "GameEvent", "Instruments", "PaintingVariants", "BannerPatterns",
}
# Maths, geometry, randomness and text: every path calls them its own way, so a missing call says nothing.
UTILITY_CLASSES = {
    "Mth", "AABB", "Vec3", "Vec3i", "Vec2", "BlockPos", "SectionPos", "ChunkPos", "Direction", "RandomSource",
    "Util", "Component", "MutableComponent", "Optional", "Math", "List", "Map", "Set", "Collection", "Iterator",
}
# Vanilla calls whose effect only the client's copy of the world shows.
CLIENT_EFFECTS = re.compile(
    r"\.addParticle\(|\.addAlwaysVisibleParticle\(|\.playLocalSound\(|\bitemUsed\(|\.isClientSide\(\)"
)
SKIP_PACKAGES = (
    "net/minecraft/gametest/", "net/minecraft/data/", "net/minecraft/util/datafix/", "net/minecraft/obfuscate/",
    "com/mojang/realmsclient/", "net/minecraft/client/telemetry/", "net/minecraft/util/profiling/",
    "com/mojang/blaze3d/", "com/mojang/renderpearl/", "net/minecraft/server/jsonrpc/", "net/minecraft/util/filefix/",
    "net/minecraft/client/data/",
)

CITE = re.compile(r"\b([A-Z][A-Za-z0-9]*(?:\.[A-Z][A-Za-z0-9]*)*)(?:(?:\.|::|#)([A-Za-z_][A-Za-z0-9_]*))?")
JAVA_CLASS = re.compile(r"\b(class|interface|enum|record|@interface)\s+([A-Za-z_]\w*)")
JAVA_EXTENDS = re.compile(r"\bextends\s+([\w.]+)")
JAVA_IMPLEMENTS = re.compile(r"\bimplements\s+([\w.,\s<>?]+)")
JAVA_METHOD = re.compile(r"\b([A-Za-z_]\w*)\s*\([^()]*(?:\([^()]*\)[^()]*)*\)\s*(?:throws\s+[\w.,\s]+)?$")
JAVA_CALL = re.compile(r"(?:\b([A-Za-z_]\w*)\s*(?:\.|::)\s*|(\.|::)\s*)?\b([a-z][A-Za-z0-9_]*)\s*\(")
JAVA_METHOD_REF = re.compile(r"\b([A-Za-z_]\w*)::([a-z]\w*)\b")
JAVA_NEW = re.compile(r"\bnew\s+([A-Z][\w.]*)\s*[(<]")
JAVA_TYPED = re.compile(r"\b([A-Z][\w.]*)(?:<[^;=(){}]*?>)?(?:\[\])*\s+([a-z]\w*)\s*(?=[=;:,)])")
JAVA_ID = re.compile(r"\b(" + "|".join(sorted(ID_HOLDERS)) + r")\.([A-Z][A-Z0-9_]+)\b")
NUMBER = re.compile(r"(?<![\w.])-?(?:0[xX][0-9a-fA-F_]+|\d[\d_]*\.\d[\d_]*(?:[eE][-+]?\d+)?|\d[\d_]*(?:[eE][-+]?\d+)?)"
                    r"(?:_?(?:f32|f64|i32|i64|u8|u16|u32|u64|usize|isize|[fFdDL]))?(?![\w.])")
RUST_FN = re.compile(r"\bfn\s+([A-Za-z_]\w*)")
RUST_IMPL = re.compile(r"^\s*(?:unsafe\s+)?impl\b(?:\s*<.*?>)?\s+(?:(?:[\w:]+)(?:<.*?>)?\s+for\s+)?([\w:]+)")
RUST_TRAIT = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:unsafe\s+)?trait\s+(\w+)")
RUST_MOD = re.compile(r"(?:^|\]\s*)(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*$")
TEST_FILE = re.compile(r"(?:^|/)(?:tests?|benches|examples)/|(?:^|/|_)tests?\.rs$|_fixture\.rs$|fixture[s]?/|/probe")
RUST_CALL = re.compile(r"\b([a-z_][a-z0-9_]*)\s*(?:::\s*<[^<>]*(?:<[^<>]*>[^<>]*)*>)?\s*\(")
RUST_STRING = re.compile(r'"((?:[^"\\]|\\.)*)"')
RUST_STRUCT = re.compile(r"(?:^|\s|\])(?:pub(?:\([^)]*\))?\s+)?struct\s+([A-Z]\w*)")
RUST_IMPL_FOR = re.compile(r"^\s*(?:unsafe\s+)?impl\b(?:\s*<.*?>)?\s+([\w:]+)(?:<.*?>)?\s+for\s+([\w:]+)")
RUST_FIELD = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?([a-z_]\w*)\s*:\s*(.+?)\s*,?\s*$")
RUST_PARAM = re.compile(r"\b(?:mut\s+)?([a-z_]\w*)\s*:\s*([^,()]+(?:\([^()]*\))?[^,()]*)")
RUST_LET = re.compile(r"\blet\s+(?:mut\s+)?([a-z_]\w*)\s*(?::\s*([^=;]+?))?\s*=\s*(?:&\s*(?:mut\s+)?)?([A-Z]\w*)\s*(?:::|\{)")
RUST_METHOD_CALL = re.compile(r"(\.|::)\s*([a-z_]\w*)\s*(?:::\s*<[^<>]*(?:<[^<>]*>[^<>]*)*>)?\s*\(")
WRAPPERS = {"Option", "Box", "Arc", "Rc", "RefCell", "Cell", "Mutex", "RwLock", "Vec", "VecDeque", "HashMap",
            "BTreeMap", "HashSet", "BTreeSet", "Result", "Weak", "Cow", "OnceLock", "OnceCell", "Self", "FxHashMap",
            "FxHashSet", "MutexGuard", "Ref", "RefMut", "Pin", "PhantomData"}


def core_type(text):
    """The type a value of `text` is, for method calls: the first named type past references, wrappers and
    impl/dyn (`&mut Option<Box<Foo<'a>>>` -> Foo, `impl HandWorld` -> HandWorld)."""
    for name in re.findall(r"\b([A-Z]\w*)", text or ""):
        if name not in WRAPPERS:
            return name
    return None


# ---- Source text --------------------------------------------------------------------------------------


def blank_java(text):
    """The code with comments and literals blanked out (offsets and newlines kept), the comments and the
    string literals with their offsets."""
    out = list(text)
    strings = []
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        if text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            for k in range(i, j):
                out[k] = " "
            i = j
        elif text.startswith("/*", i):
            j = text.find("*/", i + 2)
            j = n if j < 0 else j + 2
            for k in range(i, j):
                if out[k] != "\n":
                    out[k] = " "
            i = j
        elif text.startswith('"""', i):
            j = text.find('"""', i + 3)
            j = n if j < 0 else j + 3
            strings.append((i, text[i + 3:j - 3]))
            for k in range(i, j):
                if out[k] != "\n":
                    out[k] = " "
            i = j
        elif c == '"' or c == "'":
            j = i + 1
            while j < n and text[j] != c and text[j] != "\n":
                j += 2 if text[j] == "\\" else 1
            if c == '"':
                strings.append((i, text[i + 1:j]))
            for k in range(i, min(j + 1, n)):
                out[k] = " "
            i = j + 1
        else:
            i += 1
    return "".join(out), strings


def blank_rust(text):
    """Like blank_java for Rust: line and block comments (nested), strings, raw strings, char literals but
    not lifetimes. Also gives the comments as (line, text)."""
    out = list(text)
    comments, strings = [], []
    i, n = 0, len(text)
    line = 1
    while i < n:
        c = text[i]
        if c == "\n":
            line += 1
            i += 1
        elif text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            comments.append((line, text[i:j]))
            for k in range(i, j):
                out[k] = " "
            i = j
        elif text.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if text.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif text.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            body = text[i:j]
            for offset, part in enumerate(body.split("\n")):
                comments.append((line + offset, part))
            for k in range(i, j):
                if out[k] != "\n":
                    out[k] = " "
            line += body.count("\n")
            i = j
        elif (c == "r" and re.match(r'r#*"', text[i:i + 8])) or (c == "b" and re.match(r'br#*"', text[i:i + 9])):
            start = text.index('"', i)
            hashes = text[i:start].count("#")
            end = text.find('"' + "#" * hashes, start + 1)
            end = n if end < 0 else end + 1 + hashes
            strings.append((i, text[start + 1:end - 1 - hashes]))
            for k in range(i, end):
                if out[k] != "\n":
                    out[k] = " "
            line += text[i:end].count("\n")
            i = end
        elif c == '"' or (c == "b" and text.startswith('b"', i)):
            start = i + (1 if c == "b" else 0)
            j = start + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            strings.append((i, text[start + 1:j]))
            for k in range(i, min(j + 1, n)):
                if out[k] != "\n":
                    out[k] = " "
            line += text[i:j].count("\n")
            i = j + 1
        elif c == "'":
            # A char literal ('a', '\n', '\u{1F600}'), not a lifetime ('a, 'static).
            m = re.match(r"'(?:\\(?:u\{[0-9a-fA-F]+\}|x[0-9a-fA-F]{2}|.)|[^\\'\n])'", text[i:i + 12])
            if m:
                for k in range(i, i + m.end()):
                    out[k] = " "
                i += m.end()
            else:
                i += 1
        else:
            i += 1
    return "".join(out), comments, strings


def line_starts(text):
    starts = [0]
    for m in re.finditer("\n", text):
        starts.append(m.end())
    return starts


def line_of(starts, offset):
    lo, hi = 0, len(starts) - 1
    while lo < hi:
        mid = (lo + hi + 1) // 2
        if starts[mid] <= offset:
            lo = mid
        else:
            hi = mid - 1
    return lo + 1


def number_token(raw):
    """A numeric literal's value as a canonical token, or None for a trivial one."""
    s = raw.replace("_", "")
    if re.match(r"-?0[xX]", s):
        s = re.sub(r"(i32|i64|u8|u16|u32|u64|usize|isize|L)$", "", s)
        try:
            v = float(int(s, 16))
        except ValueError:
            return None
        s = str(int(v))
    else:
        s = re.sub(r"(f32|f64|i32|i64|u8|u16|u32|u64|usize|isize|[fFdDL])$", "", s)
    try:
        v = float(s)
    except ValueError:
        return None
    if not math.isfinite(v) or abs(v) >= 1e9 and not v.is_integer():
        return None  # NaN, infinities, Float.MAX_VALUE
    if v.is_integer() and 2 ** 31 <= v < 2 ** 32:
        v -= 2 ** 32  # an ARGB colour: Java's int is Rust's u32 (0xFF000000 is -16777216)
        s = str(int(v))
    if not v.is_integer() and any(abs(v * k / math.pi - round(v * k / math.pi)) < 1e-5 and round(v * k / math.pi) != 0
                                  for k in (1, 2, 3, 4, 6, 8, 9, 12, 18, 36, 180)):
        return None  # a multiple of pi: Rust spells angles as PI / 4.0 or to_radians()
    if abs(v) in (57.29578, 0.01745329) or abs(v - 57.2957795) < 1e-4 or abs(v - 0.0174533) < 1e-6:
        return None
    if v.is_integer() and abs(v) < 1e15 and "." not in s and "e" not in s.lower():
        token = str(int(v))
    else:
        token = canonical_float(v)
    return None if token in TRIVIAL_NUMBERS or token.lstrip("-") in TRIVIAL_NUMBERS else "n:" + token


def canonical_float(v):
    """A float literal's value to 7 significant digits: Java's 0.2F (as float bits) and Rust's 0.2 agree."""
    v = float(f"{v:.7g}")
    return str(int(v)) + ".0" if v.is_integer() else repr(v)


MAP_DIR = ROOT / "tools" / "port_map"
# Why a vanilla method or class needs no port of its own (`skip:<reason>` in the verdict map).
SKIP_REASONS = {
    "java": "Java plumbing Rust doesn't need: codecs, builders, collections, caches, JIT-minded optimisations",
    "data": "data Horizon reads from tables or files instead of code: registries, bootstraps, datagen",
    "ui": "layout or drawing Horizon's own UI and renderer do differently, with no behaviour of its own",
    "debug": "debug and developer tooling",
    "service": "an online service Horizon doesn't offer: Realms, telemetry, chat reporting, social",
    "server": "dedicated-server and JVM tooling: RCON, JMX, the server GUI, profilers",
    "other": "see the note",
}


def read_port_map():
    """The verdict map's rows: `vanilla<TAB>verdict<TAB>where<TAB>note`, one file per batch in tools/port_map/."""
    rows = []
    if os.environ.get("PORT_LEDGER_NO_MAP"):
        return rows  # measuring the automatic matches against the map needs them built without it
    for path in sorted(MAP_DIR.glob("*.tsv")) if MAP_DIR.is_dir() else []:
        for n, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if not line.strip() or line.lstrip().startswith("#"):
                continue
            parts = [x.strip() for x in line.split("\t")] + ["", "", ""]
            rows.append({"src": f"{path.relative_to(ROOT)}:{n}", "vanilla": parts[0], "verdict": parts[1],
                         "where": parts[2], "note": parts[3]})
    return rows


CLASS_SUFFIXES = ("BlockEntity", "Entity", "Block", "Item", "Renderer", "Model", "Screen", "Goal", "Menu", "Packet",
                  "Behavior", "Sensor", "Feature", "Layer", "Manager", "Handler", "Ai", "Brain")


def class_stems(class_path):
    """The Rust file stems a class could live in: ShulkerBulletRenderer -> shulker_bullet_renderer,
    shulker_bullet, shulker_bullets."""
    simple = class_path.rsplit("/", 1)[-1].split("$")[-1]
    out = {snake_case(simple)}
    for suffix in CLASS_SUFFIXES:
        if simple.endswith(suffix) and len(simple) > len(suffix) + 2:
            out.add(snake_case(simple[:-len(suffix)]))
    return out | {o + "s" for o in out} | {o + "es" for o in out} | {o[:-1] + "ies" for o in out if o.endswith("y")}


def name_similarity(a, b):
    """Shared words over all words of two snake names: drop_off and drop_off 1.0, set_active_activity_if_possible
    and set_active_if_possible 0.8."""
    wa, wb = set(a.split("_")) - {""}, set(b.split("_")) - {""}
    return len(wa & wb) / len(wa | wb) if wa and wb else 0.0


def snake_case(name):
    """AnvilLand, ANVIL_LAND and anvil_land all as anvil_land."""
    return re.sub(r"(?<=[a-z0-9])(?=[A-Z])", "_", name).lower()


def id_tokens_from_string(s):
    """Game id tokens a Rust string literal names: "minecraft:entity.shulker_bullet.hit" ->
    entity_shulker_bullet_hit (and its last two parts, for ids whose holder drops a prefix)."""
    s = s.strip()
    if not re.fullmatch(r"(?:[a-z0-9_.-]+:)?[a-z0-9_./-]+", s) or len(s) < 4:
        return set()
    path = s.split(":", 1)[-1]
    if ":" not in s and (path.count(".") == 1 or path[0] in "._"):
        return set()  # servers.dat, os.name: a file or property name, not a game id
    token = re.sub(r"[./-]", "_", path)
    if "_" not in token and len(token) < 6:
        return set()
    return {"id:" + token}


# ---- Java ---------------------------------------------------------------------------------------------


def parse_java(path_and_root):
    path, root = path_and_root
    rel = os.path.relpath(path, root).replace(os.sep, "/")
    try:
        text = open(path, encoding="utf-8", errors="replace").read()
    except OSError:
        return []
    code, strings = blank_java(text)
    starts = line_starts(text)
    package = (re.search(r"^\s*package\s+([\w.]+)\s*;", code, re.M) or [None, ""])[1].replace(".", "/")
    imports = {m[1].rsplit(".", 1)[-1]: m[1] for m in re.finditer(r"^\s*import\s+(?:static\s+)?([\w.]+)\s*;", code, re.M)}
    classes = []  # dicts
    scopes = []  # (kind, ref, depth) where kind: class/method/block
    depth = 0
    seg_start = 0  # where the current header text begins
    method_body = {}  # id(method dict) -> [body start, body end]
    i, n = 0, len(code)
    while i < n:
        c = code[i]
        if c == "{":
            header = code[seg_start:i].strip()
            header_one = " ".join(header.split())
            parent = scopes[-1] if scopes else None
            m = JAVA_CLASS.search(header_one)
            in_method = any(s[0] == "method" for s in scopes)
            if m and "new " not in header_one.split(m.group(0))[0][-6:] and not in_method:
                outer = next((s[1] for s in reversed(scopes) if s[0] == "class"), None)
                name = m.group(2)
                cls = {
                    "name": name,
                    "path": (outer["path"] + "$" + name) if outer else (package + "/" + name if package else name),
                    "kind": m.group(1),
                    "file": rel,
                    "line": line_of(starts, i),
                    "extends": [],
                    "methods": {},
                    "imports": imports,
                    "package": package,
                    "outer": outer["path"] if outer else None,
                    "fields": {},
                }
                ext = JAVA_EXTENDS.search(header_one)
                if ext:
                    cls["extends"].append(ext.group(1).split("<")[0])
                imp = JAVA_IMPLEMENTS.search(header_one)
                if imp:
                    cls["extends"] += [p.strip().split("<")[0] for p in re.split(r",(?![^<]*>)", imp.group(1)) if p.strip()]
                if m.group(1) == "record":
                    cls["fields"].update({v: t.split(".")[-1] for t, v in JAVA_TYPED.findall(header_one.split("(", 1)[-1])})
                    for field in re.findall(r"\b([a-z]\w*)\s*(?:,|\)\s*(?:implements|$))", header_one.split("(", 1)[-1]):
                        cls["methods"].setdefault(field, {"name": field, "lines": [], "calls": [], "tokens": [], "client": False, "accessor": True})
                classes.append(cls)
                scopes.append(("class", cls, depth))
            elif parent and parent[0] == "class" and not in_method:
                mm = JAVA_METHOD.search(header_one)
                name = mm.group(1) if mm else None
                if header_one.startswith("static") and header_one.strip() == "static":
                    name = "<clinit>"
                elif name in JAVA_KEYWORDS or (mm and "=" in header_one[:mm.start()]):
                    name = None
                elif not mm and re.fullmatch(r"(?:public\s+|protected\s+|private\s+)*" + re.escape(parent[1]["name"]), header_one):
                    name = "<init>"  # A record's compact constructor.
                if name and name == parent[1]["name"]:
                    name = "<init>"
                elif name and name[0].isupper():
                    name = None  # An enum constant's body.
                if name:
                    cls = parent[1]
                    meth = cls["methods"].setdefault(name, {"name": name, "lines": [], "calls": [], "tokens": [], "client": False})
                    meth.pop("accessor", None)
                    params = header_one[mm.end(1):] if mm else ""
                    meth.setdefault("vars", {}).update({v: t.split(".")[-1] for t, v in JAVA_TYPED.findall(params)})
                    span = [line_of(starts, i), None]
                    meth["lines"].append(span)
                    scopes.append(("method", (meth, span, i), depth))
                else:
                    scopes.append(("block", None, depth))
            else:
                scopes.append(("block", None, depth))
            depth += 1
            seg_start = i + 1
        elif c == "}":
            depth -= 1
            if scopes:
                kind, ref, _ = scopes.pop()
                if kind == "method":
                    meth, span, body_start = ref
                    span[1] = line_of(starts, i)
                    body = code[body_start:i]
                    raw = text[body_start:i]
                    for m in JAVA_CALL.finditer(body):
                        recv, dot, name = m.group(1), m.group(2), m.group(3)
                        if name in JAVA_KEYWORDS:
                            continue
                        before = body[max(0, m.start() - 4):m.start()]
                        if before.endswith("new "):
                            continue
                        meth["calls"].append([recv or (dot and "?") or "", name])
                    for m in JAVA_METHOD_REF.finditer(body):
                        meth["calls"].append([m.group(1), m.group(2)])
                    for m in JAVA_NEW.finditer(body):
                        meth["calls"].append([m.group(1).split(".")[-1], "<init>"])
                    tokens = set()
                    for m in JAVA_ID.finditer(body):
                        tokens.add("id:" + m.group(2).lower())
                    for m in NUMBER.finditer(body):
                        t = number_token(m.group(0))
                        if t:
                            tokens.add(t)
                    for offset, s in strings:
                        if body_start <= offset < i:
                            tokens |= id_tokens_from_string(s)
                    meth["tokens"] = sorted(set(meth["tokens"]) | tokens)
                    meth.setdefault("vars", {}).update({v: t.split(".")[-1] for t, v in JAVA_TYPED.findall(body)})
                    for t, v in re.findall(r"\binstanceof\s+([A-Z][\w.]*)(?:<[^>]*>)?\s+([a-z]\w*)", body):
                        meth["vars"][v] = t.split(".")[-1]
                    if CLIENT_EFFECTS.search(raw):
                        meth["client"] = True
            seg_start = i + 1
        elif c == ";":
            # An abstract or interface method: a declaration with no body.
            if scopes and scopes[-1][0] == "class":
                header_one = " ".join(code[seg_start:i].split())
                fm = re.search(r"\b([A-Z][\w.]*)(?:<.*>)?(?:\[\])*\s+([a-z]\w*)\s*(?:=.*)?$", header_one)
                if fm and "(" not in header_one.split("=")[0]:
                    scopes[-1][1]["fields"][fm.group(2)] = fm.group(1).split(".")[-1]
                mm = JAVA_METHOD.search(header_one)
                if mm and "=" not in header_one[:mm.start()] and mm.group(1) not in JAVA_KEYWORDS:
                    cls = scopes[-1][1]
                    cls["methods"].setdefault(mm.group(1), {"name": mm.group(1), "lines": [], "calls": [], "tokens": [], "client": False, "abstract": True})
            seg_start = i + 1
        i += 1
    for cls in classes:
        for meth in cls["methods"].values():
            meth["calls"] = sorted({tuple(x) for x in meth["calls"]})
    return classes


def java_files(roots):
    seen = {}
    for root in roots:
        for dirpath, _, files in os.walk(root):
            for f in files:
                if f.endswith(".java"):
                    full = os.path.join(dirpath, f)
                    rel = os.path.relpath(full, root).replace(os.sep, "/")
                    seen.setdefault(rel, (full, root))  # client and server share code: first wins
    return sorted(seen.values())


# ---- Bytecode -----------------------------------------------------------------------------------------
# The shipped jar is vanilla exactly (the decompiled sources may carry a loader's patches) and its calls
# name their owner class and descriptor, so when a jar is at hand the call graph, hierarchy and literals
# come from it; the sources only give file and line numbers.

INVOKES = {"182", "183", "184", "185"}
CLIENT_CALLS = {"addParticle", "addAlwaysVisibleParticle", "playLocalSound", "itemUsed", "isClientSide"}


def jar_entries(jar):
    """(class name, bytes) of every game class in a jar, the server bundler's inner jar included."""
    import io
    import zipfile

    def walk(archive):
        for name in archive.namelist():
            if name.endswith(".class") and name.startswith(("net/minecraft/", "com/mojang/")):
                yield name[:-6], archive.read(name)
            elif name.startswith("META-INF/versions/") and name.endswith(".jar"):
                with zipfile.ZipFile(io.BytesIO(archive.read(name))) as inner:
                    yield from walk(inner)

    with zipfile.ZipFile(jar) as archive:
        yield from walk(archive)


def parse_class_bytes(item):
    name, data = item
    try:
        import class_index
    except ImportError:
        from . import class_index
    try:
        cf = class_index.ClassFile(data)
    except Exception:
        return None
    acc = cf.access
    kind = ("@interface" if acc & 0x2000 else "interface" if acc & 0x200 else "enum" if acc & 0x4000
            else "record" if cf.super == "java/lang/Record" else "class")
    top = name.split("$", 1)[0]
    cls = {
        "name": name.rsplit("/", 1)[-1].split("$")[-1], "path": name, "kind": kind,
        "file": top + ".java", "line": None,
        "extends": [x for x in [cf.super] + cf.interfaces if x and not x.startswith("java/")],
        "methods": {}, "imports": {}, "package": name.rsplit("/", 1)[0],
        "outer": name.rsplit("$", 1)[0] if "$" in name else None, "fields": {}, "bytecode": True,
    }
    if any(p[:1].isdigit() for p in name.rsplit("/", 1)[-1].split("$")[1:]):
        cls["anonymous"] = True
    for access, mname, desc, code in cf.methods:
        owner = class_index.lambda_owner(mname)
        target = owner[len("lambda$"):] if owner.startswith("lambda$") else mname
        target = {"static": "<clinit>", "new": "<init>"}.get(target, target)  # lambdas in initialisers
        if access & 0x40 and not owner.startswith("lambda$"):  # bridge
            continue
        if access & 0x1000 and not owner.startswith("lambda$") or mname in ("$values", "values", "valueOf"):
            continue  # synthetic, and an enum's generated members
        m = cls["methods"].setdefault(target, {"name": target, "lines": [], "calls": set(), "tokens": set(),
                                               "client": False, "size": 0, "touches": set()})
        if code is None:
            m["abstract"] = not m["size"]
            continue
        m.pop("abstract", None)
        try:
            toks = cf.code_tokens(code)
        except Exception:
            continue
        m["size"] += len(toks) // 3
        body = toks[1:]
        if not owner.startswith("lambda$") and len(body) <= 3 and mname != "<init>":
            # A getter, setter or constant: `return this.x;` is nothing to port on its own.
            m["accessor"] = True
        elif not owner.startswith("lambda$"):
            m["accessor"] = False
        for t in toks:
            op, _, rest = t.partition(" ")
            if op in INVOKES and rest.startswith("m:"):
                ref = rest[2:]
                owner_cls, _, member = ref.partition(".")
                meth = member.split("(", 1)[0]
                if owner_cls.startswith(("net/minecraft/", "com/mojang/")):
                    m["calls"].add((owner_cls, meth))
                if meth in CLIENT_CALLS:
                    m["client"] = True
            elif op == "186" and rest.startswith("y:"):
                # invokedynamic: a method reference (Foo::bar) or lambda handle among the bootstrap arguments.
                for h in re.findall(r"h\d+:((?:net|com)/[\w/$]+)\.([\w$]+)\(", rest):
                    target_name = h[1]
                    if target_name.startswith("lambda$"):
                        continue
                    m["calls"].add((h[0], target_name))
            elif rest.startswith("m:") and (op in ("180", "181") or op in ("178", "179") and rest[2:].startswith(name + ".")):
                # The fields a method reads and writes: its port touches the same state, by its Rust name.
                fm = re.match(r"([a-z_$][\w$]*?)(?:L[\w/$]+;|\[.*|[BCDFIJSZ])$", rest[2:].partition(".")[2])
                if fm and not fm.group(1).startswith("$"):
                    m["touches"].add(fm.group(1))
                if op == "178":
                    owner_cls, _, member = rest[2:].partition(".")
                    holder = owner_cls.rsplit("/", 1)[-1].split("$")[0]
                    field = re.match(r"([A-Z][A-Z0-9_]*?)(?:L[a-z][\w/$]*;|\[.*|[BCDFIJSZ])$", member)
                    if holder in ID_HOLDERS and field and field.group(1) not in ("LOGGER", "CODEC", "STREAM_CODEC"):
                        m["tokens"].add("id:" + field.group(1).lower())
            elif op == "178" and rest.startswith("m:"):
                owner_cls, _, member = rest[2:].partition(".")
                holder = owner_cls.rsplit("/", 1)[-1].split("$")[0]
                field = re.match(r"([A-Z][A-Z0-9_]*?)(?:L[a-z][\w/$]*;|\[.*|[BCDFIJSZ])$", member)
                if holder in ID_HOLDERS and field and field.group(1) not in ("LOGGER", "CODEC", "STREAM_CODEC"):
                    m["tokens"].add("id:" + field.group(1).lower())
            elif op in ("18", "19", "20"):
                if rest.startswith("s:"):
                    m["tokens"] |= id_tokens_from_string(rest[2:])
                elif rest.startswith("f:"):
                    import struct as _s
                    v = _s.unpack(">f", bytes.fromhex(rest[2:]))[0]
                    t2 = number_token(canonical_float(v))
                    if t2:
                        m["tokens"].add(t2)
                elif rest.startswith("d:"):
                    import struct as _s
                    v = _s.unpack(">d", bytes.fromhex(rest[2:]))[0]
                    t2 = number_token(canonical_float(v))
                    if t2:
                        m["tokens"].add(t2)
                elif rest.startswith(("i:", "l:")):
                    t2 = number_token(rest[2:])
                    if t2:
                        m["tokens"].add(t2)
            elif op == "17":  # sipush
                t2 = number_token(str(int.from_bytes(bytes.fromhex(rest), "big", signed=True)))
                if t2:
                    m["tokens"].add(t2)
    for m in cls["methods"].values():
        m["calls"] = sorted(m["calls"])
        m["tokens"] = sorted(m["tokens"])
        m["touches"] = sorted(m["touches"])
    return cls


def default_jars():
    lock = json.loads((ROOT / "research-lock.json").read_text()) if (ROOT / "research-lock.json").exists() else {}
    version = str((lock.get("minecraft") or {}).get("version") or lock.get("version") or "26.3")
    home = Path.home()
    candidates = sorted((ROOT / ".local" / "minecraft" / version).glob("*.jar")) + [
        home / "Library/Application Support/minecraft/versions" / version / f"{version}.jar",
        home / ".minecraft/versions" / version / f"{version}.jar",
        Path(os.environ.get("APPDATA", "")) / ".minecraft/versions" / version / f"{version}.jar",
    ]
    return [str(c) for c in candidates if c.is_file()][:2]


# ---- Rust ---------------------------------------------------------------------------------------------


def parse_rust(path):
    text = (ROOT / path).read_text(encoding="utf-8", errors="replace")
    code, comments, strings = blank_rust(text)
    starts = line_starts(text)
    lines_raw = text.split("\n")
    fns = []
    structs = {}
    struct_lines = {}
    trait_impls = []
    scopes = []  # (kind, ref)
    depth = 0
    seg_start = 0
    paren = 0
    i, n = 0, len(code)
    while i < n:
        c = code[i]
        if c == "(" or c == "[":
            paren += 1
        elif c == ")" or c == "]":
            paren = max(0, paren - 1)
        elif c == "{":
            header = " ".join(code[seg_start:i].split())
            m = RUST_FN.search(header)
            impl_type = next((s[1] for s in reversed(scopes) if s[0] in ("impl", "trait")), None)
            if m and "=" not in header[:m.start()]:
                line = line_of(starts, seg_start + (code[seg_start:i].find("fn") if "fn" in code[seg_start:i] else 0))
                owner = next((s[1] for s in reversed(scopes) if s[0] == "fn"), None)
                in_tests = any(s[0] == "mod" and s[1] in ("tests", "test") for s in scopes) or "#[test]" in header
                params = {}
                sig = header[m.end():]
                generics = dict(re.findall(r"\b([A-Z]\w*)\s*:\s*(?:\?Sized\s*\+\s*)?([A-Z]\w*)", sig.split("(", 1)[0] + " " + (sig.split("where", 1)[1] if "where" in sig else "")))
                for pname, ptype in RUST_PARAM.findall(sig.split("->", 1)[0]):
                    t = core_type(ptype)
                    params[pname] = generics.get(t, t)
                fn = {
                    "params": params,
                    "name": m.group(1),
                    "owner": impl_type,
                    "file": path,
                    "line": line,
                    "end": None,
                    "parent": owner["name"] if owner else None,
                    "body": i,
                    "test": bool(in_tests or TEST_FILE.search(path)),
                }
                fns.append(fn)
                scopes.append(("fn", fn))
            elif RUST_IMPL.match(header) and "fn " not in header:
                scopes.append(("impl", RUST_IMPL.match(header).group(1).split("::")[-1]))
                pair = RUST_IMPL_FOR.match(header)
                if pair:
                    trait_impls.append((pair.group(1).split("::")[-1], pair.group(2).split("::")[-1]))
            elif RUST_STRUCT.search(header) and "fn " not in header and not any(s[0] == "fn" for s in scopes):
                scopes.append(("struct", (RUST_STRUCT.search(header).group(1), i)))
            elif RUST_TRAIT.match(header):
                scopes.append(("trait", RUST_TRAIT.match(header).group(1)))
            elif RUST_MOD.match(header):
                scopes.append(("mod", RUST_MOD.match(header).group(1)))
            else:
                scopes.append(("block", None))
            depth += 1
            seg_start = i + 1
        elif c == "}":
            depth -= 1
            if scopes:
                kind, ref = scopes.pop()
                if kind == "fn":
                    ref["end"] = line_of(starts, i)
                    ref["body_end"] = i
                elif kind == "struct":
                    name, at = ref
                    struct_lines.setdefault(name, line_of(starts, at))
                    fields = structs.setdefault(name, {})
                    depth_here = 0
                    for raw_line in code[at + 1:i].split("\n"):
                        if depth_here == 0:
                            fm = RUST_FIELD.match(raw_line)
                            if fm:
                                t = core_type(fm.group(2))
                                if t:
                                    fields[fm.group(1)] = t
                        depth_here += raw_line.count("{") + raw_line.count("<") - raw_line.count("}") - raw_line.count(">")
                        depth_here = max(0, depth_here)
            seg_start = i + 1
        elif c == ";" and paren == 0:
            header = " ".join(code[seg_start:i].split())
            m = RUST_FN.search(header)
            if m and scopes and scopes[-1][0] == "trait" and "=" not in header[:m.start()]:
                line = line_of(starts, seg_start + code[seg_start:i].find("fn"))
                fns.append({"name": m.group(1), "owner": scopes[-1][1], "file": path, "line": line, "end": line,
                            "parent": None, "body": i, "body_end": i, "decl": True, "test": bool(TEST_FILE.search(path))})
            seg_start = i + 1
        i += 1
    # Each fn's own code (nested fns excluded) for calls and literals.
    fns.sort(key=lambda f: (f["line"], -(f["end"] or 0)))
    for fn in fns:
        body = list(code[fn["body"]:fn.get("body_end", fn["body"])])
        for other in fns:
            if other is fn or other["body"] <= fn["body"] or other.get("body_end", 0) > fn.get("body_end", 0):
                continue
            for k in range(other["body"] - fn["body"], other.get("body_end", other["body"]) - fn["body"]):
                if 0 <= k < len(body):
                    body[k] = " "
        body = "".join(body)
        calls = set()
        for m in RUST_CALL.finditer(body):
            name = m.group(1)
            if name in RUST_KEYWORDS:
                continue
            if body[m.start(1) + len(name):m.start(1) + len(name) + 1] == "!":
                continue
            calls.add(name)
        # Typed calls: the receiver's type through self, params, lets and struct fields (resolved in build,
        # once every file's structs are known): [receiver path, method] with the path's root and fields.
        lets = {v: (core_type(t) if t else ty) for v, t, ty in RUST_LET.findall(body)}
        typed = []
        for m in RUST_METHOD_CALL.finditer(body):
            sep, name = m.group(1), m.group(2)
            before = body[:m.start()].rstrip()
            if sep == "::":
                tm = re.search(r"([A-Za-z_]\w*)\s*(?:<[^<>]*>)?$", before)
                if tm and (tm.group(1)[0].isupper()):
                    typed.append([[tm.group(1)], name])
                continue
            rm = re.search(r"([A-Za-z_]\w*(?:\s*\.\s*[a-z_]\w*)*)$", before)
            if not rm:
                continue
            start = rm.start()
            if start > 0 and before[start - 1] in ".:)]":
                continue
            chain = [p.strip() for p in rm.group(1).split(".")]
            typed.append([chain, name])
        fn["typed"] = typed
        fn["const_refs"] = sorted(set(re.findall(r"\b([A-Z][A-Z0-9_]{2,})\b", body)))
        fn["locals"] = {k: v for k, v in lets.items() if v}
        # Method references passed as values: Game::swing_arm, Self::tick, |g| g.foo
        for m in re.finditer(r"(?:\bSelf|\b[A-Z]\w*)::([a-z_]\w*)\b(?!\s*\()", body):
            calls.add(m.group(1))
        tokens = set()
        for m in NUMBER.finditer(body):
            t = number_token(m.group(0))
            if t:
                tokens.add(t)
        for offset, s in strings:
            if fn["body"] <= offset < fn.get("body_end", 0):
                tokens |= id_tokens_from_string(s)
        fn["calls"] = sorted(calls)
        fn["tokens"] = sorted(tokens)
        # Every name the body spells, snake-cased (Sound::AnvilLand -> anvil_land, WALK_TARGET -> walk_target):
        # what drift checks a vanilla id against, since Rust names most ids as enum variants, not strings.
        fn["words"] = sorted({snake_case(w) for w in re.findall(r"\b[A-Za-z][A-Za-z0-9_]{2,}\b", body)} - RUST_KEYWORDS)
        # The state it reads and writes (self.jump_delay, mob.target): what a structural match compares.
        fn["fields"] = sorted(set(re.findall(r"\.([a-z_][a-z0-9_]{2,})\b(?!\s*(?:\(|::|!))", body)))
    # Comments: a fn's doc and attribute block above its `fn` line, and the comments inside it (not inside
    # a nested fn).
    cites_by_fn = defaultdict(list)
    spans = sorted(((f["line"], f["end"] or f["line"], idx) for idx, f in enumerate(fns)), key=lambda s: (s[0], -s[1]))
    def innermost(line):
        best = None
        for a, b, idx in spans:
            if a <= line <= b and (best is None or (a >= fns[best]["line"])):
                best = idx
        return best
    doc_owner = {}
    for idx, fn in enumerate(fns):
        k = fn["line"] - 1  # 1-based line above
        while k >= 1:
            s = lines_raw[k - 1].strip()
            if s.startswith("///") or s.startswith("//") or s.startswith("#[") or s.startswith("#!["):
                doc_owner[k] = idx
                k -= 1
            else:
                break
    file_comments = []
    for line, text_line in comments:
        idx = doc_owner.get(line)
        if idx is None:
            idx = innermost(line)
        if idx is None:
            file_comments.append((line, text_line))
        else:
            cites_by_fn[idx].append((line, text_line))
    for idx, fn in enumerate(fns):
        fn["comments"] = cites_by_fn.get(idx, [])
        fn.pop("body", None)
        fn.pop("body_end", None)
    consts = {}
    for m in re.finditer(r"\b(?:const|static)\s+([A-Z][A-Z0-9_]*)\s*:\s*[^=;]+=\s*([^;]+);", code):
        toks = set()
        for n in NUMBER.finditer(m.group(2)):
            t = number_token(n.group(0))
            if t:
                toks.add(t)
        for off, st in strings:
            if m.start(2) <= off < m.end(2):
                toks |= id_tokens_from_string(st)
        if toks:
            consts[m.group(1)] = sorted(toks)
    return {"file": path, "fns": fns, "file_comments": file_comments, "structs": structs, "trait_impls": trait_impls,
            "consts": consts, "struct_lines": struct_lines}


def rust_files():
    out = subprocess.run(["git", "ls-files", "crates/*.rs", "launcher/*.rs"], cwd=ROOT, capture_output=True, text=True).stdout
    return [f for f in out.split() if f.endswith(".rs")]


# ---- Build --------------------------------------------------------------------------------------------


def default_sources():
    lock = json.loads((ROOT / "research-lock.json").read_text()) if (ROOT / "research-lock.json").exists() else {}
    version = (lock.get("minecraft") or {}).get("version") or lock.get("version") or "26.3"
    base = ROOT / ".local" / "minecraft" / str(version) / "sources"
    return [str(base / side) for side in ("client", "server") if (base / side).is_dir()]


class Vanilla:
    """The decompiled classes: simple-name lookup (with `Outer.Inner`), supertypes, methods."""

    def __init__(self, classes):
        self.classes = {c["path"]: c for c in classes}
        self.by_simple = defaultdict(set)
        for c in classes:
            if c.get("anonymous"):
                continue
            parts = c["path"].rsplit("/", 1)[-1].split("$")
            self.by_simple[parts[-1]].add(c["path"])
            if len(parts) > 1:
                self.by_simple[".".join(parts[-2:])].add(c["path"])
                self.by_simple[".".join(parts)].add(c["path"])

    def resolve(self, simple, near=None):
        """The class a (dotted) simple name means: unique; else the one `near`'s imports, package or outer
        classes give; else the unique top-level one under net/minecraft."""
        cands = self.by_simple.get(simple, set())
        if len(cands) == 1:
            return next(iter(cands))
        if not cands:
            return None
        if near:
            cls = self.classes.get(near)
            if cls:
                imported = cls["imports"].get(simple.split(".")[0])
                if imported:
                    want = imported.replace(".", "/")
                    rest = simple.split(".")[1:]
                    for cand in cands:
                        if cand == want + ("$" + "$".join(rest) if rest else "") or cand.replace("$", "/") == want:
                            return cand
                same = [c for c in cands if c.startswith(cls["package"] + "/") and "/" not in c[len(cls["package"]) + 1:]]
                if len(same) == 1:
                    return same[0]
                inner = [c for c in cands if c.startswith(near + "$") or (cls["outer"] and c.startswith(cls["outer"] + "$"))]
                if len(inner) == 1:
                    return inner[0]
        top = [c for c in cands if "$" not in c and c.startswith("net/minecraft/")]
        return top[0] if len(top) == 1 else None

    def lineage(self, path, limit=12):
        """The class and its supertypes, nearest first."""
        out, todo, seen = [], [path], set()
        while todo and len(out) < limit:
            p = todo.pop(0)
            if p in seen or p not in self.classes:
                continue
            seen.add(p)
            out.append(p)
            for sup in self.classes[p]["extends"]:
                if sup in self.classes:
                    todo.append(sup)
                    continue
                r = self.resolve(sup.split(".")[-1] if "." not in sup or sup.startswith(("net.", "com.")) else sup, near=p)
                if not r and "." in sup:
                    r = self.resolve(sup.rsplit(".", 1)[-1], near=p)
                if r:
                    todo.append(r)
        return out

    def find_method(self, path, name):
        """(declaring class, method) for `name` on the class or the nearest supertype that has it."""
        for p in self.lineage(path):
            m = self.classes[p]["methods"].get(name)
            if m:
                return p, m
        return None, None


def key(cls_path, method):
    return f"{cls_path}#{method}"


def short(cls_path, method=None):
    simple = cls_path.rsplit("/", 1)[-1].replace("$", ".")
    return f"{simple}.{method}" if method else simple


REGISTRIES = ("net/minecraft/world/entity/EntityTypes.java", "net/minecraft/world/level/block/Blocks.java",
              "net/minecraft/world/item/Items.java")
REGISTERED = re.compile(r"public static final \w+(?:<(\w+)>)? (\w+) = register\w*\((.*?)\);", re.S)


def registry_aliases(sources, van):
    """The names players use, from the registries: mooshroom -> MushroomCow, redstone_ore -> RedStoneOreBlock.
    A plain Block or Item has no class of its own to point at."""
    out = {}
    for src in sources:
        for rel in REGISTRIES:
            path = Path(src) / rel
            if not path.is_file():
                continue
            near = rel[:-5]
            for m in REGISTERED.finditer(path.read_text(encoding="utf-8", errors="replace")):
                simple = m.group(1)
                if not simple or simple in ("Block", "Item", "BlockItem", "Entity"):
                    ctor = re.search(r"\b([A-Z]\w*)::new", m.group(3))
                    simple = ctor.group(1) if ctor else None
                if not simple or simple in ("Block", "Item", "BlockItem"):
                    continue
                cp = van.resolve(simple, near=near)
                if cp:
                    out.setdefault(m.group(2).lower(), cp)
    return out


ORACLE_DIR = ROOT / "mods" / "horizon" / "src" / "test" / "java" / "dev" / "horizonlab" / "parity"


def harness_coverage(van, methods, fns, edges, known):
    """The parity harnesses that cover each ported method. A harness (tools/test_*_parity.py) runs a vanilla oracle
    (mods/horizon/src/test/java/.../parity/*.java) and a Rust fixture (crates/*/examples/**/*_fixture.rs) on the same
    inputs. It covers a method when its fixture reaches the method's port and the vanilla side is in reach too: the
    oracle calls into the method, or the harness or oracle names the method's class (oracles often reach vanilla
    through an interface, a block state for the block, which a call graph can't follow)."""
    oracles = {p.stem: p for p in ORACLE_DIR.glob("*.java")} if ORACLE_DIR.is_dir() else {}
    fixture_fns = defaultdict(list)  # the probes' example files: *_fixture.rs, *_probe.rs and the like
    for fn in fns:
        if "/examples/" in fn["file"]:
            fixture_fns[fn["file"].rsplit("/", 1)[-1][:-3]].append(fn["id"])

    def java_reach(names, depth=3):
        keys = set()
        for name in names:
            text = oracles[name].read_text(encoding="utf-8", errors="replace")
            imported = {m.group(2): m.group(1).replace(".", "/") + "/" + m.group(2)
                        for m in re.finditer(r"^import\s+((?:net|com)\.minecraft[\w.]*|com\.mojang[\w.]*)\.([A-Z]\w*);", text, re.M)}
            called = set(re.findall(r"\.(\w+)\s*\(", text)) | {m for _, m in re.findall(r"\b([A-Z]\w*)::(\w+)", text)}
            for cp in imported.values():
                if cp not in van.classes:
                    continue
                for meth in called:
                    decl, _ = van.find_method(cp, meth)
                    if decl:
                        keys.add(key(decl, meth))
        frontier = set(keys)
        for _ in range(depth):
            frontier = {c for k in frontier for c in methods.get(k, {}).get("callees", ())} - keys
            keys |= frontier
        return keys

    def rust_reach(starts, depth=5):
        seen = set(starts)
        frontier = set(starts)
        for _ in range(depth):
            frontier = {c for f in frontier for c in edges.get(f, ())} - seen
            seen |= frontier
        return seen

    covered = defaultdict(list)
    for path in sorted((ROOT / "tools").glob("test_*_parity.py")):
        text = path.read_text(encoding="utf-8", errors="replace")
        tokens = set(re.findall(r"\w+", text))
        names = tokens & oracles.keys()
        fixtures = {n for n in tokens & fixture_fns.keys() if n.endswith(("_fixture", "_probe"))}
        if not names or not fixtures:
            continue
        vkeys = java_reach(names)
        rfns = rust_reach([f for n in fixtures for f in fixture_fns[n]])
        words = set(re.findall(r"\b[A-Z]\w+", text + "".join(oracles[n].read_text(encoding="utf-8", errors="replace") for n in names)))
        harness_name = path.stem.removeprefix("test_").removesuffix("_parity")
        for k, fids in known.items():
            if not fids & rfns:
                continue
            simple = methods[k]["class"].rsplit("/", 1)[-1] if k in methods else ""
            if k in vkeys or simple.split("$")[0] in words:
                covered[k].append(harness_name)
    return dict(covered)


def build(sources, jars, jobs):
    if not sources and not jars:
        sys.exit("No vanilla code: run `python3 tools/lab.py decompile`, or pass --sources DIR / --jar JAR.")
    src_classes = []
    with ProcessPoolExecutor(max_workers=jobs) as pool:
        if sources:
            for chunk in pool.map(parse_java, java_files(sources), chunksize=64):
                src_classes.extend(chunk)
        bc_classes = []
        seen_bc = set()
        for jar in jars:
            entries = [e for e in jar_entries(jar) if e[0] not in seen_bc]
            seen_bc.update(e[0] for e in entries)
            bc_classes += [c for c in pool.map(parse_class_bytes, entries, chunksize=64) if c]
        rust = list(pool.map(parse_rust, rust_files(), chunksize=8))
    if bc_classes:
        # The jar decides what exists and what calls what; the sources add files and lines.
        src_by_path = {c["path"]: c for c in src_classes}
        for c in bc_classes:
            src = src_by_path.get(c["path"])
            if not src:
                continue
            c["file"], c["line"], c["fields"], c["imports"] = src["file"], src["line"], src["fields"], src["imports"]
            for name, m in c["methods"].items():
                sm = src["methods"].get(name)
                if sm:
                    m["lines"] = sm["lines"]
                    m["accessor"] = m.get("accessor", False) or (sm.get("accessor", False) and not m.get("calls"))
        classes = bc_classes
    else:
        classes = src_classes
    van = Vanilla(classes)
    aliases = registry_aliases(sources, van)

    # Vanilla methods: one entry per class and name (overloads merged), with resolved callees.
    methods = {}
    for c in classes:
        for name, m in c["methods"].items():
            methods[key(c["path"], name)] = {
                "class": c["path"], "name": name, "file": c["file"], "lines": m["lines"],
                "calls_raw": m["calls"], "tokens": m["tokens"], "client": m["client"],
                "abstract": m.get("abstract", False), "accessor": m.get("accessor", False),
                "vars": m.get("vars", {}),
                "size": m.get("size") or sum((b or a) - a + 1 for a, b in m["lines"]),
                "bytecode": bool(c.get("bytecode")), "anonymous": bool(c.get("anonymous")),
                "touches": [snake_case(f) for f in m.get("touches", ())],
            }
    defined = defaultdict(set)
    for k, m in methods.items():
        defined[m["name"]].add(k)
    # A field name every class declares with the same type (player: LocalPlayer in the client classes
    # that hold one) resolves through other classes too (this.minecraft.player.itemUsed).
    seen_types = defaultdict(set)
    for c in classes:
        for f, t in c["fields"].items():
            seen_types[f].add(t)
    field_types = {f: next(iter(t)) for f, t in seen_types.items() if len(t) == 1}
    for k, m in methods.items():
        callees = set()
        for recv, name in m["calls_raw"]:
            target = None
            if m["bytecode"]:
                # The call names its owner exactly; the method may be declared on a supertype.
                decl, _ = van.find_method(recv, name) if recv in van.classes else (None, None)
                if decl and key(decl, name) != k:
                    callees.add(key(decl, name))
                continue
            if recv in ("", "this", "super"):
                decl, _ = van.find_method(m["class"], name)
                if not decl:
                    cls = van.classes[m["class"]]
                    if cls["outer"]:
                        decl, _ = van.find_method(cls["outer"], name)
                target = key(decl, name) if decl else None
            elif recv[:1].isupper() and not recv.isupper():
                cp = van.resolve(recv, near=m["class"])
                if cp:
                    decl, _ = van.find_method(cp, name)
                    target = key(decl, name) if decl else None
            elif recv[:1].islower():
                # A field, parameter or local whose declared type is known.
                typ = m["vars"].get(recv)
                if not typ:
                    for p in van.lineage(m["class"]):
                        typ = van.classes[p]["fields"].get(recv)
                        if typ:
                            break
                if not typ:
                    typ = field_types.get(recv)
                cp = van.resolve(typ, near=m["class"]) if typ else None
                if cp:
                    decl, _ = van.find_method(cp, name)
                    target = key(decl, name) if decl else None
            if not target and name not in GENERIC_CALLS and len(defined.get(name, ())) == 1:
                target = next(iter(defined[name]))
            if target and target != k:
                callees.add(target)
        m["callees"] = sorted(callees)
        del m["calls_raw"]
        del m["vars"]
    callers = defaultdict(set)
    for k, m in methods.items():
        for c in m["callees"]:
            callers[c].add(k)
    # Overrides: a call to Entity.getKnownMovement also runs ServerPlayer.getKnownMovement.
    subclasses = defaultdict(set)
    for p in van.classes:
        for sup in van.lineage(p)[1:]:
            subclasses[sup].add(p)
    def overrides(k):
        m = methods[k]
        return [key(sub, m["name"]) for sub in subclasses.get(m["class"], ()) if key(sub, m["name"]) in methods]

    # Rust functions and their citations.
    fns = []
    stale = []
    class_cites = defaultdict(list)
    file_class_cites = defaultdict(set)
    for f in rust:
        for fn in f["fns"]:
            fn["id"] = len(fns)
            fn["cites"] = []
            fn["primary"] = []
            seen = set()
            doc = [(ln, t) for ln, t in fn["comments"] if ln < fn["line"] and t.strip().startswith("///")]
            lead = doc[0] if doc else None
            for line, text in fn["comments"]:
                is_lead = lead is not None and line == lead[0] and re.match(r"\s*///\s*`?[A-Z]", text)
                for m in CITE.finditer(text):
                    cls_name, member = m.group(1), m.group(2)
                    if member and member[:1].isupper():
                        # Outer.Inner (a class) or a constant.
                        if van.resolve(f"{cls_name}.{member}"):
                            cls_name, member = f"{cls_name}.{member}", None
                        else:
                            continue
                    cp = van.resolve(cls_name)
                    if not cp:
                        if member and sum(ch.isupper() for ch in cls_name) >= 1 and len(cls_name) > 3 and cls_name not in ("Self", "Some", "None", "Ok", "Err", "Vec", "Option", "Result", "String", "Box", "Arc", "Rc", "HashMap", "HashSet", "BTreeMap"):
                            stale.append({"what": "class", "cite": f"{cls_name}.{member}", "file": f["file"], "line": line})
                        continue
                    if not member:
                        if sum(ch.isupper() for ch in cls_name.split(".")[-1]) >= 2:
                            class_cites[cp].append(fn["id"])
                        continue
                    decl, meth = van.find_method(cp, member)
                    if not decl:
                        cls = van.classes[cp]
                        fields = re.search(r"\b" + re.escape(member) + r"\b", open(os.path.join(next(s for s in sources if os.path.exists(os.path.join(s, cls["file"]))), cls["file"]), errors="replace").read()) if member[0].islower() else None
                        if not fields:
                            stale.append({"what": "method", "cite": f"{short(cp)}.{member}", "file": f["file"], "line": line})
                        continue
                    k = key(decl, member)
                    if is_lead and k not in fn["primary"]:
                        fn["primary"].append(k)
                    if k not in seen:
                        seen.add(k)
                        fn["cites"].append(k)
            del fn["comments"]
            fns.append(fn)
        for line, text in f["file_comments"]:
            for m in CITE.finditer(text):
                if not m.group(2) and sum(ch.isupper() for ch in m.group(1).split(".")[-1]) >= 2:
                    cp = van.resolve(m.group(1))
                    if cp:
                        file_class_cites[cp].add(f["file"])
    ported = defaultdict(list)  # vanilla key -> fns whose doc opens with it
    mentioned = defaultdict(list)  # vanilla key -> fns that name it elsewhere
    for fn in fns:
        if fn.get("test"):
            continue
        for k in fn["primary"]:
            ported[k].append(fn["id"])
        for k in fn["cites"]:
            if k not in fn["primary"]:
                mentioned[k].append(fn["id"])

    # Constants a fn uses by name count as its literals (`const SPEED: f32 = 0.458;`).
    file_consts = {f["file"]: f["consts"] for f in rust}
    global_consts = defaultdict(list)
    for f in rust:
        for cname, toks in f["consts"].items():
            global_consts[cname].append(toks)
    for fn in fns:
        extra = set()
        for cname in fn.get("const_refs", ()):
            toks = file_consts.get(fn["file"], {}).get(cname)
            if toks is None and len(global_consts.get(cname, ())) == 1:
                toks = global_consts[cname][0]
            if toks:
                extra.update(toks)
        if extra:
            fn["tokens"] = sorted(set(fn["tokens"]) | extra)
        fn.pop("const_refs", None)

    # Named matches: a fn with the method's own name (tickRain -> tick_rain) in a file that cites the class.
    files_citing = defaultdict(set)
    for fn in fns:
        for k in fn["cites"]:
            files_citing[methods[k]["class"]].add(fn["file"]) if k in methods else None
    for cp, ids in class_cites.items():
        for fid in ids:
            files_citing[cp].add(fns[fid]["file"])
    for cp, files_ in file_class_cites.items():
        files_citing[cp] |= files_
    by_name_live = defaultdict(list)
    for fn in fns:
        if not fn.get("test"):
            by_name_live[fn["name"]].append(fn["id"])
    by_simple = defaultdict(set)
    for cp in van.classes:
        by_simple[cp.rsplit("/", 1)[-1].split("$")[-1]].add(cp)
    simple_names = sorted(n.lower() for n in by_simple)
    lineage_cache = {}

    def lineage_names(cp):
        if cp not in lineage_cache:
            lineage_cache[cp] = {p.rsplit("/", 1)[-1].split("$")[-1] for p in van.lineage(cp)}
        return lineage_cache[cp]

    def resembles_a_class(name):
        """HurtByTarget is HurtByTargetGoal's port; Game and MotionWorld are Horizon's own containers."""
        n = name.lower()
        i = bisect.bisect_left(simple_names, n)
        return i < len(simple_names) and simple_names[i].startswith(n)

    def owner_fits(fid, class_path):
        """Whether a fn's impl type lets it port a method of `class_path`: `impl HurtByTarget` doesn't port
        AvoidEntityGoal.canUse however alike the names; `impl Mob` may port LivingEntity's or Zombie's; a fn
        on one of Horizon's own containers (Game, Sim, MotionWorld) may port anything."""
        owner = fns[fid].get("owner")
        if not owner:
            return True
        cls = CLASS_AFFIXES.sub("", class_path.rsplit("/", 1)[-1].split("$")[-1]).lower()
        own = CLASS_AFFIXES.sub("", owner).lower()
        if len(own) >= 4 and len(cls) >= 4 and (cls.startswith(own) or own.startswith(cls)):
            return True
        if owner in by_simple:
            return owner in lineage_names(class_path) or any(class_path in van.lineage(o) for o in by_simple[owner])
        return not resembles_a_class(owner)

    named = {}
    for k, m in methods.items():
        if k in ported or k in mentioned or m["name"].startswith("<") or len(m["name"]) < 4:
            continue
        snake = re.sub(r"(?<!^)(?=[A-Z])", "_", m["name"]).lower()
        if snake in GENERIC_CALLS:
            continue
        homes = files_citing.get(m["class"], set())
        outer = van.classes[m["class"]]["outer"]
        if outer:
            homes = homes | files_citing.get(outer, set())
        hits = [fid for fid in by_name_live.get(snake, ()) if fns[fid]["file"] in homes and owner_fits(fid, m["class"])]
        if hits:
            named[k] = hits
    named_via = {k: "file cites the class" for k in named}
    # Then by the class's own name, for files that cite nothing: a same-named fn in `impl ShulkerBullet`,
    # or in shulker_bullet.rs (ShulkerBulletEntity, ShulkerBullets too).
    files_by_stem = defaultdict(set)
    for fn in fns:
        if not fn.get("test"):
            files_by_stem[fn["file"].rsplit("/", 1)[-1][:-3]].add(fn["file"])
    for k, m in methods.items():
        if (k in ported or k in mentioned or k in named or m["name"].startswith("<") or len(m["name"]) < 4
                or m["class"].startswith(SKIP_PACKAGES)):
            continue
        snake = snake_case(m["name"])
        cands = by_name_live.get(snake, ()) if snake not in GENERIC_CALLS else ()
        if not cands:
            continue
        simple = m["class"].rsplit("/", 1)[-1].split("$")[-1]
        outer = van.classes[m["class"]]["outer"]
        homes = set().union(*(files_by_stem.get(st, set()) for st in class_stems(outer or m["class"])))
        if outer:
            homes |= files_citing.get(outer, set())
        via = "impl " + simple
        hits = [fid for fid in cands if fns[fid].get("owner") == simple
                and (not outer or fns[fid]["file"] in homes)]
        if not hits and not outer:
            via = "file name"
            hits = [fid for fid in cands if fns[fid]["file"] in homes and owner_fits(fid, m["class"])]
        if hits:
            named[k] = hits
            named_via[k] = via

    # Inferred matches for what nobody cites: rare shared ids and numbers, plus the name.
    df_j, df_r = Counter(), Counter()
    for m in methods.values():
        df_j.update(set(m["tokens"]))
    for fn in fns:
        df_r.update(set(fn["tokens"]))
    by_token = defaultdict(list)
    for fn in fns:
        for t in fn["tokens"]:
            if df_r[t] <= 6:
                by_token[t].append(fn["id"])
    nj, nr = max(1, len(methods)), max(1, len(fns))
    inferred = {}
    for k, m in methods.items():
        if k in ported or k in mentioned or k in named or m["class"].startswith(SKIP_PACKAGES):
            continue
        rare = [t for t in m["tokens"] if df_j[t] <= 6 and t in by_token]
        if not rare:
            continue
        score = defaultdict(float)
        shared = defaultdict(list)
        for t in rare:
            w = math.log(nj / df_j[t]) + math.log(nr / df_r[t])
            for fid in by_token[t]:
                score[fid] += w
                shared[fid].append(t)
        snake = re.sub(r"(?<!^)(?=[A-Z])", "_", m["name"]).lower()
        best = []
        for fid, s in score.items():
            n_shared = len(shared[fid])
            ids = sum(t.startswith("id:") for t in shared[fid])
            name_hit = fns[fid]["name"] == snake or snake in fns[fid]["name"]
            if n_shared >= 3 or (n_shared >= 2 and (ids or name_hit)) or (ids and name_hit):
                best.append((s + (8 if name_hit else 0), fid, shared[fid]))
        best.sort(reverse=True)
        if best:
            inferred[k] = [{"fn": fid, "score": round(s, 1), "shared": sh[:6]} for s, fid, sh in best[:3]]

    # Rust call graph: a call resolves through its receiver's type (self, a parameter, a let, struct
    # fields; a trait to its impls) when the type is known, else by name, fanning out to at most a few fns.
    by_name = defaultdict(list)
    by_owner = defaultdict(list)
    for fn in fns:
        by_name[fn["name"]].append(fn["id"])
        by_owner[(fn.get("owner"), fn["name"])].append(fn["id"])
    structs = defaultdict(dict)
    impls_of = defaultdict(set)
    for f in rust:
        for name, fields in f["structs"].items():
            structs[name].update(fields)
        for trait, typ in f["trait_impls"]:
            impls_of[trait].add(typ)

    def resolve_typed(fn, path, name):
        root = path[0]
        if root in ("self", "Self"):
            t = fn.get("owner")
        elif root[:1].isupper():
            t = root
        else:
            t = fn.get("params", {}).get(root) or fn.get("locals", {}).get(root)
        for seg in path[1:]:
            t = structs.get(t, {}).get(seg) if t else None
        if not t:
            return None
        found = list(by_owner.get((t, name), ()))
        for impl_type in impls_of.get(t, ()):
            found += by_owner.get((impl_type, name), ())
        return found or None

    edges = defaultdict(set)
    unresolved_names = defaultdict(set)
    typed_hits = 0
    for fn in fns:
        precise = set()
        for path, name in fn.get("typed", []):
            hit = resolve_typed(fn, path, name)
            if hit:
                typed_hits += 1
                edges[fn["id"]].update(hit)
                precise.add(name)
        for c in fn["calls"]:
            if c in precise or c in RUST_KEYWORDS:
                continue
            cands = by_name.get(c, ())
            if c not in GENERIC_CALLS and 0 < len(cands) <= 4:
                edges[fn["id"]].update(cands)
            else:
                unresolved_names[fn["id"]].add(c)
        edges[fn["id"]].discard(fn["id"])
    rcallers = defaultdict(set)
    for f, targets in edges.items():
        for t in targets:
            rcallers[t].add(f)

    # The verdict map: what someone read and decided where nothing else finds a port. A method row maps it
    # to a fn (`port`), to the fn it is folded into (`inline`), to nothing (`missing`), or says why it needs
    # no port (`skip:<reason>`); a class row (`skip:`, `missing`, or `home` with files) covers the methods of
    # the class that have no row of their own.
    fn_at = defaultdict(list)
    for fn in fns:
        if not fn.get("decl"):
            fn_at[(fn["file"], fn.get("owner") or "", fn["name"])].append(fn["id"])
            fn_at[(fn["file"], "*", fn["name"])].append(fn["id"])
    mapped, skipped, absent, map_homes, map_problems = {}, {}, {}, defaultdict(list), []

    def map_keys(name):
        """The vanilla keys a map row names: Class.method, Outer.Inner.method, a class, or a full path#method."""
        if "#" in name:
            return [name] if name in methods else [], None
        head, _, last = name.rpartition(".")
        if head and (last[:1].islower() or last.startswith("<")):
            cls_q, meth = head, last
        else:
            cls_q, meth = name, None
        q = cls_q.replace(".", "$")
        cands = [p for p in van.classes if p.endswith("/" + q) or p.rsplit("/", 1)[-1] == q]
        if len(cands) != 1:
            return [], ("no class " if not cands else "ambiguous class ") + cls_q
        cp = cands[0]
        if meth is None:
            return [cp], None
        if key(cp, meth) in methods:
            return [key(cp, meth)], None
        decl, _ = van.find_method(cp, meth)
        return ([key(decl, meth)], None) if decl else ([], f"no method {meth} on {cls_q}")

    class_rows = []
    for row in read_port_map():
        keys, problem = map_keys(row["vanilla"])
        verdict = row["verdict"]
        if not keys:
            map_problems.append({**row, "problem": problem or "unknown"})
            continue
        if keys[0] in van.classes:
            class_rows.append((keys[0], row))
            continue
        k = keys[0]
        if verdict in ("port", "inline"):
            file_, _, rest = row["where"].partition("::")
            owner, _, fname = rest.rpartition("::")
            hit = fn_at.get((file_, owner, fname)) or fn_at.get((file_, "*", fname))
            if not hit:
                map_problems.append({**row, "problem": f"no fn {row['where']}"})
                continue
            mapped[k] = {"fn": hit[0], "verdict": verdict, "note": row["note"], "src": row["src"]}
        elif verdict == "missing":
            absent[k] = {"note": row["note"], "src": row["src"]}
        elif verdict.startswith("skip:") and verdict[5:] in SKIP_REASONS:
            skipped[k] = {"reason": verdict[5:], "note": row["note"], "src": row["src"]}
        else:
            map_problems.append({**row, "problem": f"unknown verdict {verdict}"})
    for cp, row in class_rows:
        verdict = row["verdict"]
        if verdict == "home":
            files_ = [f.strip() for f in row["where"].split(",") if f.strip()]
            bad = [f for f in files_ if not (ROOT / f).is_file()]
            if bad:
                map_problems.append({**row, "problem": "no file " + ", ".join(bad)})
            map_homes[cp] += [f for f in files_ if f not in bad]
            continue
        if verdict != "missing" and not (verdict.startswith("skip:") and verdict[5:] in SKIP_REASONS):
            map_problems.append({**row, "problem": f"a class row takes skip:<reason>, missing or home, not {verdict}"})
            continue
        for k, m in methods.items():
            if m["class"] == cp and k not in ported and k not in mapped and k not in absent and k not in skipped:
                (absent if verdict == "missing" else skipped)[k] = (
                    {"note": row["note"], "src": row["src"]} if verdict == "missing"
                    else {"reason": verdict[5:], "note": row["note"], "src": row["src"]})

    # Linked matches: walk the call graph out from the known pairs. When X ports as A, a callee of X with
    # no port and a callee of A that nothing cites are a pair too if their names agree (less Java's get).
    claimed = {f for d in (ported, named, mentioned) for v in d.values() for f in v}
    known = defaultdict(set)
    for d in (ported, named):
        for k, v in d.items():
            known[k] |= set(v)
    for k, v in mapped.items():
        known[k].add(v["fn"])
        claimed.add(v["fn"])
    decided = set(mapped) | set(skipped) | set(absent)
    linked = {}

    def bare(name):
        sn = snake_case(name)
        return sn[4:] if sn.startswith("get_") and len(sn) > 7 else sn

    for _ in range(3):
        added = {}
        for k, fids in list(known.items()):
            m = methods.get(k)
            if not m:
                continue
            jc = [c for c in m["callees"] if c in methods and c not in known and c not in mentioned
                  and c not in added and c not in decided and not methods[c]["name"].startswith("<")]
            rc = {c for f in fids for c in edges.get(f, ()) if c not in claimed and not fns[c].get("test")
                  and fns[c]["name"] not in GENERIC_CALLS}
            if not jc or not rc:
                continue
            for c in jc:
                b = bare(methods[c]["name"])
                if b in GENERIC_CALLS or len(b) < 4:
                    continue
                scored = sorted(((name_similarity(b, fns[r]["name"]), r) for r in rc), reverse=True)
                if scored[0][0] >= 0.67 and (len(scored) == 1 or scored[1][0] < scored[0][0]):
                    added[c] = (scored[0][1], k)
        if not added:
            break
        for c, (fid, via) in added.items():
            if fid in claimed:
                continue
            claimed.add(fid)
            linked[c] = {"fn": fid, "via": via}
            known[c] = {fid}

    # Class homes: the Rust files a vanilla class lives in, weighed from every match of its methods, the
    # files citing it, files named for it, and impls of a type named for it; a nested class without its own
    # lives with its outer class. `where`, `issue` and the structural matches below start from these.
    home_w = defaultdict(Counter)
    for k, fids in known.items():
        for f in fids:
            if k in methods and not fns[f].get("test"):
                home_w[methods[k]["class"]][fns[f]["file"]] += 3
    for k, fids in mentioned.items():
        for f in fids:
            if k in methods and not fns[f].get("test"):
                home_w[methods[k]["class"]][fns[f]["file"]] += 1
    for cp, ids in class_cites.items():
        for f in ids:
            if not fns[f].get("test"):
                home_w[cp][fns[f]["file"]] += 1
    for cp, files_ in file_class_cites.items():
        for f in files_:
            home_w[cp][f] += 1
    owner_files = defaultdict(set)
    for fn in fns:
        if fn.get("owner") and not fn.get("test"):
            owner_files[fn["owner"]].add(fn["file"])
    for cp in van.classes:
        simple = cp.rsplit("/", 1)[-1].split("$")[-1]
        for st in class_stems(cp):
            for f in files_by_stem.get(st, ()):
                home_w[cp][f] += 2
        if "$" not in cp or "_" in snake_case(simple):
            for f in owner_files.get(simple, ()):
                home_w[cp][f] += 2
    homes = {}
    for cp, w in home_w.items():
        top = max(w.values())
        homes[cp] = [f for f, x in w.most_common() if x >= max(2, top * 0.25)][:6]
    for cp, files_ in map_homes.items():
        homes[cp] = files_ + [f for f in homes.get(cp, []) if f not in files_]

    def home_of(cp):
        return homes.get(cp) or homes.get(cp.split("$")[0]) or []

    # Structural matches: in the class's home files, the fn that touches the same state (the fields a
    # method reads and writes, by their Rust names) and reaches the ports of the same callees, weighed
    # against its size so one long fn doesn't take every method.
    field_df = Counter()
    fns_in = defaultdict(list)
    for fn in fns:
        if not fn.get("test") and not fn.get("decl"):
            field_df.update(set(fn["fields"]))
            fns_in[fn["file"]].append(fn["id"])
    port_names_of = {k: {fns[f]["name"] for f in v} for k, v in known.items()}
    structural = {}
    for k, m in methods.items():
        if (k in known or k in mentioned or k in decided or m["anonymous"] or m["name"].startswith("<")
                or m["accessor"] or m["abstract"] or m["size"] <= 3 or m["class"].startswith(SKIP_PACKAGES)):
            continue
        cands = {f for file_ in home_of(m["class"]) for f in fns_in.get(file_, ())}
        if not cands:
            continue
        want_fields = set(m["touches"])
        want_calls = [port_names_of[c] for c in m["callees"] if c in port_names_of]
        b = bare(m["name"])
        scored = []
        for fid in cands:
            fn = fns[fid]
            if not owner_fits(fid, m["class"]):
                continue
            fs = sum(1.0 if field_df[x] <= 30 else 0.5 if field_df[x] <= 200 else 0.15
                     for x in want_fields & set(fn["fields"]))
            reach1 = set(fn["calls"]) | {fns[e]["name"] for e in edges.get(fid, ())}
            cs = sum(1 for names in want_calls if names & reach1)
            ns = name_similarity(b, fn["name"])
            evidence = (fs + 1.5 * cs) / (1 + math.log1p(((fn["end"] or fn["line"]) - fn["line"]) / 40))
            scored.append((evidence + 3 * ns, evidence, ns, fid))
        if not scored:
            continue
        scored.sort(reverse=True)
        score, evidence, ns, fid = scored[0]
        second = scored[1][0] if len(scored) > 1 else 0.0
        specific = "_" in b and b not in GENERIC_CALLS
        if (ns >= 0.5 and evidence >= 0.5) or (ns >= 0.99 and specific) or (evidence >= 2.5 and score >= 1.4 * second + 0.3):
            structural[k] = {"fn": fid, "evidence": round(evidence, 1), "name": round(ns, 2)}
    for k, v in structural.items():
        known.setdefault(k, set()).add(v["fn"])

    harness = harness_coverage(van, methods, fns, edges, known)

    def reach(fid, depth=5):
        """Names of the fns `fid` reaches within `depth` calls, and of the calls left unresolved."""
        seen, frontier = {fid}, {fid}
        names = set()
        for _ in range(depth):
            nxt = set()
            for f in frontier:
                names |= unresolved_names.get(f, set())
                nxt |= edges.get(f, set()) - seen
            seen |= nxt
            frontier = nxt
        return names | {fns[f]["name"] for f in seen if f != fid}

    remote = {fn["id"] for fn in fns if re.search(r"/game/remote[^/]*\.rs$", fn["file"])}

    def ancestors(fid, depth=2):
        """The fns that call `fid` within `depth` steps (through specific names only)."""
        seen, frontier = set(), {fid}
        for _ in range(depth):
            nxt = set()
            for f in frontier:
                if fns[f]["name"] in GENERIC_CALLS or len(by_name.get(fns[f]["name"], ())) > 4:
                    continue
                nxt |= rcallers.get(f, set()) - seen
            seen |= nxt
            frontier = nxt
        return seen

    def side(fid):
        """Which path a port runs on: the server-client side (game/remote*.rs), the server
        (game/lan*.rs, src/lan/), or the in-frame game."""
        f = fns[fid]["file"]
        if re.search(r"/game/remote[^/]*\.rs$", f):
            return "client"
        if re.search(r"/game/lan[^/]*\.rs$|/src/lan/", f):
            return "server"
        return "frame"

    def callee_port_names(c):
        """Names of the fns that port vanilla method c: those whose doc opens with it, else a fn with the
        same, specific name (itemUsed -> item_used)."""
        names = {fns[f]["name"] for o in [c] + overrides(c) for f in known.get(o, ())}
        if names:
            return names
        snake = re.sub(r"(?<!^)(?=[A-Z])", "_", methods[c]["name"]).lower()
        if (len(snake) >= 8 or "_" in snake) and snake not in GENERIC_CALLS and 0 < len(by_name.get(snake, ())) <= 4:
            return {snake}
        return set()

    def substantial(c):
        """Worth reporting as a missing call: Horizon ports it as a function of its own (or an override
        of it), or it is more than a getter."""
        if any(ported.get(o) for o in [c] + overrides(c)):
            return True
        m = methods[c]
        if m["size"] <= 3 or m["accessor"]:
            return False
        return not re.match(r"(get|is|has|can|should|set)[A-Z]", m["name"]) or m["size"] > 8

    def reached_from_remote(fid, depth=4):
        seen, frontier = {fid}, {fid}
        for _ in range(depth):
            if frontier & remote:
                return True
            nxt = set()
            for f in frontier:
                if fns[f]["name"] in GENERIC_CALLS:
                    continue
                nxt |= rcallers.get(f, set()) - seen
            seen |= nxt
            frontier = nxt
        return bool(frontier & remote)

    gaps = []
    for k, fids in ported.items():
        m = methods.get(k)
        if not m:
            continue
        reaches = {fid: reach(fid) for fid in fids}
        port_names = {c: callee_port_names(c) for c in m["callees"]}
        port_names = {c: v for c, v in port_names.items() if v}
        # One port is often split over several fns (the click, then each hand's turn; a tick spread over
        # the loop): every port on one path is judged as one, against the ports on the other paths.
        by_side = defaultdict(list)
        for fid in sorted(fids, key=lambda f: (fns[f]["file"], fns[f]["line"])):
            by_side[side(fid)].append(fid)
        clusters = list(by_side.values())
        creach = [set().union(*(reaches[f] for f in c)) | {fns[f]["name"] for f in c} for c in clusters]
        ccover = [{c for c, names in port_names.items() if r & names} for r in creach]
        for callee, targets in port_names.items():
            if methods[callee]["class"].rsplit("/", 1)[-1].split("$")[0] in UTILITY_CLASSES:
                continue
            hit = [i for i, r in enumerate(creach) if r & targets]
            miss = [i for i, r in enumerate(creach) if not (r & targets)]
            if hit and miss and substantial(callee):
                for i in miss:
                    peers = [h for h in hit if side(clusters[h][0]) != side(clusters[i][0]) and ccover[h] & ccover[i]]
                    if peers:
                        # Singleplayer runs as a client of the integrated server, so a gap on the
                        # in-frame path rarely shows; it sorts after the others.
                        gaps.append({"kind": "seam", "vanilla": k, "callee": callee, "fn": clusters[i][0],
                                     "cluster": clusters[i], "sibling": clusters[peers[0]][0],
                                     "old_path": side(clusters[i][0]) == "frame"})
            elif not hit and len(targets) <= 3 and substantial(callee):
                gaps.append({"kind": "callee", "vanilla": k, "callee": callee, "fn": fids[0]})
        if m["client"] and not any(reached_from_remote(fid) for fid in fids):
            gaps.append({"kind": "client", "vanilla": k, "fn": fids[0]})
    # Drift: what a vanilla method names (a sound, particle, item, block, effect) or the rare numbers it
    # uses, that no port of it (nor what the port calls, two steps deep) has.
    df_tok = Counter()
    for m in methods.values():
        df_tok.update(set(m["tokens"]))
    for k, fids in ported.items():
        m = methods.get(k)
        if not m or not m["tokens"]:
            continue
        # A port a fraction of the method's size ports a part of it; what the rest names is not drift.
        if m["size"] > 20 and sum(fns[f]["end"] - fns[f]["line"] + 1 for f in fids) * 4 < m["size"]:
            continue
        have, words, seen = set(), set(), set(fids)
        frontier = set(fids)
        for depth in range(3):
            for f in frontier:
                have |= set(fns[f]["tokens"])
                words |= set(fns[f]["words"])
            if depth == 2:
                break
            frontier = {c for f in frontier for c in edges.get(f, ())} - seen
            seen |= frontier
        words |= {t[3:] for t in have if t.startswith("id:")}
        long_words = [w for w in words if "_" in w or len(w) >= 6]

        def spelled(ident):
            if ident in words:
                return True
            # entity.shulker_bullet.hit vs Sound::ShulkerBulletHit; speed vs MOVEMENT_SPEED
            return any(ident.endswith("_" + w) or w.endswith("_" + ident) for w in long_words)

        missing_ids = sorted(t for t in m["tokens"] if t.startswith("id:") and not spelled(t[3:]))
        missing_nums = sorted(t for t in m["tokens"] if t.startswith("n:") and t not in have and df_tok[t] <= 40)
        if missing_ids or missing_nums:
            main = max(fids, key=lambda f: fns[f]["end"] - fns[f]["line"])
            gaps.append({"kind": "drift", "vanilla": k, "fn": main, "ids": missing_ids, "numbers": missing_nums})

    for s in stale:
        gaps.append({"kind": "stale", **s})
    for row in map_problems:
        gaps.append({"kind": "map", "src": row["src"], "row": row["vanilla"], "problem": row["problem"]})
    # Whole classes nothing ports, in packages where Horizon ports the neighbours (a renderer, a sound
    # instance, a goal): the kind of gap no call graph shows, since the game reaches them by registration.
    by_class = defaultdict(list)
    for k, m in methods.items():
        if not m["abstract"] and not m["accessor"] and m["name"] != "<clinit>":
            by_class[m["class"]].append(k)
    pkg_rate = defaultdict(Counter)
    for cp, ks in by_class.items():
        pkg = cp.rsplit("/", 1)[0]
        hit = any(x in ported or x in mentioned for x in ks) or cp in class_cites
        pkg_rate[pkg]["classes"] += 1
        pkg_rate[pkg]["hit"] += hit
    for cp, ks in by_class.items():
        if cp.startswith(SKIP_PACKAGES) or cp.startswith("com/") or "$" in cp or len(ks) < 2 or van.classes[cp].get("anonymous"):
            continue
        if any(x in ported or x in mentioned or x in inferred for x in ks) or cp in class_cites or cp in file_class_cites:
            continue
        if van.classes[cp]["kind"] in ("interface", "@interface", "record"):
            continue
        pkg = cp.rsplit("/", 1)[0]
        rate = pkg_rate[pkg]["hit"] / max(1, pkg_rate[pkg]["classes"])
        if rate >= 0.3:
            gaps.append({"kind": "class", "vanilla_class": cp, "package_ported": round(rate, 2), "methods": len(ks)})

    stats = {"typed_calls_resolved": typed_hits, "edges": sum(len(v) for v in edges.values())}
    ledger = {
        "stats": stats,
        "sources": sources,
        "jars": jars,
        "methods": methods,
        "callers": {k: sorted(v) for k, v in callers.items()},
        "fns": fns,
        "ported": dict(ported),
        "mentioned": dict(mentioned),
        "named": named,
        "named_via": named_via,
        "linked": linked,
        "structural": structural,
        "mapped": mapped,
        "skipped": skipped,
        "absent": absent,
        "homes": homes,
        "aliases": aliases,
        "harness": harness,
        "inferred": inferred,
        "class_cites": {k: sorted(set(v)) for k, v in class_cites.items()},
        "file_class_cites": {k: sorted(v) for k, v in file_class_cites.items()},
        "gaps": gaps,
        "classes": {c["path"]: {"file": c["file"], "line": c["line"], "extends": c["extends"], "kind": c["kind"]} for c in classes},
    }
    for fn in fns:
        fn.pop("typed", None)
        fn.pop("locals", None)
        fn.pop("params", None)
        fn.pop("words", None)
        fn["callees"] = sorted(edges.get(fn["id"], ()))
    OUT.mkdir(parents=True, exist_ok=True)
    LEDGER.write_text(json.dumps(ledger))
    return ledger


# ---- Queries ------------------------------------------------------------------------------------------


def load():
    if not LEDGER.exists():
        sys.exit("No ledger yet: run `python3 tools/port_ledger.py build` first.")
    return json.loads(LEDGER.read_text())


def fn_ref(led, fid):
    f = led["fns"][fid]
    owner = f"{f['owner']}::" if f.get("owner") else ""
    return f"{f['file']}:{f['line']}-{f['end']} {owner}{f['name']}"


def status_of(led, k):
    if k in led["ported"]:
        return "ported"
    if k in led.get("mapped", {}):
        return "mapped"
    if k in led.get("absent", {}):
        return "absent"
    if k in led.get("skipped", {}):
        return "skipped"
    if k in led["mentioned"]:
        return "mentioned"
    if k in led.get("named", {}):
        return "named"
    if k in led.get("linked", {}):
        return "linked"
    if k in led.get("structural", {}):
        return "structural"
    if k in led["inferred"]:
        return "inferred"
    m = led["methods"][k]
    if m.get("accessor") or m.get("abstract") or m.get("size", 9) <= 3 or (m["name"] == "<init>" and m.get("size", 0) <= 30):
        # A getter, setter, abstract or one-line method, or a constructor that only sets fields (the
        # struct's initialiser): nothing to port on its own.
        return "trivial"
    return "missing"


def port_of(led, k):
    """The Rust fns a method's best match names, best first."""
    if k in led["ported"]:
        return led["ported"][k]
    for d in ("mapped", "linked", "structural"):
        if k in led.get(d, {}):
            return [led[d][k]["fn"]]
    if k in led.get("named", {}):
        return led["named"][k]
    return [i["fn"] for i in led["inferred"].get(k, [])[:1]]


def match_classes(led, query):
    q = query.replace(".", "$")
    return [p for p in led["classes"] if p.endswith("/" + q) or p == q or p.rsplit("/", 1)[-1] == q]


def cmd_where(led, query, as_json):
    out = []
    if "." in query and query.split(".")[-1][:1].islower():
        cls_q, meth = query.rsplit(".", 1)
        for cp in match_classes(led, cls_q):
            # The method on the class or a supertype.
            for p in [cp] + [x for x in led["classes"] if False]:
                k = key(p, meth)
                if k in led["methods"]:
                    out.append(method_view(led, k))
    else:
        for cp in match_classes(led, query):
            fids = led["class_cites"].get(cp, [])
            meths = [k for k in led["methods"] if led["methods"][k]["class"] == cp]
            out.append({
                "class": short(cp), "path": cp, "file": led["classes"][cp]["file"],
                "home": led.get("homes", {}).get(cp, []),
                "named_in": [fn_ref(led, f) for f in fids[:40]] + led.get("file_class_cites", {}).get(cp, []),
                "methods": {led["methods"][k]["name"]: status_of(led, k) for k in sorted(meths)},
            })
    emit(out, as_json, f"Nothing in the pinned version is called {query}.")


def method_view(led, k):
    m = led["methods"][k]
    return {
        "vanilla": short(m["class"], m["name"]),
        "source": f"{m['file']}:{m['lines'][0][0] if m['lines'] else '?'}",
        "status": status_of(led, k),
        "ported_in": [fn_ref(led, f) for f in led["ported"].get(k, [])],
        "mentioned_in": [fn_ref(led, f) for f in led["mentioned"].get(k, [])][:20],
        "same_name_in": [fn_ref(led, f) + f" (by {led.get('named_via', {}).get(k, 'name')})" for f in led.get("named", {}).get(k, [])],
        "linked": ([f"{fn_ref(led, led['linked'][k]['fn'])} (called where {short(led['methods'][led['linked'][k]['via']]['class'], led['methods'][led['linked'][k]['via']]['name'])} is ported)"]
                   if k in led.get("linked", {}) else []),
        "structural": ([f"{fn_ref(led, led['structural'][k]['fn'])} (shares state and calls, evidence {led['structural'][k]['evidence']})"]
                       if k in led.get("structural", {}) else []),
        "map": ([f"{led['mapped'][k]['verdict']}: {fn_ref(led, led['mapped'][k]['fn'])} {led['mapped'][k]['note']} ({led['mapped'][k]['src']})"]
                if k in led.get("mapped", {}) else
                [f"skip:{led['skipped'][k]['reason']} {led['skipped'][k]['note']} ({led['skipped'][k]['src']})"] if k in led.get("skipped", {}) else
                [f"missing {led['absent'][k]['note']} ({led['absent'][k]['src']})"] if k in led.get("absent", {}) else []),
        "class_home": led.get("homes", {}).get(m["class"]) or led.get("homes", {}).get(m["class"].split("$")[0], []),
        "parity_harnesses": led.get("harness", {}).get(k, []),
        "inferred": [f"{fn_ref(led, i['fn'])} (score {i['score']}, shares {', '.join(i['shared'])})" for i in led["inferred"].get(k, [])],
        "client_effect": m["client"],
        "calls": [f"{short(led['methods'][c]['class'], led['methods'][c]['name'])} [{status_of(led, c)}]" for c in m["callees"] if c in led["methods"]][:40],
        "called_by": [short(led["methods"][c]["class"], led["methods"][c]["name"]) for c in led["callers"].get(k, []) if c in led["methods"]][:40],
        "gaps": [gap_text(led, g) for g in led["gaps"] if g.get("vanilla") == k],
    }


def cmd_what(led, where, as_json):
    path, _, line = where.partition(":")
    path = path.removeprefix("./")
    fns = [f for f in led["fns"] if f["file"] == path]
    if line:
        n = int(line)
        inside = [f for f in fns if f["line"] <= n <= (f["end"] or f["line"])]
        fns = inside[-1:] if inside else []
    out = []
    for f in fns:
        out.append({
            "fn": fn_ref(led, f["id"]),
            "ports": [short(led["methods"][k]["class"], led["methods"][k]["name"]) for k in f["primary"] if k in led["methods"]],
            "mentions": [short(led["methods"][k]["class"], led["methods"][k]["name"]) for k in f["cites"] if k in led["methods"] and k not in f["primary"]],
            "same_name_as": [short(led["methods"][k]["class"], led["methods"][k]["name"]) for k, v in led.get("named", {}).items() if f["id"] in v],
            "linked_to": [short(led["methods"][k]["class"], led["methods"][k]["name"]) for k, v in led.get("linked", {}).items() if v["fn"] == f["id"]],
            "mapped_as": [short(led["methods"][k]["class"], led["methods"][k]["name"]) + f" ({v['verdict']})" for k, v in led.get("mapped", {}).items() if v["fn"] == f["id"]],
            "structural_for": [short(led["methods"][k]["class"], led["methods"][k]["name"]) for k, v in led.get("structural", {}).items() if v["fn"] == f["id"]],
            "inferred_for": [short(led["methods"][k]["class"], led["methods"][k]["name"]) for k, v in led["inferred"].items() if any(i["fn"] == f["id"] for i in v)],
        })
    emit(out, as_json, f"No function at {where}.")


def cmd_class(led, query, as_json):
    out = []
    for cp in match_classes(led, query):
        rows = []
        for k in sorted(k for k in led["methods"] if led["methods"][k]["class"] == cp):
            m = led["methods"][k]
            where = [fn_ref(led, f) for f in port_of(led, k)[:3]] or [
                fn_ref(led, f) + " (mention)" for f in led["mentioned"].get(k, [])[:1]]
            rows.append({"method": m["name"], "status": status_of(led, k), "where": where, "callers": len(led["callers"].get(k, []))})
        out.append({"class": short(cp), "file": led["classes"][cp]["file"], "home": led.get("homes", {}).get(cp, []),
                    "methods": rows})
    emit(out, as_json, f"No class {query}.")


def package_of(path, depth):
    parts = path.split("/")
    base = 2 if parts[:2] == ["net", "minecraft"] else 1
    return "/".join(parts[base:base + depth])


def coverage_rows(led, package=None, depth=2):
    rows = defaultdict(Counter)
    for k, m in led["methods"].items():
        if (m["class"].startswith(SKIP_PACKAGES) or m["accessor"] or m["abstract"] or m["name"] == "<clinit>"
                or m.get("anonymous")):
            continue
        pkg = package_of(m["class"], depth)
        if package and not pkg.startswith(package) and not m["class"].split("/", 2)[-1].startswith(package):
            continue
        rows[pkg][status_of(led, k)] += 1
        rows[pkg]["total"] += 1
    return rows


COVERAGE_COLS = ("ported", "mapped", "mentioned", "named", "linked", "structural", "inferred", "skipped", "absent",
                 "trivial", "missing")


def found_pct(c):
    """The share of the methods worth porting (not trivial, not skipped) that have a port found."""
    worth = c["total"] - c["trivial"] - c["skipped"]
    return round(100 * (worth - c["missing"] - c["absent"]) / worth, 1) if worth else 100.0


def classified_pct(c):
    """The share of all methods with an answer: a port, a reason to skip, trivial, or known to be missing."""
    return round(100 * (c["total"] - c["missing"]) / c["total"], 1) if c["total"] else 100.0


def cmd_coverage(led, package, depth, as_json):
    rows = coverage_rows(led, package, depth)
    out = [dict({"package": p, "total": c["total"]}, **{k: c[k] for k in COVERAGE_COLS}, found_pct=found_pct(c),
                answered_pct=classified_pct(c))
           for p, c in sorted(rows.items(), key=lambda kv: -kv[1]["total"])]
    if as_json:
        print(json.dumps(out, indent=1))
        return
    tot = Counter()
    for r in out:
        tot.update({k: r[k] for k in ("total",) + COVERAGE_COLS})
    print(f"{'package':32s} {'methods':>7s} " + " ".join(f"{k[:6]:>6s}" for k in COVERAGE_COLS) + " found% answered%")
    for r in out + ([dict({"package": "all"}, **tot, found_pct=found_pct(tot), answered_pct=classified_pct(tot))]
                    if tot["total"] else []):
        print(f"{r['package'][:32]:32s} {r['total']:7d} " + " ".join(f"{r[k]:6d}" for k in COVERAGE_COLS)
              + f" {r['found_pct']:6.0f} {r['answered_pct']:8.0f}")


def gap_text(led, g):
    m = led["methods"]
    name = lambda k: short(m[k]["class"], m[k]["name"]) if k in m else k
    if g["kind"] == "seam":
        rest = [led["fns"][f]["name"] for f in g.get("cluster", [])[1:]]
        with_rest = f" (with {', '.join(rest)})" if rest else ""
        return (f"seam: {fn_ref(led, g['fn'])}{with_rest} ports {name(g['vanilla'])} but never reaches the port of "
                f"{name(g['callee'])}; {fn_ref(led, g['sibling'])} does" + (" (old in-frame path)" if g.get("old_path") else ""))
    if g["kind"] == "callee":
        return f"callee: {name(g['vanilla'])} calls {name(g['callee'])}; {fn_ref(led, g['fn'])} doesn't reach its port"
    if g["kind"] == "client":
        return (f"client: {name(g['vanilla'])} shows an effect only the client's copy draws; its port "
                f"{fn_ref(led, g['fn'])} isn't reached from game/remote*.rs")
    if g["kind"] == "drift":
        parts = []
        if g["ids"]:
            parts.append("never names " + ", ".join(t[3:] for t in g["ids"][:8]))
        if g["numbers"]:
            parts.append("lacks the constants " + ", ".join(t[2:] for t in g["numbers"][:8]))
        return f"drift: the port of {name(g['vanilla'])} ({fn_ref(led, g['fn'])}) " + " and ".join(parts)
    if g["kind"] == "class":
        return (f"class: nothing ports {short(g['vanilla_class'])} ({g['methods']} methods), though "
                f"{round(100 * g['package_ported'])}% of {g['vanilla_class'].rsplit('/', 1)[0]} is ported")
    if g["kind"] == "stale":
        return f"stale: {g['file']}:{g['line']} cites {g['cite']}, which the pinned version doesn't have"
    if g["kind"] == "map":
        return f"map: {g['src']} ({g['row']}): {g['problem']}"
    return json.dumps(g)


GAP_ORDER = {"map": -1, "seam": 0, "client": 1, "drift": 2, "class": 3, "callee": 4, "stale": 5}


def gap_rank(g):
    """Best leads first: by kind, the live path before the old one, then the classes with the most ported neighbours."""
    return (GAP_ORDER.get(g["kind"], 9), g.get("old_path", False), -g.get("package_ported", 0), -len(g.get("ids", ())),
            g.get("vanilla") or g.get("vanilla_class") or g.get("file", ""))


def untested(led, package=None, limit=50):
    """Ported methods no parity harness covers, most-called first: where a harness would catch the most drift."""
    harness = led.get("harness", {})
    rows = []
    for k, m in led["methods"].items():
        if m["class"].startswith(SKIP_PACKAGES) or k in harness or m["name"].startswith("<"):
            continue
        if package and not m["class"].startswith("net/minecraft/" + package):
            continue
        # Sure pairs only (cited, or answered in the map), and methods with behaviour of their own.
        if status_of(led, k) not in ("ported", "mapped") or m.get("accessor") or m.get("size", 0) <= 8:
            continue
        rows.append((len(led["callers"].get(k, [])), k))
    rows.sort(reverse=True)
    return [{"vanilla": short(led["methods"][k]["class"], led["methods"][k]["name"]), "callers": n,
             "port": fn_ref(led, port_of(led, k)[0]) if port_of(led, k) else ""} for n, k in rows[:limit]]


def unported(led, package=None, limit=50):
    rows = []
    for k, m in led["methods"].items():
        if (status_of(led, k) != "missing" or m["class"].startswith(SKIP_PACKAGES) or m["accessor"] or m["abstract"]
                or m.get("anonymous")):
            continue
        if m["name"] in ("<clinit>", "toString", "hashCode", "equals", "codec", "streamCodec") or m["class"].startswith("com/"):
            continue
        if package and not m["class"].startswith("net/minecraft/" + package):
            continue
        # Positions, maths and block-state lookups: Rust has its own types for those, not ports of them.
        if m["class"].rsplit("/", 1)[-1].split("$")[0] in UTILITY_CLASSES | {"StateHolder", "BlockStateBase"}:
            continue
        rows.append((len(led["callers"].get(k, [])), k))
    rows.sort(reverse=True)
    return [{"vanilla": short(led["methods"][k]["class"], led["methods"][k]["name"]), "callers": n,
             "status": status_of(led, k), "source": led["methods"][k]["file"]} for n, k in rows[:limit]]


def cmd_gaps(led, kind, package, limit, as_json):
    if kind == "unported":
        emit(unported(led, package, limit), as_json, "Nothing unported there.")
        return
    if kind == "untested":
        emit(untested(led, package, limit), as_json, "Every port there has a parity harness.")
        return
    gaps = [g for g in led["gaps"] if (not kind or g["kind"] == kind)]
    if package:
        gaps = [g for g in gaps if (g.get("vanilla") or g.get("vanilla_class") or "").startswith("net/minecraft/" + package)
                or package in g.get("file", "")]
    gaps.sort(key=gap_rank)
    shown = gaps[:limit]
    if as_json:
        print(json.dumps([dict(g, text=gap_text(led, g)) for g in shown], indent=1))
        return
    counts = Counter(g["kind"] for g in gaps)
    print(", ".join(f"{k} {v}" for k, v in sorted(counts.items(), key=lambda kv: GAP_ORDER.get(kv[0], 9))) or "No gaps.")
    for g in shown:
        print("-", gap_text(led, g))


def emit(out, as_json, empty):
    if as_json:
        print(json.dumps(out, indent=1))
        return
    if not out:
        print(empty)
        return
    for item in out:
        for k, v in item.items():
            if isinstance(v, list):
                if v:
                    print(f"{k}:")
                    for x in v:
                        print(f"  {x if not isinstance(x, dict) else json.dumps(x)}")
            elif isinstance(v, dict):
                print(f"{k}:")
                for a, b in v.items():
                    print(f"  {a}: {b}")
            else:
                print(f"{k}: {v}")
        print()


ISSUE_REPO = "horizon-rust/horizon"
# Class-name affixes a player never says: "ender pearl" is ThrownEnderpearl, "shulker bullet" ShulkerBullet.
CLASS_AFFIXES = re.compile(r"^(Thrown|Abstract|Base|Client|Server)|(Entity|Item|Block|BlockEntity|Renderer|Model|Goal)$")


def issue_text(ref):
    """An issue's title and text: a GitHub issue number, `-` for stdin, or the report itself."""
    if ref == "-":
        return clean_report(sys.stdin.read())
    if not ref.isdigit():
        return clean_report(ref)
    try:
        raw = subprocess.run(["gh", "issue", "view", ref, "--repo", ISSUE_REPO, "--json", "title,body,comments"],
                             capture_output=True, text=True, check=True, timeout=30).stdout
    except (OSError, subprocess.SubprocessError) as e:
        sys.exit(f"Couldn't read issue {ref} with gh ({e}); pass the report's text instead.")
    data = json.loads(raw)
    # The title counts twice: it names the bug, the body describes everything around it.
    return clean_report("\n".join([data["title"], data["title"], data.get("body") or ""]
                                   + [c["body"] for c in data.get("comments", [])[:10]]))


REPORT_BOILERPLATE = re.compile(
    r"^\W*(\*\*)?(version|os|horizon version|os, cpu, gpu|reported on discord|attachments|opened by|tags)\b.*$"
    r"|^\W*(what i did|what happened|what i expected)\s*:", re.I | re.M)


def clean_report(text):
    """A report less what the bug-report template and its links say about every bug."""
    text = re.sub(r"<!--.*?-->|<[^>]+>|\[([^\]]*)\]\([^)]*\)|https?://\S+", lambda m: m.group(1) or " ", text, flags=re.S)
    return REPORT_BOILERPLATE.sub(" ", text)


def suspect_classes(led, text, limit=8):
    """Vanilla classes a report names, as a player writes them ("shulker bullet", "EnderEyeItem",
    "eye of ender"), most specific first."""
    words = re.findall(r"[a-z0-9]+", re.sub(r"(?<=[a-z0-9])(?=[A-Z])", " ", text).lower())
    phrases = Counter()
    for n in range(1, 5):
        for i in range(len(words) - n + 1):
            phrases["".join(words[i:i + n])] += 1
    title = set(re.findall(r"[a-z0-9]+", text.split("\n", 1)[0].lower()))
    scores = {}
    for cp, c in led["classes"].items():
        if cp.startswith(SKIP_PACKAGES) or re.search(r"\$\d", cp):
            continue
        simple = cp.rsplit("/", 1)[-1].split("$")[-1]
        # An inner class's own name (Storage, Builder, Entry) is too plain to go on by itself.
        inner = 0.3 if "$" in cp and "_" not in snake_case(simple) else 1.0
        for name, weight in ((simple, inner), (CLASS_AFFIXES.sub("", simple), 0.8 * inner)):
            k = name.lower()
            if len(k) < 4 or k not in phrases:
                continue
            parts = snake_case(name).split("_")
            # Longer names say more; one common word ("Player", "Block") alone says little.
            score = weight * (len(k) + 6 * (len(parts) - 1)) * (1.5 if any(p in title for p in parts) else 1.0)
            score *= 1 + 0.2 * min(phrases[k] - 1, 5)
            if score > scores.get(cp, 0):
                scores[cp] = score
    # The names players use: "mooshroom" is MushroomCow, "redstone ore" RedStoneOreBlock.
    for alias, cp in led.get("aliases", {}).items():
        parts = alias.split("_")
        k = "".join(parts)
        if len(k) < 4 or k not in phrases:
            continue
        score = (len(k) + 6 * (len(parts) - 1)) * (1.5 if any(p in title for p in parts) else 1.0)
        score *= 1 + 0.2 * min(phrases[k] - 1, 5)
        if score > scores.get(cp, 0):
            scores[cp] = score
    # A class named inside another (Outer$Inner) yields to its outer class when both match the same words.
    return sorted(scores, key=lambda c: -scores[c])[:limit], scores


def suspect_methods(led, text, classes, limit=10):
    """Vanilla methods the report's words name: in the suspect classes (slotClicked for "right-clicking" in a
    creative inventory report), or anywhere for a rare word of the title ("hitbox" -> getHitbox)."""
    def words_of(t):
        return set(re.findall(r"[a-z0-9]+", re.sub(r"(?<=[a-z0-9])(?=[A-Z])", " ", t).lower()))
    words = {w for w in words_of(text) if len(w) >= 4} | {w[:-1] for w in words_of(text) if w.endswith("s")}
    words |= {w[:-3] for w in words if w.endswith("ing")} | {w[:-2] for w in words if w.endswith("ed")}
    title = {w for w in words_of(text.split("\n", 1)[0]) if len(w) >= 5}
    homes = {c for cp in classes[:3] for c in (cp, cp.split("$")[0])}
    by_word = defaultdict(list)
    for k, m in led["methods"].items():
        if m["class"].startswith(SKIP_PACKAGES) or m["name"].startswith(("<", "lambda$")) or status_of(led, k) == "trivial":
            continue
        for w in set(snake_case(m["name"]).split("_")):
            by_word[w].append(k)
    picks = Counter()
    for w in words:
        for k in by_word.get(w, ()):
            m = led["methods"][k]
            parts = set(snake_case(m["name"]).split("_"))
            shared = len(parts & words) / len(parts)
            if m["class"].split("$")[0] in homes or m["class"] in homes:
                picks[k] = max(picks[k], 2 + shared)
            elif w in title and len(by_word[w]) <= 10 and shared >= 0.5:
                picks[k] = max(picks[k], shared + 1 / len(by_word[w]))
    return [k for k, _ in picks.most_common(limit)]


def cmd_issue(led, ref, as_json):
    text = issue_text(ref)
    classes, scores = suspect_classes(led, text)
    out = []
    methods = suspect_methods(led, text, classes)
    if methods:
        rows = []
        for k in methods:
            fids = port_of(led, k)
            m = led["methods"][k]
            rows.append(f"{short(m['class'], m['name'])} [{status_of(led, k)}]" + (f" -> {fn_ref(led, fids[0])}" if fids else ""))
        out.append({"methods named by the report": rows})
    for cp in classes:
        ks = [k for k, m in led["methods"].items() if m["class"] == cp]
        found = []
        for k in sorted(ks, key=lambda k: -led["methods"][k].get("size", 0)):
            st = status_of(led, k)
            fids = port_of(led, k) if st != "inferred" else []
            if fids:
                found.append(f"{led['methods'][k]['name']} [{st}] -> {fn_ref(led, fids[0])}")
        gaps = sorted((g for g in led["gaps"] if (g.get("vanilla") or "").startswith(cp + "#")
                       or g.get("vanilla_class") == cp or (g.get("callee") or "").startswith(cp + "#")), key=gap_rank)
        counts = Counter(status_of(led, k) for k in ks)
        out.append({
            "class": short(cp), "score": round(scores[cp], 1), "source": led["classes"][cp]["file"],
            "status": ", ".join(f"{v} {k}" for k, v in counts.most_common()),
            "files": led.get("homes", {}).get(cp) or sorted(set(led.get("file_class_cites", {}).get(cp, [])) | {led["fns"][f]["file"] for f in led["class_cites"].get(cp, [])})[:8],
            "ported_as": found[:12],
            "gaps": [gap_text(led, g) for g in [g for g in gaps if g["kind"] != "callee"][:10] + [g for g in gaps if g["kind"] == "callee"][:3]],
        })
    emit(out, as_json, "The report names no vanilla class the ledger knows; try the class or item name.")


def gap_identity(led, g):
    """A gap as the same gap in another build of the ledger: by what it is about, not by line numbers."""
    f = led["fns"][g["fn"]] if "fn" in g else {}
    return (g["kind"], g.get("vanilla"), g.get("callee"), g.get("vanilla_class"), f.get("file"), f.get("name"),
            g.get("file"), g.get("cite"))


def diff_ledgers(old, new):
    """What changed between two builds: gaps closed and opened, methods newly found or lost."""
    og = {gap_identity(old, g): g for g in old["gaps"]}
    ng = {gap_identity(new, g): g for g in new["gaps"]}
    ost = {k: status_of(old, k) for k in old["methods"]}
    nst = {k: status_of(new, k) for k in new["methods"]}
    rank = {s: i for i, s in enumerate(COVERAGE_COLS)}
    found, lost = [], []
    for k, st in nst.items():
        before = ost.get(k, "missing")
        if rank[st] < rank[before] and before in ("missing", "trivial", "inferred"):
            found.append(f"{short(new['methods'][k]['class'], new['methods'][k]['name'])}: {before} -> {st}")
        elif rank[st] > rank[before] and st in ("missing", "inferred") and before not in ("trivial",):
            lost.append(f"{short(new['methods'][k]['class'], new['methods'][k]['name'])}: {before} -> {st}")
    return {
        "gaps closed": [gap_text(old, og[k]) for k in sorted(set(og) - set(ng), key=lambda k: gap_rank(og[k]))],
        "gaps opened": [gap_text(new, ng[k]) for k in sorted(set(ng) - set(og), key=lambda k: gap_rank(ng[k]))],
        "newly found": sorted(found), "lost": sorted(lost),
    }


def cmd_diff(led, other, as_json):
    old = json.loads(Path(other).read_text())
    out = diff_ledgers(old, led)
    emit([out], as_json, "")


def changed_ranges(base):
    """New-side line ranges each Rust file changed since its merge base with `base` (uncommitted work too)."""
    mb = subprocess.run(["git", "merge-base", base, "HEAD"], cwd=ROOT, capture_output=True, text=True)
    if mb.returncode:
        sys.exit(f"No merge base with {base}: {mb.stderr.strip()}")
    diff = subprocess.run(["git", "diff", "-U0", mb.stdout.strip(), "--", "crates", "launcher"],
                          cwd=ROOT, capture_output=True, text=True).stdout
    ranges, path = defaultdict(list), None
    for line in diff.splitlines():
        if line.startswith("+++ "):
            path = line[6:] if line.startswith("+++ b/") else None
        elif line.startswith("@@") and path and path.endswith(".rs"):
            m = re.search(r"\+(\d+)(?:,(\d+))?", line)
            start, n = int(m.group(1)), int(m.group(2) or 1)
            ranges[path].append((start, start + max(n, 1) - 1))
    return mb.stdout.strip(), ranges


def build_at(rev, led):
    """The ledger of another revision, built in a throwaway worktree with the same vanilla inputs."""
    tmp = Path(tempfile.mkdtemp(prefix="port-ledger-"))
    tree = tmp / "tree"
    subprocess.run(["git", "worktree", "add", "--detach", "--quiet", str(tree), rev], cwd=ROOT, check=True)
    try:
        env = dict(os.environ, PORT_LEDGER_ROOT=str(tree), PORT_LEDGER_OUT=str(tree / ".local" / "port-ledger"))
        cmd = [sys.executable, str(Path(__file__).resolve()), "build"]
        for src in led["sources"]:
            cmd += ["--sources", src]
        cmd += [x for j in led.get("jars", []) for x in ("--jar", j)] or ["--no-jar"]
        subprocess.run(cmd, env=env, check=True, capture_output=True, text=True)
        return json.loads((tree / ".local" / "port-ledger" / "ledger.json").read_text())
    finally:
        subprocess.run(["git", "worktree", "remove", "--force", str(tree)], cwd=ROOT, capture_output=True)
        shutil.rmtree(tmp, ignore_errors=True)


def cmd_check(led, base, compare, as_json):
    mb, ranges = changed_ranges(base)
    changed = {p for p in ranges}
    newest = max(((ROOT / p).stat().st_mtime for p in changed if (ROOT / p).exists()), default=0)
    if newest > LEDGER.stat().st_mtime:
        print("The ledger is older than your changes; rebuilding...", file=sys.stderr)
        led = build(led["sources"], led.get("jars", []), os.cpu_count())
    named_for = defaultdict(list)
    for k, v in led.get("named", {}).items():
        for f in v:
            named_for[f].append(k)
    for k, v in led.get("linked", {}).items():
        named_for[v["fn"]].append(k)
    gaps_by_vanilla = defaultdict(list)
    for g in led["gaps"]:
        if g.get("vanilla"):
            gaps_by_vanilla[g["vanilla"]].append(g)
    ports, suggest, bare, stale = [], [], [], []
    for f in led["fns"]:
        spans = ranges.get(f["file"])
        if not spans or f.get("test") or not any(a <= (f["end"] or f["line"]) and f["line"] <= b for a, b in spans):
            continue
        name = lambda k: short(led["methods"][k]["class"], led["methods"][k]["name"])
        for k in f["primary"]:
            if k not in led["methods"]:
                continue
            gs = sorted(gaps_by_vanilla.get(k, []), key=gap_rank)
            gs = [g for g in gs if g["kind"] != "callee"] + [g for g in gs if g["kind"] == "callee"][:2]
            m = led["methods"][k]
            ports.append({"fn": fn_ref(led, f["id"]), "ports": f"{name(k)} ({m['file']}:{m['lines'][0][0] if m['lines'] else '?'})",
                          "open gaps": [gap_text(led, g) for g in gs[:6]]})
        if not f["cites"] and named_for.get(f["id"]):
            k = named_for[f["id"]][0]
            suggest.append(f"{fn_ref(led, f['id'])}: looks like the port of {name(k)}; open its doc with `{name(k)}`")
        elif not f["cites"] and (f["end"] or f["line"]) - f["line"] >= 12:
            bare.append(fn_ref(led, f["id"]))
    for g in led["gaps"]:
        if g["kind"] == "stale" and any(a <= g["line"] <= b for a, b in ranges.get(g["file"], ())):
            stale.append(gap_text(led, g))
    out = {"base": f"{base} ({mb[:9]})", "files changed": len(changed), "ports you touched": ports,
           "citations to add": suggest, "stale citations": stale,
           "long uncited fns (cite the vanilla method if they port one)": bare[:15]}
    if compare:
        print(f"Building the ledger at {mb[:9]} to compare...", file=sys.stderr)
        out.update(diff_ledgers(build_at(mb, led), led))
    if as_json:
        print(json.dumps(out, indent=1))
        return
    for k, v in out.items():
        if isinstance(v, list):
            if not v:
                continue
            print(f"{k}:")
            for x in v:
                if isinstance(x, dict):
                    print(f"  {x['fn']}\n    ports {x['ports']}")
                    for gg in x["open gaps"]:
                        print(f"    - {gg}")
                else:
                    print(f"  {x}")
        else:
            print(f"{k}: {v}")


REPORT_HTML = """<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Port ledger</title>
<style>
:root { --bg: #fff; --fg: #1b1b1b; --muted: #666; --line: #e3e3e3; --chip: #f2f2f2; --accent: #2457c5;
  --ok: #1d7a3a; --mid: #946200; --bad: #b3261e; }
@media (prefers-color-scheme: dark) { :root { --bg: #151515; --fg: #e8e8e8; --muted: #9a9a9a; --line: #2c2c2c;
  --chip: #232323; --accent: #8ab0ff; --ok: #6fcf8f; --mid: #e0b54a; --bad: #ff8a80; } }
* { box-sizing: border-box; }
body { margin: 0; background: var(--bg); color: var(--fg); font: 14px/1.45 ui-sans-serif, system-ui, sans-serif; }
header { padding: 20px 16px 8px; max-width: 1200px; margin: auto; }
h1 { font-size: 20px; margin: 0 0 4px; }
.sub { color: var(--muted); }
nav { display: flex; gap: 4px; margin: 14px 0 0; border-bottom: 1px solid var(--line); }
nav button { background: none; border: 0; border-bottom: 2px solid transparent; color: var(--muted); padding: 8px 12px;
  font: inherit; cursor: pointer; }
nav button.on { color: var(--fg); border-color: var(--accent); }
main { max-width: 1200px; margin: auto; padding: 12px 16px 40px; }
input, select { font: inherit; color: inherit; background: var(--chip); border: 1px solid var(--line); border-radius: 6px;
  padding: 8px 10px; }
input { width: 100%; }
.row { display: flex; gap: 8px; margin-bottom: 12px; flex-wrap: wrap; }
.row > input { flex: 1; min-width: 220px; }
.item { padding: 8px 0; border-bottom: 1px solid var(--line); overflow-wrap: anywhere; }
.k { font-weight: 600; }
.st { display: inline-block; font-size: 12px; padding: 0 6px; border-radius: 4px; background: var(--chip); margin-left: 6px; }
.ported, .mapped, .named, .mentioned { color: var(--ok); } .linked, .structural, .inferred { color: var(--mid); }
.missing, .absent { color: var(--bad); } .trivial, .skipped { color: var(--muted); }
code { font: 12.5px ui-monospace, SFMono-Regular, Menlo, monospace; color: var(--muted); }
table { border-collapse: collapse; width: 100%; font-variant-numeric: tabular-nums; }
th, td { text-align: right; padding: 5px 8px; border-bottom: 1px solid var(--line); }
th:first-child, td:first-child { text-align: left; }
th { cursor: pointer; color: var(--muted); font-weight: 500; }
.scroll { overflow-x: auto; }
.bar { height: 6px; background: var(--chip); border-radius: 3px; min-width: 60px; }
.bar i { display: block; height: 100%; background: var(--accent); border-radius: 3px; }
.more { color: var(--muted); padding: 10px 0; }
</style></head><body>
<header><h1>Port ledger</h1><div class="sub" id="summary"></div>
<nav><button data-tab="search" class="on">Find</button><button data-tab="gaps">Gaps</button><button data-tab="coverage">Coverage</button></nav></header>
<main>
<section id="search"><div class="row"><input id="q" placeholder="A vanilla class or method (EyeOfEnder, LivingEntity.travel) or a Rust fn or file (remote_use, spear.rs)" autofocus>
<select id="st"><option value="">any status</option><option>ported</option><option>mapped</option><option>mentioned</option><option>named</option><option>linked</option><option>structural</option><option>inferred</option><option>skipped</option><option>absent</option><option>trivial</option><option>missing</option></select></div><div id="results"></div></section>
<section id="gaps" hidden><div class="row"><select id="gk"></select><input id="gq" placeholder="Filter gaps"></div><div id="gapl"></div></section>
<section id="coverage" hidden><div class="scroll"><table id="cov"></table></div></section>
</main>
<script>
const D = __DATA__;
const $ = id => document.getElementById(id);
const esc = s => s.replace(/[&<>]/g, c => ({"&": "&amp;", "<": "&lt;", ">": "&gt;"})[c]);
$("summary").textContent = D.summary;
document.querySelectorAll("nav button").forEach(b => b.onclick = () => {
  document.querySelectorAll("nav button").forEach(x => x.classList.toggle("on", x === b));
  document.querySelectorAll("main section").forEach(s => s.hidden = s.id !== b.dataset.tab);
});
const hay = D.methods.map(m => (m[0] + " " + m[1] + " " + m[4].join(" ")).toLowerCase());
function search() {
  const q = $("q").value.trim().toLowerCase(), st = $("st").value, out = [];
  if (q.length < 2 && !st) { $("results").innerHTML = '<div class="more">Type at least two letters.</div>'; return; }
  const terms = q.split(/\\s+/).filter(Boolean);
  for (let i = 0; i < D.methods.length && out.length < 300; i++) {
    const m = D.methods[i];
    if (st && m[2] !== st) continue;
    if (terms.every(t => hay[i].includes(t))) out.push(m);
  }
  $("results").innerHTML = out.map(m => `<div class="item"><span class="k">${esc(m[0])}</span><span class="st ${m[2]}">${m[2]}</span>
    <div><code>${esc(m[3])}</code></div>${m[4].map(l => `<div>&rarr; <code>${esc(l)}</code></div>`).join("")}</div>`).join("")
    + (out.length >= 300 ? '<div class="more">First 300 shown; narrow the search.</div>' : out.length ? "" : '<div class="more">Nothing matches.</div>');
}
$("q").oninput = search; $("st").onchange = search;
const kinds = [...new Set(D.gaps.map(g => g[0]))];
$("gk").innerHTML = '<option value="">every kind</option>' + kinds.map(k => `<option>${k}</option>`).join("");
function gaps() {
  const k = $("gk").value, q = $("gq").value.trim().toLowerCase();
  const list = D.gaps.filter(g => (!k || g[0] === k) && (!q || g[1].toLowerCase().includes(q)));
  $("gapl").innerHTML = list.slice(0, 500).map(g => `<div class="item"><span class="st">${g[0]}</span> ${esc(g[1])}</div>`).join("")
    + `<div class="more">${list.length} gaps${list.length > 500 ? ", first 500 shown" : ""}.</div>`;
}
$("gk").onchange = gaps; $("gq").oninput = gaps; gaps();
let sortCol = 1, sortDir = -1;
function cov() {
  const rows = D.coverage.slice().sort((a, b) => (a[sortCol] > b[sortCol] ? 1 : a[sortCol] < b[sortCol] ? -1 : 0) * sortDir);
  $("cov").innerHTML = "<tr>" + D.covcols.map((c, i) => `<th data-i="${i}">${c}</th>`).join("") + "<th></th></tr>"
    + rows.map(r => "<tr>" + r.map(v => `<td>${v}</td>`).join("") + `<td><div class="bar"><i style="width:${r[r.length - 1]}%"></i></div></td></tr>`).join("");
  document.querySelectorAll("#cov th[data-i]").forEach(th => th.onclick = () => {
    const i = +th.dataset.i; sortDir = sortCol === i ? -sortDir : (i ? -1 : 1); sortCol = i; cov();
  });
}
cov(); search();
</script></body></html>
"""


def cmd_map_check(led, paths):
    """Check verdict-map files against the last build without rebuilding: every class, method and fn a row
    names must exist (several people can check their own files at once)."""
    fn_at = set()
    for f in led["fns"]:
        if not f.get("decl"):
            fn_at.add((f["file"], f.get("owner") or "", f["name"]))
            fn_at.add((f["file"], "*", f["name"]))
    classes = led["classes"]

    def has_method(cp, meth, depth=0):
        if key(cp, meth) in led["methods"]:
            return True
        return depth < 12 and any(sup in classes and has_method(sup, meth, depth + 1) for sup in classes.get(cp, {}).get("extends", []))

    problems, rows = [], 0
    for path in paths:
        for n, line in enumerate(Path(path).read_text(encoding="utf-8").splitlines(), 1):
            if not line.strip() or line.lstrip().startswith("#"):
                continue
            rows += 1
            parts = [x.strip() for x in line.split("\t")] + ["", "", ""]
            name, verdict, where = parts[0], parts[1], parts[2]
            if "#" in name:
                if name not in led["methods"]:
                    problems.append(f"{path}:{n} {name}: no method {name}")
                continue
            head, _, last = name.rpartition(".")
            cls_q, meth = (head, last) if head and (last[:1].islower() or last.startswith("<")) else (name, None)
            q = cls_q.replace(".", "$")
            cands = [p for p in classes if p.endswith("/" + q) or p.rsplit("/", 1)[-1] == q]
            where_ok = True
            if verdict in ("port", "inline"):
                file_, _, rest = where.partition("::")
                owner, _, fname = rest.rpartition("::")
                where_ok = (file_, owner, fname) in fn_at or (file_, "*", fname) in fn_at
            if len(cands) != 1:
                problems.append(f"{path}:{n} {name}: {'no' if not cands else 'ambiguous'} class {cls_q}")
            elif meth and not has_method(cands[0], meth):
                problems.append(f"{path}:{n} {name}: no method {meth} on {cls_q}")
            elif not (verdict in ("port", "inline", "missing") or verdict.startswith("skip:") and verdict[5:] in SKIP_REASONS
                      or verdict == "home" and not meth):
                problems.append(f"{path}:{n} {name}: unknown verdict {verdict!r}")
            elif meth is None and verdict in ("port", "inline"):
                problems.append(f"{path}:{n} {name}: a class row takes skip:<reason>, missing or home")
            elif not where_ok:
                problems.append(f"{path}:{n} {name}: no fn {where}")
    print("\n".join(problems) if problems else f"{rows} rows, no problems.")


def todo_batch(led, package=None, limit=150, start_after=None):
    """The next methods with no answer, whole classes at a time, in package order: what a mapper reads next."""
    by_class = defaultdict(list)
    for k, m in led["methods"].items():
        if m["class"].startswith(SKIP_PACKAGES) or m.get("anonymous") or m["name"] == "<clinit>":
            continue
        if package and not m["class"].startswith("net/minecraft/" + package.strip("/") + "/"):
            continue
        if status_of(led, k) == "missing":
            by_class[m["class"]].append(k)
    out, n = [], 0
    for cp in sorted(by_class):
        if start_after and cp <= start_after:
            continue
        if n and n + len(by_class[cp]) > limit:
            break
        out.append((cp, sorted(by_class[cp], key=lambda k: (led["methods"][k]["lines"] or [[0]])[0][0] or 0)))
        n += len(by_class[cp])
    return out


def cmd_todo(led, package, limit, start_after, as_json):
    homes = led.get("homes", {})
    by_file = defaultdict(list)
    for f in led["fns"]:
        if not f.get("test") and not f.get("decl"):
            by_file[f["file"]].append(f)
    batch = todo_batch(led, package, limit, start_after)
    rows = []
    for cp, ks in batch:
        home = homes.get(cp) or homes.get(cp.split("$")[0]) or []
        methods = []
        for k in ks:
            m = led["methods"][k]
            b = snake_case(m["name"]).removeprefix("get_")
            near = sorted({fn_ref(led, f["id"]) for file_ in home for f in by_file[file_]
                           if name_similarity(b, f["name"]) >= 0.34})[:4]
            methods.append({"method": m["name"], "line": (m["lines"] or [[None]])[0][0], "size": m["size"],
                            "callers": len(led["callers"].get(k, [])),
                            "hints": near + [fn_ref(led, i["fn"]) + " (shares ids/numbers)" for i in led["inferred"].get(k, [])[:2]]})
        rows.append({"class": short(cp), "path": cp, "source": led["classes"][cp]["file"], "home": home, "methods": methods})
    if as_json:
        print(json.dumps(rows, indent=1))
        return
    if not rows:
        print("Nothing left to answer there.")
        return
    total = sum(len(r["methods"]) for r in rows)
    print(f"# {total} methods in {len(rows)} classes; next batch: --after {rows[-1]['path']}")
    for r in rows:
        print(f"\n## {r['class']}  ({r['source']})")
        print("home: " + (", ".join(r["home"]) if r["home"] else "none found"))
        for mm in r["methods"]:
            hint = ("  hints: " + "; ".join(mm["hints"])) if mm["hints"] else ""
            print(f"- {mm['method']} :{mm['line']} size {mm['size']} callers {mm['callers']}{hint}")


def cmd_report(led):
    lines = ["# Port ledger", "", f"Vanilla sources: {', '.join(led['sources'])}", ""]
    rows = coverage_rows(led, None, 2)
    tot = Counter()
    for c in rows.values():
        tot.update(c)
    lines += [f"{tot['total']} vanilla methods outside the skipped packages: {tot['ported']} ported by a fn whose doc opens "
              f"with them, {tot['named']} by a same-named fn (in a file citing the class, an impl of it or a file named "
              f"for it), {tot['mentioned']} named in a comment, {tot['linked']} linked through the call graph, "
              f"{tot['inferred']} matched by shared ids and numbers, {tot['trivial']} too small to port alone, "
              f"{tot['mapped']} mapped and {tot['skipped']} skipped and {tot['absent']} confirmed missing in tools/port_map, "
              f"{tot['structural']} matched by the state and calls they share, "
              f"{tot['missing']} with no answer yet ({found_pct(tot):.0f}% of those worth porting found, "
              f"{classified_pct(tot):.0f}% of all answered).", "",
              "## Coverage by package", "",
              "| Package | Methods | " + " | ".join(k.capitalize() for k in COVERAGE_COLS) + " | Found % |",
              "|---|---:|" + "---:|" * len(COVERAGE_COLS) + "---:|"]
    for p, c in sorted(rows.items(), key=lambda kv: -kv[1]["total"])[:80]:
        lines.append(f"| {p} | {c['total']} | " + " | ".join(str(c[k]) for k in COVERAGE_COLS) + f" | {found_pct(c):.0f} |")
    counts = Counter(g["kind"] for g in led["gaps"])
    lines += ["", "## Gaps", "", ", ".join(f"{k}: {v}" for k, v in counts.most_common()), ""]
    for kind, title in (("seam", "Ported on both paths (in-frame and server-client), wired on only one"),
                        ("client", "Client-only effects the server-client path never reaches"),
                        ("drift", "Ports that never name a sound, particle, item or constant vanilla uses"),
                        ("class", "Whole classes nothing ports, next to ported neighbours"),
                        ("callee", "Ported callers that never reach a ported callee"),
                        ("stale", "Citations the pinned version doesn't have")):
        items = sorted((g for g in led["gaps"] if g["kind"] == kind), key=gap_rank)
        if not items:
            continue
        lines += [f"### {title} ({len(items)})", ""]
        lines += ["- " + gap_text(led, g).split(": ", 1)[1] for g in items[:300]]
        lines.append("")
    lines += ["## Most-called vanilla methods with no port", ""]
    lines += [f"- {u['vanilla']} ({u['callers']} callers, {u['status']})" for u in unported(led, None, 150)]
    (OUT / "report.md").write_text("\n".join(lines) + "\n")
    write_html(led, rows, tot)
    print(OUT / "report.md")
    print(OUT / "report.html")


def write_html(led, rows, tot):
    """report.html: one page to look anything up in, offline, no server."""
    def locs(k):
        fids = port_of(led, k)
        return [fn_ref(led, f) for f in fids[:3]] or [fn_ref(led, f) + " (mention)" for f in led["mentioned"].get(k, [])[:2]]
    methods = []
    for k, m in led["methods"].items():
        if m["class"].startswith(SKIP_PACKAGES) or m["name"] == "<clinit>" or m.get("anonymous"):
            continue
        methods.append([short(m["class"], m["name"]), m["class"], status_of(led, k),
                        f"{m['file']}:{m['lines'][0][0] if m['lines'] else '?'}", locs(k)])
    methods.sort(key=lambda r: (COVERAGE_COLS.index(r[2]) if r[2] in COVERAGE_COLS else 9, r[0]))
    gaps = [[g["kind"], gap_text(led, g).split(": ", 1)[1]] for g in sorted(led["gaps"], key=gap_rank)]
    covcols = ["package", "methods"] + list(COVERAGE_COLS) + ["found %"]
    coverage = [[p, c["total"]] + [c[k] for k in COVERAGE_COLS] + [found_pct(c)]
                for p, c in sorted(rows.items(), key=lambda kv: -kv[1]["total"])]
    data = {"summary": f"{len(led['methods'])} vanilla methods, {len(led['fns'])} Rust functions; "
                       f"{found_pct(tot):.0f}% of the methods worth porting have a port found; {len(led['gaps'])} gaps.",
            "methods": methods, "gaps": gaps, "covcols": covcols, "coverage": coverage}
    payload = json.dumps(data, separators=(",", ":")).replace("</", "<\\/")
    (OUT / "report.html").write_text(REPORT_HTML.replace("__DATA__", payload))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--json", action="store_true", help="machine-readable output")
    sub = parser.add_subparsers(dest="cmd", required=True)
    b = sub.add_parser("build")
    b.add_argument("--sources", action="append", help="a directory of decompiled vanilla Java (repeatable)")
    b.add_argument("--jar", action="append", help="a vanilla game jar (repeatable; default: the lab's or the launcher's)")
    b.add_argument("--no-jar", action="store_true", help="read the sources only")
    b.add_argument("--jobs", type=int, default=os.cpu_count())
    sub.add_parser("where").add_argument("query")
    sub.add_parser("what").add_argument("location")
    sub.add_parser("class").add_argument("query")
    c = sub.add_parser("coverage")
    c.add_argument("--package")
    c.add_argument("--depth", type=int, default=2)
    g = sub.add_parser("gaps")
    g.add_argument("--kind", choices=["seam", "client", "drift", "class", "callee", "stale", "map", "unported", "untested"])
    g.add_argument("--package")
    g.add_argument("--limit", type=int, default=50)
    sub.add_parser("report")
    mc = sub.add_parser("map-check", help="check verdict-map files against the last build, without rebuilding")
    mc.add_argument("paths", nargs="+")
    t = sub.add_parser("todo", help="the next methods with no answer, for the verdict map (tools/port_map)")
    t.add_argument("--package", help="e.g. world/entity/animal")
    t.add_argument("--limit", type=int, default=150, help="about this many methods, whole classes at a time")
    t.add_argument("--after", help="start after this class path (the last batch's last class)")
    d = sub.add_parser("diff", help="what changed against another ledger.json")
    d.add_argument("other")
    ck = sub.add_parser("check", help="what a branch's changes touch: ports, their open gaps, citations to add")
    ck.add_argument("--base", default="origin/main")
    ck.add_argument("--no-compare", action="store_true", help="skip building the base to list gaps opened and closed")
    sub.add_parser("issue", help="the vanilla classes, Rust code and gaps a bug report points at").add_argument(
        "ref", help=f"an issue number on {ISSUE_REPO}, the report's text, or - to read it from stdin")
    args = parser.parse_args()
    if args.cmd == "build":
        jars = [] if args.no_jar else (args.jar or default_jars())
        led = build(args.sources or default_sources(), jars, args.jobs)
        rows = coverage_rows(led)
        tot = Counter()
        for r in rows.values():
            tot.update(r)
        print(f"{len(led['methods'])} vanilla methods, {len(led['fns'])} Rust functions; of {tot['total']} outside the "
              f"skipped packages: " + ", ".join(f"{tot[k]} {k}" for k in COVERAGE_COLS)
              + f" ({found_pct(tot):.0f}% of those worth porting found, {classified_pct(tot):.0f}% of all answered;"
              f" {len(led.get('harness', {}))} covered by a parity harness); gaps: "
              + ", ".join(f"{k} {v}" for k, v in Counter(gg['kind'] for gg in led['gaps']).most_common())
              + f" -> {LEDGER}")
        return
    led = load()
    if args.cmd == "where":
        cmd_where(led, args.query, args.json)
    elif args.cmd == "what":
        cmd_what(led, args.location, args.json)
    elif args.cmd == "class":
        cmd_class(led, args.query, args.json)
    elif args.cmd == "coverage":
        cmd_coverage(led, args.package, args.depth, args.json)
    elif args.cmd == "gaps":
        cmd_gaps(led, args.kind, args.package, args.limit, args.json)
    elif args.cmd == "report":
        cmd_report(led)
    elif args.cmd == "issue":
        cmd_issue(led, args.ref, args.json)
    elif args.cmd == "diff":
        cmd_diff(led, args.other, args.json)
    elif args.cmd == "map-check":
        cmd_map_check(led, args.paths)
    elif args.cmd == "todo":
        cmd_todo(led, args.package, args.limit, args.after, args.json)
    elif args.cmd == "check":
        cmd_check(led, args.base, not args.no_compare, args.json)


if __name__ == "__main__":
    main()
