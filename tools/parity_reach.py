"""Which of a harness's Rust sources its probes can reach, for `parity.py --changed`.

Cargo's dep-info names every source file of every crate a probe links, so a change anywhere in horizon-client
selected every harness whose probe links horizon-client. This module narrows a harness's recorded dependencies to
the files holding an item its entry points (the probe modules it ran, the binaries it built) can reach, over a graph
of the workspace's current sources whose nodes are items:

- everything in an entry is reached, and so is everything in a Rust file the module tree doesn't place (an
  `include!`d source);
- a module is open when it holds reached code, or is named in a path (`name::`, `use ...name`) by reached code while
  its parent is open; a library's root once reached code names its crate (`horizon_world`);
- a free item (fn, struct, enum, union, trait, type, const, static, an item-level macro call's items) is reached when
  reached code in its own module or below it mentions its name, or when its module is open and reached code names
  it in a path or an import, or when its module is glob-imported (`use m::*`) and its name is mentioned;
- an inherent impl's methods and constants are reached one by one, by name, once their type is; a trait impl once
  its trait and its type are (a std or outside name always is, a generic parameter needs nothing); a macro once
  invoked; a `pub use` like a free item named by everything it imports;
- "a crate's reached code" covers the crates it names (`horizon_world::`), directly or through one that does, and
  code under `#[cfg(test)]` or `#[test]` is left out (probes are never test builds).

Names match by spelling, so a file can be kept for a name it never meant: the graph only over-approximates. A
reached file's parent modules count through their `mod` declarations of it (parity.py compares those with the merge
base). Everything that isn't a workspace Rust source stays a dependency, except a crate's non-Rust files (shaders,
assets, fonts) that only unreached sources name; a dependency that no longer exists counts as changed, and Rust files
a change adds take part.
"""

import functools
import hashlib
from pathlib import Path
import re
import subprocess

IDENT = r"[A-Za-z_]\w*"
WORD = re.compile(rf"\b({IDENT})\b")
MOD_DECLARATION = re.compile(rf"\bmod\s+({IDENT})\s*;")
INLINE_MOD = re.compile(rf"\bmod\s+({IDENT})\s*\{{")
PATH_ATTRIBUTE = re.compile(r'#\[\s*path\s*=\s*"([^"]+)"\s*\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*;$')
BRACE = re.compile(r"[{}]")
TEST_ONLY = re.compile(r"#\[\s*(?:cfg\s*\(\s*test\s*\)|test)\s*\]")
TEST_MODULE = re.compile(r"#\[\s*cfg\s*\(\s*test\s*\)\s*\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*;$")
SPLIT = re.compile(r"[{}()\[\];]")
ITEM = re.compile(rf"(?:pub(?:\s*\([^)]*\))?\s+)?(?:(?:unsafe|async|const|extern|default|auto)\s+)*"
                  rf"(fn|struct|enum|union|trait|type|const|static|impl|mod|use|macro_rules\s*!|extern\s+crate)"
                  rf"(?!\w)\s*(?:mut\s+)?({IDENT})?")
PATH_BEFORE = re.compile(rf"\b({IDENT})\s*::")
PATH_AFTER = re.compile(rf"::\s*({IDENT})")
MACRO_USE = re.compile(rf"\b({IDENT})\s*!\s*[(\[{{]")
MACRO_CALL = re.compile(rf"({IDENT}(?:\s*::\s*{IDENT})*)\s*!")
DEFINED_INSIDE = re.compile(rf"\b(?:fn|struct|enum|union|trait|type|const|static)\s+({IDENT})")
NESTED_IMPL = re.compile(r"^[ \t]*(?:unsafe\s+)?impl\b([^{;]*)\{", re.MULTILINE)
QUALIFIED = re.compile(rf"{IDENT}(?:\s*::\s*{IDENT})*")
GLOB_IMPORT = re.compile(r"\buse\s[^;]*::\s*\*")
# File names a source may name in a literal (include_str!, include_bytes!, a path it reads). A file of another kind
# is never "named", so it always counts.
FILE_NAME = re.compile(r"[\w-]+\.(?:wgsl|glsl|json|png|ttf|otf|svg|txt|toml|ogg|wav|nbt|bin|dat|csv|mcmeta|wit|wasm)\b")
TYPE_KINDS = {"struct", "enum", "union", "trait", "type", "macro_call"}
# Names an impl header can take from outside the workspace: std's prelude and primitive types, and the std and serde
# traits a glob import may bring. A glob-imported name outside this list is taken to be the workspace's.
STD_NAMES = {"Display", "Error", "Read", "Write", "Seek", "BufRead", "Deref", "DerefMut", "Index", "IndexMut", "Add",
             "Sub", "Mul", "Div", "Rem", "Neg", "Not", "BitAnd", "BitOr", "BitXor", "Shl", "Shr", "AddAssign",
             "SubAssign", "MulAssign", "DivAssign", "RemAssign", "BitAndAssign", "BitOrAssign", "BitXorAssign",
             "Borrow", "BorrowMut", "Future", "Any", "FromStr", "Hasher", "BuildHasher", "Formatter", "Ordering",
             "Serialize", "Deserialize", "Serializer", "Deserializer", "Visitor", "Termination", "Sum", "Product",
             "Wake", "Cow", "Rc", "Arc", "Mutex", "RwLock", "Cell", "RefCell", "HashMap", "HashSet", "BTreeMap",
             "BTreeSet", "VecDeque"}
PRELUDE = {"Option", "Some", "None", "Result", "Ok", "Err", "Vec", "String", "Box", "ToString", "ToOwned", "Clone",
           "Copy", "Send", "Sync", "Sized", "Unpin", "Drop", "Fn", "FnMut", "FnOnce", "From", "Into", "TryFrom",
           "TryInto", "AsRef", "AsMut", "Default", "Eq", "PartialEq", "Ord", "PartialOrd", "Iterator", "Extend",
           "IntoIterator", "DoubleEndedIterator", "ExactSizeIterator", "FromIterator", "Debug", "Hash",
           "bool", "char", "str", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128",
           "isize", "f32", "f64"}
STD_NAMES |= PRELUDE
KEYWORDS = {"crate", "self", "super", "Self", "pub", "in", "as", "use", "for", "where", "dyn", "mut", "const", "fn",
            "unsafe", "impl", "std", "core", "alloc", "let", "if", "else", "match", "return", "struct", "enum", "mod",
            "type", "static", "trait", "true", "false", "loop", "while", "move", "ref", "break", "continue", "async",
            "await", "extern", "union"}


def segments(code, start, end):
    """The item-level pieces of code[start:end]: each ends at a `;` or a `}` outside any bracket."""
    found, begin, depth = [], start, 0
    for match in SPLIT.finditer(code, start, end):
        character = match.group()
        if character in "{([":
            depth += 1
        elif character in "})]":
            depth -= 1
            if character == "}" and depth == 0:
                found.append((begin, match.end()))
                begin = match.end()
        elif depth == 0:
            found.append((begin, match.end()))
            begin = match.end()
    if code[begin:end].strip():
        found.append((begin, end))
    return found


def skip_attributes(code, index, end):
    """The offset of the first non-space character after the attributes (`#[...]`, `#![...]`) starting at index."""
    while True:
        while index < end and code[index].isspace():
            index += 1
        if not code.startswith("#", index) or not re.match(r"#!?\[", code[index:index + 3]):
            return index
        depth = 0
        for position in range(code.index("[", index), end):
            depth += {"[": 1, "]": -1}.get(code[position], 0)
            if depth == 0:
                index = position + 1
                break
        else:
            return end


def body_of(code, start, end):
    """The range inside an item's outermost `{ }` block, or None."""
    opening = code.find("{", start, end)
    closing = code.rfind("}", start, end)
    return (opening + 1, closing) if 0 <= opening < closing else None


def words(text):
    return set(WORD.findall(text)) - KEYWORDS


def qualified_words(text):
    """Names a piece of code reaches through a path (`a::b`: b), the way items of other modules are named."""
    return set(PATH_AFTER.findall(text)) - KEYWORDS


def path_words(text):
    """Names a piece of code uses as path segments (`a::`, `::b`), where module names appear."""
    return set(PATH_BEFORE.findall(text)) | set(PATH_AFTER.findall(text))


def parse_items(code, start, end, items):
    """Append the items of code[start:end] (a file or an inline module's body) to items, as dicts."""
    for begin, finish in segments(code, start, end):
        head = skip_attributes(code, begin, finish)
        text = code[head:finish]
        if not text.strip(" \n\t;") or TEST_ONLY.search(code[begin:head]):
            continue  # compiled into tests only, never into a probe
        match = ITEM.match(text)
        kind = re.sub(r"\s+", "", match.group(1)) if match else None
        name = match.group(2) if match else None
        if kind == "mod":
            body = body_of(code, head, finish)
            if body:
                parse_items(code, body[0], body[1], items)
            continue
        if kind in ("externcrate", None) and not MACRO_CALL.match(text):
            if kind is None:
                items.append({"kind": "glue", "mentions": sorted(words(text)), "paths": sorted(path_words(text)), "qualified": sorted(qualified_words(text)), "invokes": sorted(set(MACRO_USE.findall(text)))})
            continue
        if kind is None:  # an item-level macro call: it may define items
            items.append({"kind": "macro_call", "names": sorted(set(DEFINED_INSIDE.findall(text))),
                          "macro": MACRO_CALL.match(text).group(1).split("::")[-1].strip(),
                          "mentions": sorted(words(text)), "paths": sorted(path_words(text)), "qualified": sorted(qualified_words(text)), "invokes": sorted(set(MACRO_USE.findall(text)))})
        elif kind == "use":
            head_word = re.match(rf"\s*(?:::\s*)?({IDENT})", text[match.end(1):])
            items.append({"kind": "use", "public": text.startswith("pub"), "names": sorted(words(text)),
                          "head": head_word.group(1) if head_word else "", "glob": bool(re.search(r"::\s*\*", text)),
                          "mentions": sorted(words(text)), "paths": sorted(words(text)), "qualified": sorted(words(text)),
                          "invokes": []})
        elif kind == "impl":
            body = body_of(code, head, finish)
            subject = code[head + match.end(1) - match.start(): body[0] - 1 if body else finish]
            inherent = not re.search(r"\bfor\b", impl_subject(subject)[1])
            if inherent and body:
                members = []
                for member_begin, member_end in segments(code, body[0], body[1]):
                    member = code[skip_attributes(code, member_begin, member_end):member_end]
                    found = ITEM.match(member)
                    members.append({"name": found.group(2) if found else None, "mentions": sorted(words(member)),
                                    "paths": sorted(path_words(member)), "qualified": sorted(qualified_words(member)), "invokes": sorted(set(MACRO_USE.findall(member)))})
                items.append({"kind": "impl", "subject": subject, "inherent": True, "members": members,
                              "mentions": sorted(words(subject)), "paths": sorted(path_words(subject)), "qualified": sorted(qualified_words(subject)), "invokes": sorted(set(MACRO_USE.findall(subject)))})
            else:
                items.append({"kind": "impl", "subject": subject, "inherent": False, "mentions": sorted(words(text)),
                              "paths": sorted(path_words(text)), "qualified": sorted(qualified_words(text)), "invokes": sorted(set(MACRO_USE.findall(text)))})
        else:
            kind = "macro" if kind.startswith("macro_rules") else kind
            items.append({"kind": kind, "names": [name] if name else [], "mentions": sorted(words(text)),
                          "defines": sorted(set(DEFINED_INSIDE.findall(text))) if kind == "macro" else [],
                          "paths": sorted(path_words(text)), "qualified": sorted(qualified_words(text)), "invokes": sorted(set(MACRO_USE.findall(text)))})
            if kind in ("fn", "const", "static"):
                # An impl inside a function body still holds for the whole program.
                for header in NESTED_IMPL.findall(text):
                    items.append({"kind": "impl", "subject": header, "inherent": False,
                                  "mentions": sorted(words(text)), "paths": sorted(path_words(text)), "qualified": sorted(qualified_words(text)), "invokes": sorted(set(MACRO_USE.findall(text)))})


def library_root(crate_dir):
    """Cargo's library source, including an explicit path outside the package directory."""
    manifest = crate_dir / "Cargo.toml"
    text = manifest.read_text(encoding="utf-8") if manifest.is_file() else ""
    table = re.search(r"^\[lib\]([^\[]*)", text, re.M)
    path = re.search(r'^path\s*=\s*"([^"\n]+)"', table.group(1), re.M) if table else None
    return (crate_dir / (path.group(1) if path else "src/lib.rs")).resolve()


def crate_roots(crate_dir):
    """A crate's root files: its library, binaries and examples."""
    path = crate_dir / "Cargo.toml"
    manifest = path.read_text(encoding="utf-8") if path.is_file() else ""
    roots = [library_root(crate_dir)]
    if not re.search(r"^autobins\s*=\s*false", manifest, re.M):
        roots.append(crate_dir / "src" / "main.rs")
        folder = crate_dir / "src" / "bin"
        roots += sorted(folder.glob("*.rs")) + sorted(folder.glob("*/main.rs"))
    if not re.search(r"^autoexamples\s*=\s*false", manifest, re.M):
        folder = crate_dir / "examples"
        roots += sorted(folder.glob("*.rs")) + sorted(folder.glob("*/main.rs"))
    for _, table in re.findall(r"^\[\[(bin|example)\]\]([^\[]*)", manifest, re.M):
        path = re.search(r'^path\s*=\s*"([^"\n]+)"', table, re.M)
        if path:
            roots.append(crate_dir / path.group(1))
    return [root.resolve() for root in roots if root.is_file()]



class SourceGraph:
    """The workspace's Rust sources, parsed once: what each file names, defines and declares."""

    def __init__(self, root, rust_code, cache=None):
        """cache: a dict of earlier parses by content digest, which the graph reuses and adds to (parity.py keeps it
        in .local/parity-runner/reach-cache.json)."""
        self.root = Path(root)
        self.rust_code = rust_code
        self.cache = {} if cache is None else cache
        self.used = set()
        self.files = {}
        self.parent = {}  # module file -> the file declaring it
        self.module_name = {}
        self.roots = set()
        self.library_roots = {manifest.parent.name: self.relative(library_root(manifest.parent))
                              for manifest in (self.root / "crates").glob("*/Cargo.toml")}
        self.target_cache = {}
        self.workspace_heads = None
        self.test_only = set()  # modules compiled into tests only
        self.tables = None
        self.root_of = {}  # source -> the root of its target (lib.rs, a bin's or an example's main file)
        self.results = {}  # (entries, universe) -> reachable files
        for manifest in sorted((self.root / "crates").glob("*/Cargo.toml")):
            for crate_root in crate_roots(manifest.parent):
                path = self.relative(crate_root)
                self.roots.add(path)
                self.root_of[path] = path
                self.place(path, crate_root.parent, set())

    def entries(self, artifacts, rust):
        """The root files of the probes and binaries a harness built (its record's artifacts), among its Rust
        dependencies: for a crate's probe binary, the probe modules it ran (parity.only_probes_run kept only those).
        None when an artifact can't be placed."""
        found = set()
        for kind, name, *rest in artifacts:
            selection = rest[1] if len(rest) > 1 else []
            packages = [value for flag, value in zip(selection, selection[1:]) if flag == "-p"] or \
                [manifest.parent.name for manifest in sorted((self.root / "crates").glob("*/Cargo.toml"))]
            roots = set()
            if name and not any(name in self.targets(self.root / "crates" / package) for package in packages):
                owners = [manifest.parent.name for manifest in (self.root / "crates").glob("*/Cargo.toml")
                          if self.targets(manifest.parent).get(name, (None,))[0] == kind.rstrip("s")]
                if len(owners) == 1:
                    packages = [*packages, owners[0]]
            if not name and kind in ("bins", "examples") and "horizon-client" in packages \
                    and "horizon-client-lib" not in packages \
                    and (self.root / "crates/horizon-client-lib/Cargo.toml").is_file():
                # Broad records predate the client package split too.
                packages = [*packages, "horizon-client-lib"]
            for package in packages:
                crate = self.root / "crates" / package
                targets = self.targets(crate)
                if kind in ("example", "examples"):
                    candidates = [path for target, (target_kind, path) in targets.items()
                                  if target_kind == "example" and (not name or target == name)]
                else:
                    candidates = [path for target, (target_kind, path) in targets.items()
                                  if target_kind == "bin" and (not name or target == name)]
                for path in candidates:
                    if path.endswith("/examples/probe/main.rs"):
                        # The probes it ran, and the dispatch and support modules they use (only_probes_run kept
                        # those); the binary's root, which only declares them, so their own submodules are placed.
                        folder = path.removesuffix("main.rs")
                        roots |= {dependency for dependency in rust if dependency.startswith(folder)
                                  and "/" not in dependency.removeprefix(folder).removeprefix("support/")}
                        if roots:
                            roots.add(path)
                    elif path in rust:
                        roots.add(path)
            if not roots:
                return None
            found |= roots
        return found or None

    def targets(self, crate):
        """A crate's bins and examples: {target name: (kind, root file)}, from its Cargo.toml and Cargo's defaults."""
        if crate in self.target_cache:
            return self.target_cache[crate]
        found = {}
        manifest = (crate / "Cargo.toml").read_text(encoding="utf-8") if (crate / "Cargo.toml").is_file() else ""
        package = re.search(r'^\[package\][^\[]*?^name\s*=\s*"([^"]+)"', manifest, re.MULTILINE | re.DOTALL)
        if (crate / "src" / "main.rs").is_file() and package and not re.search(r"^autobins\s*=\s*false", manifest, re.M):
            found[package.group(1)] = ("bin", self.relative(crate / "src" / "main.rs"))
        for kind, folder in (("bin", crate / "src" / "bin"), ("example", crate / "examples")):
            if re.search(rf"^auto{kind}s\s*=\s*false", manifest, re.M):
                continue
            for path in sorted(folder.glob("*.rs")) + sorted(folder.glob("*/main.rs")):
                found[path.stem if path.name != "main.rs" else path.parent.name] = (kind, self.relative(path))
        for kind, table in re.findall(r"^\[\[(bin|example)\]\]([^\[]*)", manifest, re.MULTILINE):
            name = re.search(r'^name\s*=\s*"([^"]+)"', table, re.MULTILINE)
            path = re.search(r'^path\s*=\s*"([^"]+)"', table, re.MULTILINE)
            if name and path and (crate / path.group(1)).is_file():
                found[name.group(1)] = (kind, self.relative(crate / path.group(1)))
        self.target_cache[crate] = found
        return found

    def target(self, path):
        """The Cargo target a source belongs to: the root file of the library, bin or example whose module tree
        holds it (a file nothing places is its own)."""
        return self.root_of.get(path, path)

    def is_library(self, path):
        return self.target(path) in self.library_roots.values()

    def relative(self, path):
        return Path(path).resolve().relative_to(self.root.resolve()).as_posix()

    def place(self, path, module_dir, seen):
        """Walk the module tree from a file: record each declared module's file, parent and name."""
        if path in seen:
            return
        seen.add(path)
        parsed = self.parse(path)
        if parsed is None:
            return
        for name, nesting, attribute, test in parsed["declarations"]:
            if attribute is not None:
                if nesting:
                    continue  # a #[path] inside an inline module: unplaced, so always kept
                candidates = [(self.root / path).parent / attribute]
            else:
                folder = module_dir.joinpath(*nesting)
                candidates = [folder / f"{name}.rs", folder / name / "mod.rs"]
            for candidate in candidates:
                if candidate.is_file():
                    child = self.relative(candidate)
                    self.parent.setdefault(child, path)
                    self.module_name.setdefault(child, name)
                    self.root_of.setdefault(child, self.root_of[path])
                    if test or path in self.test_only:
                        self.test_only.add(child)
                    child_dir = candidate.parent if candidate.name == "mod.rs" or attribute is not None \
                        else candidate.parent / candidate.stem
                    self.place(child, child_dir, seen)
                    break

    def parse(self, path):
        if path in self.files:
            return self.files[path]
        try:
            data = (self.root / path).read_bytes()
        except OSError:
            self.files[path] = None
            return None
        digest = hashlib.sha256(data).hexdigest()
        self.used.add(digest)
        if digest in self.cache:
            self.files[path] = parsed = self.cache[digest]
            return parsed
        raw = data.decode("utf-8", errors="replace")
        code = self.rust_code(raw)  # same length as raw: literals and comments blanked
        opening = {match.end() - 1: match.group(1) for match in INLINE_MOD.finditer(code)}
        events = sorted([(match.start(), match) for match in BRACE.finditer(code)]
                        + [(match.start(), match) for match in MOD_DECLARATION.finditer(code)], key=lambda event: event[0])
        declarations, stack, depth = [], [], 0
        for index, match in events:
            if match.group() == "{":
                depth += 1
                if index in opening:
                    stack.append((opening[index], depth))
            elif match.group() == "}":
                if stack and stack[-1][1] == depth:
                    stack.pop()
                depth -= 1
            else:
                window = " ".join(raw[max(0, index - 300):match.end()].split())
                attribute = PATH_ATTRIBUTE.search(window)
                declarations.append([match.group(1), [name for name, _ in stack], attribute.group(1) if attribute else None,
                                     bool(TEST_MODULE.search(window))])
        items = []
        # An extern-crate alias still links its source crate, even when callers use the alias.
        for match in re.finditer(r"\bextern\s+crate\s+(\w+)(?:\s+as\s+\w+)?\s*;", code):
            items.append({"kind": "glue", "names": [], "mentions": [match.group(1)],
                          "paths": [match.group(1)], "qualified": [], "invokes": []})
        parse_items(code, 0, len(code), items)
        self.files[path] = parsed = {
            "declarations": declarations,
            "items": items,
            "glob": bool(GLOB_IMPORT.search(code)),
            "file_names": sorted(set(FILE_NAME.findall(raw))),
        }
        self.cache[digest] = parsed
        return parsed

    def saved_cache(self):
        """The parses of the files this graph read, to keep for the next run."""
        return {digest: self.cache[digest] for digest in self.used if digest in self.cache}

    def requirement(self, path, item):
        """For an impl: for its trait and for its type, [names the file defines, other workspace names], one of
        which must be a reached type or trait (of this file, of a crate it sees) before the impl can matter; both
        empty: nothing of the workspace is needed. A name is the workspace's when the path naming it starts in the
        workspace, or the file defines it, imports it from the workspace or may glob-import it; otherwise (std's,
        another crate's, the prelude's) it needs nothing."""
        if "requires" in item:
            return item["requires"]
        if self.workspace_heads is None:
            self.workspace_heads = {"crate", "self", "super"} | set(self.module_name.values()) | {
                manifest.parent.name.replace("-", "_") for manifest in (self.root / "crates").glob("*/Cargo.toml")}
        parsed = self.parse(path)
        defined = {name for other in parsed["items"] for name in other.get("names", ()) if other["kind"] != "use"}
        workspace_imports = {name for other in parsed["items"] if other["kind"] == "use"
                             and other["head"] in self.workspace_heads for name in other["names"]}
        parameters, subject = impl_subject(item["subject"])
        parts = []
        for part in re.split(r"\bfor\b", subject, maxsplit=1):
            names, own = set(), set()
            for match in QUALIFIED.finditer(part):
                segments_ = [segment.strip() for segment in match.group().split("::")]
                name = segments_[-1]
                if name in parameters or name in KEYWORDS:
                    continue
                if len(segments_) > 1:
                    if segments_[0] in self.workspace_heads:
                        names.add(name)
                elif name in defined:
                    own.add(name)
                elif name in workspace_imports or (parsed["glob"] and name not in STD_NAMES):
                    names.add(name)
            parts.append([sorted(own), sorted(names)])
        item["requires"] = parts
        return parts

    def index(self):
        """Lookup tables over every placed source, built once: which crates each crate sees, the nodes (items, an
        inherent impl's members) by the names they define and the types they need, the modules by name."""
        if self.tables is not None:
            return self.tables
        idents = {manifest.parent.name.replace("-", "_"): manifest.parent.name
                  for manifest in (self.root / "crates").glob("*/Cargo.toml")}
        paths = [path for path in self.files if self.files[path] is not None and path not in self.test_only]
        sees = {}
        for path in paths:
            named = {word for item in self.files[path]["items"] for word in item["mentions"]} & idents.keys()
            sees.setdefault(crate_of(path), {crate_of(path)}).update(idents[name] for name in named)
        changed = True
        while changed:
            changed = False
            for crate, seen in sees.items():
                wider = set().union(*(sees.get(other, {other}) for other in seen))
                if not wider <= seen:
                    seen |= wider
                    changed = True
        seen_by = {}  # crate -> the crates that see it
        for crate, seen in sees.items():
            for other in seen:
                seen_by.setdefault(other, set()).add(crate)
        # What a call of a workspace macro defines: what its definition's body defines.
        expands = {}
        for path in paths:
            for item in self.files[path]["items"]:
                if item["kind"] == "macro":
                    for name in item["names"]:
                        expands.setdefault(name, set()).update(item["defines"])
        names_of, by_name, by_file_name, by_type, by_module, children, seeds = {}, {}, {}, {}, {}, {}, []
        for path in paths:
            crate = crate_of(path)
            if path in self.module_name:
                by_module.setdefault((crate, self.module_name[path]), []).append(path)
                children.setdefault(self.parent[path], []).append(path)
            for index, item in enumerate(self.files[path]["items"]):
                node = (path, index, None)
                names = set(item.get("names", ()))
                if item["kind"] == "macro_call":
                    names |= expands.get(item["macro"], set())
                names_of[node] = names
                if item["kind"] == "glue":
                    seeds.append(node)
                elif item["kind"] == "macro_call" and not names:
                    # A macro call defining nothing by name may still expand to impls: it counts once a type or
                    # trait it mentions is reached.
                    for name in item["mentions"]:
                        by_type.setdefault(name, []).append(node)
                elif item["kind"] == "impl":
                    required = {name for own, foreign in self.requirement(path, item) for name in own + foreign}
                    members = [(path, index, member) for member in range(len(item["members"]))] if item["inherent"] else []
                    for member_node in members:
                        name = item["members"][member_node[2]]["name"]
                        names_of[member_node] = set()
                        if name:
                            by_name.setdefault(name, []).append(member_node)
                    for name in required:
                        for other in [node] + members:
                            by_type.setdefault(name, []).append(other)
                    if not required:
                        seeds.append(node)
                        seeds += [member_node for member_node in members if not item["members"][member_node[2]]["name"]]
                else:
                    for name in names:
                        by_name.setdefault(name, []).append(node)
                        by_file_name.setdefault((path, name), []).append(node)
        self.tables = dict(idents=idents, sees=sees, seen_by=seen_by, names_of=names_of, by_name=by_name,
                           by_file_name=by_file_name, by_type=by_type, by_module=by_module, children=children,
                           seeds=seeds, node_sets={})
        return self.tables

    def reachable(self, entries, universe, reasons=None, stop=frozenset()):
        """The files of universe (workspace Rust sources) holding an item reachable from entries (by the rules in
        this module's doc), and the entries. reasons, a dict, gets why each file was reached (for debugging). The
        search ends early, with only part of the answer, once it reaches a file of stop."""
        # Searching every source of the crates involved (not only those Cargo listed) only adds reach, and lets
        # harnesses with the same entries (the game binary's) share one search.
        crates = frozenset(crate_of(path) for path in universe)
        key = (frozenset(entries), crates)
        if reasons is None and key in self.results:
            return self.results[key] & set(universe)
        everything = {path for path in self.files if crate_of(path) in crates} | set(universe)
        files, complete = self.search(entries, everything, reasons, stop)
        if complete:
            self.results[key] = files
        return files & set(universe)

    def search(self, entries, universe, reasons, stop=frozenset()):
        targets = {self.target(path) for path in entries}
        universe = {path for path in universe if (self.is_library(path) or self.target(path) in targets)
                    and path not in self.test_only}
        tables = self.index()
        idents, sees, seen_by, names_of = tables["idents"], tables["sees"], tables["seen_by"], tables["names_of"]
        node_sets = tables["node_sets"]
        by_name, by_file_name, by_type = tables["by_name"], tables["by_file_name"], tables["by_type"]
        by_module, children = tables["by_module"], tables["children"]
        seeds = [node for node in tables["seeds"] if node[0] in universe]
        for path in universe:
            if (path in entries or (path not in self.parent and path not in self.roots)) and self.parse(path):
                seeds += [(path, index, None) for index in range(len(self.parse(path)["items"]))]

        def names(node):
            return names_of.get(node, set())

        def sets_of(node, item):
            """A node's mentions, path words, qualified names and macro calls, as sets (parses keep lists)."""
            if node not in node_sets:
                source = item["members"][node[2]] if node[2] is not None else item
                node_sets[node] = {key: set(source.get(key, ())) for key in ("mentions", "paths", "qualified", "invokes")}
            return node_sets[node]

        def item_of(node):
            return self.parse(node[0])["items"][node[1]]

        reached, opened, globbed, files, queue = set(), set(), set(), set(), []
        # name -> the crates whose reached code mentions it plainly, names it in a path or an import, invokes it.
        mentioned, qualified, invoked = {}, {}, {}
        done = {"mentions": {}, "qualified": {}, "invokes": {}, "paths": {}}  # crate -> names already handled
        path_names, types = {}, {}  # by crate
        local = {}  # file -> plain mentions by reached code in it or below it (child modules may `use super::*`)
        local_types = {}  # file -> its reached types and traits

        def heard(table, name, crate):
            """Whether a crate that can see crate's items has name in table."""
            sources = table.get(name)
            return bool(sources) and not sources.isdisjoint(seen_by.get(crate, {crate}))

        def ready(node):
            path, index, member = node
            item = item_of(node)
            crate = crate_of(path)
            if item["kind"] == "impl":
                visible = visible_types(crate)
                here = local_types.get(path, set())
                if not all((not own and not foreign) or here.intersection(own) or visible.intersection(foreign)
                           for own, foreign in self.requirement(path, item)):
                    return False
                name = item["members"][member]["name"] if member is not None else None
                return name is None or heard(mentioned, name, crate)  # a method: called by its name
            if item["kind"] == "macro_call" and not names(node):
                return not visible_types(crate).isdisjoint(item["mentions"])
            names_ = names(node)
            if item["kind"] == "macro":
                return any(heard(invoked, name, crate) for name in names_)
            if item["kind"] == "use" and item["glob"]:
                return path in opened
            nearby = local.get(path, ())
            if any(name in nearby for name in names_):
                return True  # named in its own module, or below it
            if item["kind"] == "use" and not item["public"]:
                return False
            if path in opened and any(heard(qualified, name, crate) for name in names_):
                return True  # named through a path, or imported
            return path in globbed and any(heard(mentioned, name, crate) for name in names_)

        visible_cache = {}

        def visible_types(crate):
            """The reached types and traits of the crates a crate sees (cached until a type is reached)."""
            if crate not in visible_cache:
                visible_cache[crate] = set().union(*(types.get(other, set()) for other in sees.get(crate, {crate})))
            return visible_cache[crate]

        def wake(nodes, why):
            for other in nodes:
                if other not in reached and other[0] in universe and ready(other):
                    reach(other, why)

        def open_module(path, why):
            pending = [path]
            while pending:
                current = pending.pop()
                if current in opened:
                    continue
                opened.add(current)
                crate = crate_of(current)
                pending += [child for child in children.get(current, ())
                            if child in universe and self.module_name[child] in path_names.get(crate, ())]
                parsed = self.parse(current)
                wake([(current, index, None) for index, item in enumerate(parsed["items"] if parsed else ())
                      if item["kind"] != "impl"], f"{current} opened ({why})")

        def reach(node, why):
            if node in reached:
                return
            reached.add(node)
            queue.append(node)
            path, _, member = node
            item = item_of(node)
            if (item["kind"] == "use" and not item["public"]) or (item["kind"] == "impl" and item["inherent"] and member is None):
                return
            if path not in files:
                files.add(path)
                if reasons is not None:
                    reasons[path] = why
                if path in stop:
                    raise Stop

        try:
            for path in universe:
                if path in entries or path not in self.parent and not self.is_library(path):
                    open_module(path, "entry, or a file no module places")
            for node in seeds:
                reach(node, "entry" if node[0] in entries else "always counted")
            for path in entries & universe:
                files.add(path)
                if reasons is not None:
                    reasons.setdefault(path, "entry")
            while queue:
                node = queue.pop()
                path, index, member = node
                item = item_of(node)
                crate = crate_of(path)
                label = (item.get("names") or [item["kind"]])[0] if member is None else item["members"][member]["name"]
                source = sets_of(node, item)
                why = f"{path} ({label})"
                # A library's root opens once reached code names the crate (`horizon_world::`, `use horizon_command
                # as command`): its items are out of reach until then.
                for word in (source["mentions"] | source["paths"]) & idents.keys():
                    root = self.library_roots[idents[word]]
                    if root in universe and root not in opened:
                        open_module(root, f"{why} names {word}")
                # Plain mentions reach items of this file and of the modules above it.
                ancestor, new = path, source["mentions"]
                while ancestor is not None and new:
                    known = local.setdefault(ancestor, set())
                    new = new - known
                    known |= new
                    for name in new:
                        wake(by_file_name.get((ancestor, name), ()), f"{why} mentions {name}")
                    ancestor = self.parent.get(ancestor)
                if item["kind"] == "use" and item["glob"]:
                    for name in item["names"]:
                        for target in sees.get(crate, {crate}):
                            for module in list(by_module.get((target, name), ())) + \
                                    ([self.library_roots[idents[name]]] if name in idents else []):
                                if module not in globbed and module in universe:
                                    globbed.add(module)
                                    wake([(module, index, None) for index in range(len(self.parse(module)["items"]))]
                                         if self.parse(module) else [], f"{why} glob-imports {name}")
                visible_to = sees.get(crate, {crate})
                new = source["paths"] - done["paths"].setdefault(crate, set())
                done["paths"][crate] |= new
                for name in new:
                    for target in visible_to:
                        path_names.setdefault(target, set()).add(name)
                        for module in by_module.get((target, name), ()):
                            if module in universe and self.parent[module] in opened:
                                open_module(module, f"{why} names {name}")
                for key, table, wanted in (("qualified", qualified, lambda other: True),
                                           ("invokes", invoked, lambda other: item_of(other)["kind"] == "macro"),
                                           ("mentions", mentioned, lambda other: other[2] is not None or other[0] in globbed)):
                    new = source[key] - done[key].setdefault(crate, set())
                    done[key][crate] |= new
                    for name in new:
                        table.setdefault(name, set()).add(crate)
                        wake([other for other in by_name.get(name, ()) if crate_of(other[0]) in visible_to and wanted(other)],
                             f"{why} {key} {name}")
                if item["kind"] in TYPE_KINDS and member is None:
                    own = types.setdefault(crate, set())
                    local_types.setdefault(path, set()).update(names(node))
                    if not names(node) <= own:
                        own |= names(node)
                        visible_cache.clear()
                    seeing = seen_by.get(crate, {crate})
                    for name in names(node):  # also when the crate has one by that name already: impls of this one
                        wake([other for other in by_type.get(name, ()) if crate_of(other[0]) in seeing],
                             f"implements for {name}, defined in {path}")
        except Stop:
            return files, False
        return files, True


class Stop(Exception):
    """A search reached a file it was told to stop at."""


def impl_subject(header):
    """An impl header (the text after `impl`): its generic parameters' names, and the rest without its where clause
    (the trait and the type it is for)."""
    header, parameters = header.strip(), set()
    if header.startswith("<"):
        depth = 0
        for index, character in enumerate(header):
            depth += {"<": 1, ">": -1}.get(character, 0)
            if depth == 0:
                generics, header = header[1:index], header[index + 1:]
                parameters = {match.group(1) for match in re.finditer(rf"(?:^|,)\s*(?:const\s+)?({IDENT})", generics)}
                break
    return parameters, re.split(r"\bwhere\b", header)[0]


_CRATES = {}


def crate_of(path):
    """The crate folder of a crates/<crate>/... path (called millions of times by a search: cached)."""
    crate = _CRATES.get(path)
    if crate is None:
        crate = _CRATES[path] = path.split("/", 2)[1]
    return crate


def is_workspace_rust(path):
    return path.startswith("crates/") and path.endswith(".rs") and path.count("/") >= 2


PATH_DEPENDENCY = re.compile(r'path\s*=\s*"\.\./([\w-]+)"')


@functools.cache
def _path_dependencies(root, crate):
    manifest = Path(root) / "crates" / crate / "Cargo.toml"
    text = manifest.read_text(encoding="utf-8") if manifest.is_file() else ""
    return frozenset(PATH_DEPENDENCY.findall(text))


def linked_crates(root, crates):
    """The workspace crates (folders under crates/) that crates link, themselves included: their path dependencies,
    transitively. A harness recorded before code moved to a new crate links that crate without naming its files."""
    found, queue = set(crates), list(crates)
    while queue:
        for dependency in _path_dependencies(str(root), queue.pop()):
            if dependency not in found:
                found.add(dependency)
                queue.append(dependency)
    return found


def unrecorded(root, plain, changed):
    """Changed workspace Rust files of crates a harness's recorded crates link but its record doesn't name."""
    crates = {crate_of(path) for path in plain if path.startswith("crates/") and path.count("/") >= 2}
    linked = linked_crates(root, crates) - crates
    return {path for path in changed if is_workspace_rust(path) and crate_of(path) in linked}


DECLARATION_LINE = re.compile(rf"\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+({IDENT})\s*;")


def declarations(text, names, keep_rest):
    """A Rust file's `mod` declarations of names, each with the attribute lines just above it; with keep_rest, the
    whole text except the declarations of other modules instead."""
    lines, found, attributes = text.splitlines(), [], []
    for line in lines:
        if line.strip().startswith("#[") and not line.strip().startswith("#!["):
            attributes.append(line.strip())
            continue
        match = DECLARATION_LINE.match(line)
        if match and match.group(1) in names:
            found += attributes + [line.strip()]
        elif keep_rest and not match:
            found += attributes + [line]
        attributes = []
    return "\n".join(found + attributes)


def declarations_changed(root, path, children, base, whole):
    """Whether a changed file differs from the commit base in what a harness depends on: for a module parent, its
    `mod` declarations of the reached children; for a whole file (whole), everything but its declarations of other
    modules (adding a module beside reached ones changes nothing). True without a base, or if either side is
    missing."""
    if base is None:
        return True
    return _declarations_changed(str(root), path, frozenset(children), base, whole)


@functools.lru_cache(maxsize=None)
def _declarations_changed(root, path, children, base, whole):
    try:
        head = (Path(root) / path).read_text(encoding="utf-8", errors="replace")
        before = subprocess.run(["git", "show", f"{base}:{path}"], cwd=root, capture_output=True, text=True, check=True).stdout
    except (OSError, subprocess.CalledProcessError):
        return True
    return declarations(head, children, whole) != declarations(before, children, whole)


def narrow(dependencies, artifacts, graph, changed=(), stop=frozenset()):
    """A harness's dependencies narrowed to what its entry points can reach: {dependency: (whole, children)}, where
    whole says the file counts for its own content (else only through its `mod` declarations of children, as a
    module ancestor of a reached file) and children are its reached child modules. Without known entry points (a
    probe or binary the harness built that isn't found here), every dependency is kept whole. The search may end
    as soon as it reaches a file of stop (changed files that count once reached): then only part is returned."""
    plain = [dependency.split("#keep:")[0] for dependency in dependencies]
    crates = {crate_of(path) for path in plain if path.startswith("crates/") and path.count("/") >= 2}
    rust = {path for path in plain if is_workspace_rust(path)}
    added = {path for path in changed if is_workspace_rust(path) and crate_of(path) in linked_crates(graph.root, crates)
             and path not in rust and (graph.root / path).is_file()}
    entries = graph.entries(artifacts, rust)
    if entries is None:
        return {dependency: (True, None) for dependency in dependencies}
    missing = {path for path in rust if graph.parse(path) is None}
    reached = graph.reachable(entries, rust | added | entries, stop=frozenset(stop) & (rust | added)) | missing
    named, named_in_crate = set(), {}
    for path in rust | added:
        parsed = graph.parse(path)
        file_names = set(parsed["file_names"]) if parsed else set()
        if path in reached:
            named |= file_names
        named_in_crate.setdefault(crate_of(path), set()).update(file_names)
    kept = {}
    for dependency, path in zip(dependencies, plain):
        if is_workspace_rust(path):
            if path in reached:
                kept[dependency] = (True, set())
        elif not path.startswith("crates/") or path.endswith(("Cargo.toml", "build.rs")) \
                or path.rsplit("/", 1)[-1] in named \
                or path.rsplit("/", 1)[-1] not in named_in_crate.get(crate_of(path), ()):
            kept[dependency] = (True, None)
    for path in sorted(added & reached):
        kept[path] = (True, set())
    for path in sorted(reached):
        child = path
        while child in graph.parent:
            parent = graph.parent[child]
            kept.setdefault(parent, (False, set()))[1].add(graph.module_name[child])
            child = parent
    return kept
