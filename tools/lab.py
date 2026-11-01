#!/usr/bin/env python3
"""Pinned Java Edition import/decompilation. Game files come from the user's own
copy (their official launcher install and the server jar they downloaded); all
generated files stay in .local/. Also the small helpers the harnesses share
(free ports, offline UUIDs, VarInts, float bits)."""

import argparse
import atexit
from collections import Counter
import hashlib
import json
import os
import platform
from pathlib import Path, PurePosixPath
import re
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import urllib.request
import uuid
import zipfile

ROOT = Path(__file__).resolve().parents[1]
LOCAL = ROOT / ".local"
LOCK = ROOT / "research-lock.json"


def pinned_version():
    """The Minecraft version research-lock.json pins: every tool reads it from there, never by hand."""
    version = json.loads(LOCK.read_text(encoding="utf-8"))["minecraft"]["version"]
    if not re.fullmatch(r"[A-Za-z0-9_.-]+", version) or version in {".", ".."}:
        raise ValueError("Unsafe version identifier in lock file")
    return version


VERSION = pinned_version()
# The pinned version's imported jars, libraries and sources (tools/lab.py prepare), and the resources
# horizon-prepare made from them.
GAME = LOCAL / "minecraft" / VERSION
RESOURCES = LOCAL / "horizon-client" / VERSION


def version_info(client_jar=None):
    """What the pinned client jar states about itself in its version.json (protocol_version, world_version,
    pack_version, java_version...)."""
    with zipfile.ZipFile(client_jar or GAME / "jars" / "client.jar") as archive:
        return json.loads(archive.read("version.json"))


def cargo_target():
    """Cargo's target directory, honouring CARGO_TARGET_DIR (tools/parity.py sets it)."""
    return ROOT / os.environ.get("CARGO_TARGET_DIR", "target")


_SCRATCH_DIRS = []


def _scratch_dir(prefix):
    path = tempfile.mkdtemp(prefix=prefix)
    _SCRATCH_DIRS.append(path)
    return path


@atexit.register
def _remove_scratch_dirs():
    for path in _SCRATCH_DIRS:
        shutil.rmtree(path, ignore_errors=True)


def game_env(base=None, **extra):
    """The environment for every run of the game, the launcher or horizon-ctl from a tool or a harness: no audio,
    no Discord presence, and fresh scratch folders for the game's home (logs, saves, options), the launcher's home and the XDG folders,
    so a run never writes into a real player's folders (the game falls back to them when HORIZON_HOME is unset).
    A folder the caller already has in `base` (default os.environ) or passes in `extra` is kept. The scratch folders
    are removed when this process exits."""
    env = dict(os.environ if base is None else base)
    env.update({key: str(value) for key, value in extra.items()})
    env["HORIZON_NO_AUDIO"] = "1"
    env["HORIZON_LAUNCHER_NO_SELF_UPDATE"] = "1"
    # Never a real Discord's Rich Presence; tools/discord_presence_e2e.py points the game at a fake one.
    if not env.get("HORIZON_DISCORD_IPC_DIR"):
        env["HORIZON_NO_DISCORD"] = "1"
    for key, prefix in (("HORIZON_HOME", "horizon-home-"), ("HORIZON_LAUNCHER_HOME", "horizon-launcher-home-"),
                        ("XDG_DATA_HOME", "horizon-xdg-data-"), ("XDG_CONFIG_HOME", "horizon-xdg-config-")):
        if not env.get(key):
            env[key] = _scratch_dir(prefix)
    return env


def link_dir(target, link):
    """Make `link` a link to the folder `target`: a symbolic link, or on Windows, where creating one needs
    Developer Mode or an administrator (WinError 1314), a directory junction, which needs neither.
    shutil.rmtree removes a junction without touching what it points to, as it does a symbolic link."""
    try:
        os.symlink(target, link, target_is_directory=True)
    except OSError as error:
        if sys.platform != "win32" or getattr(error, "winerror", None) != 1314:
            raise
        subprocess.run(["cmd", "/c", "mklink", "/J", str(link), str(Path(target).resolve())],
                       check=True, capture_output=True)


def write_json(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")


def digest(path, algorithm):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, algorithm).hexdigest()


def free_port():
    """A free local TCP port, for a server a harness starts."""
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def offline_uuid(name):
    """UUIDUtil.createOfflinePlayerUUID: the UUID an offline-mode server gives a player name."""
    digest = bytearray(hashlib.md5(("OfflinePlayer:" + name).encode()).digest())
    digest[6] = (digest[6] & 0x0F) | 0x30
    digest[8] = (digest[8] & 0x3F) | 0x80
    return uuid.UUID(bytes=bytes(digest))


def varint(value):
    """FriendlyByteBuf.writeVarInt: the int's 32 bits, 7 at a time."""
    out = b""
    value &= 0xFFFFFFFF
    while True:
        b = value & 0x7F
        value >>= 7
        out += bytes([b | (0x80 if value else 0)])
        if not value:
            return out


def parse_varint(data, pos):
    """The VarInt at data[pos]: (the position after it, its unsigned value)."""
    value = shift = 0
    while True:
        b = data[pos]
        pos += 1
        value |= (b & 0x7F) << shift
        shift += 7
        if not b & 0x80:
            return pos, value


def fbits(value):
    """A float's IEEE 754 bits in hex, as the Java oracles print them."""
    return struct.pack(">f", value).hex()


def dbits(value):
    """A double's IEEE 754 bits in hex, as the Java oracles print them."""
    return struct.pack(">d", value).hex()


def settled(lines):
    """One player's lines of one step, in order, with the waypoint packets reduced to the last for each waypoint:
    what the locator bar shows once the step is over. Arriving in a level, vanilla sends the same waypoint more
    than once on the way there; the packet by packet order is test_locator_bar_parity's."""
    def waypoint(line):
        parts = line.split(" ")
        return parts[3] if len(parts) > 3 and parts[2] in ("track", "untrack", "update") else None
    last = {waypoint(l): i for i, l in enumerate(lines) if waypoint(l)}
    return [l for i, l in enumerate(lines) if not waypoint(l) or last[waypoint(l)] == i]


def download(url, destination, checksum, algorithm="sha1"):
    """Never promote an unverified or partial download into the artifact cache."""
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists() and digest(destination, algorithm) == checksum:
        return destination
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=destination.parent, delete=False) as stream:
            temporary = Path(stream.name)
            request = urllib.request.Request(url, headers={"User-Agent": "horizon/0.1"})
            with urllib.request.urlopen(request, timeout=120) as response:
                shutil.copyfileobj(response, stream)
        actual = digest(temporary, algorithm)
        if actual != checksum:
            raise ValueError(f"Checksum mismatch for {destination.name}: expected {checksum}, got {actual}")
        temporary.replace(destination)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)
    return destination


LAUNCHER_HINT = ("Horizon does not download the game: install Minecraft Java Edition with the official "
                 "launcher, play {version} once, then rerun. If the launcher uses another game directory, "
                 "pass --minecraft-dir or set HORIZON_MINECRAFT_DIR.")
SERVER_HINT = ("Horizon does not download the game: download the {version} server jar yourself (minecraft.net/download/server "
               "while {version} is the current release, otherwise the downloads.server.url in your launcher's "
               "versions/{version}/{version}.json), then pass --server-jar PATH or set HORIZON_SERVER_JAR.")


def minecraft_dir(override=None):
    """The official launcher's game directory, which holds the user's own copy of the game."""
    if override or os.environ.get("HORIZON_MINECRAFT_DIR"):
        return Path(override or os.environ["HORIZON_MINECRAFT_DIR"]).expanduser()
    if sys.platform == "darwin":
        return Path.home() / "Library" / "Application Support" / "minecraft"
    if sys.platform == "win32":
        return Path(os.environ.get("APPDATA", Path.home())) / ".minecraft"
    return Path.home() / ".minecraft"


def import_copy(source, destination, checksum, algorithm="sha1", hint=""):
    """Copy one file from the user's own install, verified like a download."""
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists() and digest(destination, algorithm) == checksum:
        return destination
    if not source.is_file():
        raise FileNotFoundError(f"Missing {source}. {hint}".strip())
    actual = digest(source, algorithm)
    if actual != checksum:
        raise ValueError(f"{source} is not the pinned file: expected {algorithm} {checksum}, got {actual}")
    with tempfile.NamedTemporaryFile(dir=destination.parent, delete=False) as stream:
        temporary = Path(stream.name)
    try:
        shutil.copyfile(source, temporary)
        temporary.replace(destination)
    finally:
        temporary.unlink(missing_ok=True)
    return destination


def manifest_accepted(lock, checksum):
    """Mojang republishes a version JSON for asset-index changes; every known one is accepted."""
    game = lock["minecraft"]
    return checksum == game["manifest_sha1"] or checksum in game.get("accepted_manifest_sha1", [])


def import_manifest(lock, directory, minecraft=None):
    """The pinned version JSON, from the launcher's versions/ directory.

    The launcher's copy wins over the cached one so the manifest matches the asset index the
    launcher installed alongside it."""
    game = lock["minecraft"]
    version = game["version"]
    destination = directory / "manifest.json"
    source = minecraft_dir(minecraft) / "versions" / version / f"{version}.json"
    if source.is_file():
        manifest = json.loads(source.read_text(encoding="utf-8"))
        downloads = manifest.get("downloads", {})
        if (manifest.get("id") != version
                or downloads.get("client", {}).get("sha1") != game["client_sha1"]
                or downloads.get("server", {}).get("sha1") != game["server_sha1"]):
            raise ValueError(f"{source} does not describe the pinned {version} release")
        checksum = digest(source, "sha1")
        if not manifest_accepted(lock, checksum):
            raise ValueError(f"{source} is a {version} version file Horizon does not know yet (sha1 {checksum}); "
                             f"Mojang may have republished it. Update Horizon, or report it if this is the latest release.")
        if not (destination.exists() and digest(destination, "sha1") == checksum):
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, destination)
    elif not (destination.exists() and manifest_accepted(lock, digest(destination, "sha1"))):
        raise FileNotFoundError(f"Missing {source}. {LAUNCHER_HINT.format(version=version)}")
    manifest = json.loads(destination.read_text(encoding="utf-8"))
    if manifest["javaVersion"]["majorVersion"] != game["java_major"]:
        raise ValueError("Manifest Java version does not match lock file")
    return manifest


def import_client(lock, directory, minecraft=None):
    """The user's client jar from their launcher install, checked against the lock."""
    version = lock["minecraft"]["version"]
    manifest = import_manifest(lock, directory, minecraft)
    source = minecraft_dir(minecraft) / "versions" / version / f"{version}.jar"
    jar = import_copy(source, directory / "jars" / "client.jar", lock["minecraft"]["client_sha1"],
                      hint=LAUNCHER_HINT.format(version=version))
    return manifest, jar


def import_server(lock, directory, server_jar=None):
    """The server jar the user downloaded themselves, checked against the lock."""
    version = lock["minecraft"]["version"]
    destination = directory / "jars" / "server.jar"
    checksum = lock["minecraft"]["server_sha1"]
    source = server_jar or os.environ.get("HORIZON_SERVER_JAR")
    hint = SERVER_HINT.format(version=version)
    if source is None:
        if destination.exists() and digest(destination, "sha1") == checksum:
            return destination
        raise FileNotFoundError(f"No {version} server jar. {hint}")
    return import_copy(Path(source).expanduser(), destination, checksum, hint=hint)


def rule_system(system=None):
    """The launcher's os.name for this machine: linux, osx or windows."""
    return system or {"Darwin": "osx", "Linux": "linux", "Windows": "windows"}[platform.system()]


ARCHITECTURE_ALIASES = [{"x86_64", "amd64", "x64"}, {"arm64", "aarch64"}]


def rule_architectures(architecture=None):
    """platform.machine() and its aliases: Windows reports AMD64/ARM64, Linux aarch64, macOS arm64."""
    architecture = (architecture or platform.machine()).lower()
    return next((names for names in ARCHITECTURE_ALIASES if architecture in names), {architecture})


def library_allowed(library, system=None, architecture=None):
    """The official launcher's rules: the last matching rule decides, no rules means allowed."""
    system, architectures = rule_system(system), rule_architectures(architecture)
    rules = library.get("rules", [])
    allowed = not rules
    for rule in rules:
        operating_system = rule.get("os", {})
        if operating_system.get("name", system) != system:
            continue
        if "arch" in operating_system and not any(re.fullmatch(operating_system["arch"], name)
                                                  for name in architectures):
            continue
        if "version" in operating_system and not re.search(operating_system["version"], platform.release()):
            continue
        if rule.get("features"):
            continue
        allowed = rule["action"] == "allow"
    return allowed


ARM_CLASSIFIER = re.compile(r"(?:^|-)(?:arm64|aarch64|aarch_64)$")
X86_CLASSIFIER = re.compile(r"(?:^|-)(?:x86_64|x64)$")


def native_library_matches(library, architecture=None, names=None):
    """Modern manifests list x64 and ARM natives with OS-only rules; keep this machine's.

    A plain natives-<os> jar is the x64 build when the manifest's library names also hold
    an ARM variant of it (assumed when names is None), and the only build otherwise."""
    arm = "arm64" in rule_architectures(architecture)
    parts = library["name"].split(":")
    classifier = parts[3] if len(parts) > 3 else ""
    if ARM_CLASSIFIER.search(classifier):
        return arm
    if X86_CLASSIFIER.search(classifier):
        return not arm
    if arm and classifier.startswith("natives-"):
        return names is not None and not any(f"{library['name']}-{suffix}" in names
                                             for suffix in ("arm64", "aarch64"))
    return True


def client_library_artifacts(manifest, system=None, architecture=None):
    """The client libraries the official launcher installs on this OS and architecture."""
    names = {library["name"] for library in manifest["libraries"]}
    return [library["downloads"]["artifact"] for library in manifest["libraries"]
            if "artifact" in library.get("downloads", {})
            and library_allowed(library, system, architecture)
            and native_library_matches(library, architecture, names)]


def import_library(artifact, destination_root, minecraft=None):
    """One library jar from the launcher's libraries/ directory."""
    relative = PurePosixPath(artifact["path"])
    if relative.is_absolute() or ".." in relative.parts or "\\" in artifact["path"]:
        raise ValueError("Unsafe library path")
    source = minecraft_dir(minecraft) / "libraries" / relative
    hint = LAUNCHER_HINT.format(version=json.loads(LOCK.read_text(encoding="utf-8"))["minecraft"]["version"])
    return import_copy(source, destination_root / relative, artifact["sha1"], hint=hint)


def launcher_assets(manifest, minecraft=None):
    """The launcher's asset directory and the pinned version's asset index."""
    assets = minecraft_dir(minecraft) / "assets"
    info = manifest["assetIndex"]
    index = assets / "indexes" / f"{info['id']}.json"
    if not index.is_file():
        raise FileNotFoundError(f"Missing {index}. {LAUNCHER_HINT.format(version=manifest['id'])}")
    checksum = digest(index, "sha1")
    known = json.loads(LOCK.read_text(encoding="utf-8"))["minecraft"].get("accepted_asset_index_sha1", [])
    if checksum != info["sha1"] and checksum not in known:
        # Mojang republishes an index under the same id (#1076); each object is still verified.
        print(f"{index} is an asset index Horizon does not know yet (sha1 {checksum}); "
              "using it, every file is still checked against its own SHA-1", file=sys.stderr)
    return assets, index


def launcher_object(assets, checksum):
    """One verified object from the launcher's asset store."""
    path = assets / "objects" / checksum[:2] / checksum
    if not path.is_file():
        raise FileNotFoundError(f"Missing asset object {path}. Play the pinned version once in the official launcher.")
    if digest(path, "sha1") != checksum:
        raise ValueError(f"Corrupt asset object {path}; let the launcher repair it")
    return path


def config():
    lock = json.loads(LOCK.read_text(encoding="utf-8"))
    return lock, LOCAL / "minecraft" / pinned_version()


def bundle_entries(archive, name):
    for line in archive.read(name).decode("utf-8").splitlines():
        if not line.strip():
            continue
        checksum, identifier, relative = line.split("\t")
        path = PurePosixPath(relative)
        if path.is_absolute() or ".." in path.parts or "\\" in relative:
            raise ValueError(f"Unsafe bundled path: {relative}")
        yield checksum, identifier, path


def extract_verified(archive, member, destination, checksum):
    data = archive.read(member)
    if hashlib.sha256(data).hexdigest() != checksum:
        raise ValueError(f"Bundled artifact checksum mismatch: {member}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_bytes(data)


def unpack_server(server, directory, version):
    """Modern server downloads are launchers; decompile their inner game JAR."""
    with zipfile.ZipFile(server) as archive:
        if "META-INF/versions.list" not in archive.namelist():
            return server, []
        versions = list(bundle_entries(archive, "META-INF/versions.list"))
        if len(versions) != 1 or versions[0][1] != version:
            raise ValueError("Expected one bundled server matching the pinned version")
        checksum, _, relative = versions[0]
        game = directory / "jars" / "server-game.jar"
        extract_verified(archive, f"META-INF/versions/{relative}", game, checksum)
        libraries = []
        for checksum, _, relative in bundle_entries(archive, "META-INF/libraries.list"):
            destination = directory / "libraries" / "server" / relative
            extract_verified(archive, f"META-INF/libraries/{relative}", destination, checksum)
            libraries.append(destination)
        return game, libraries


class ClassReader:
    """Read SourceFile and method count; no Java text needs to be published."""

    def __init__(self, data):
        self.data = data
        self.offset = 0

    def take(self, count):
        value = self.data[self.offset:self.offset + count]
        if len(value) != count:
            raise ValueError("Truncated class file")
        self.offset += count
        return value

    def number(self, count):
        return int.from_bytes(self.take(count), "big")

    def attributes(self):
        return [(self.number(2), self.take(self.number(4))) for _ in range(self.number(2))]

    def metadata(self):
        if self.take(4) != b"\xca\xfe\xba\xbe":
            raise ValueError("Invalid class file magic")
        self.take(4)  # Minor and major version.
        pool = [None] * self.number(2)
        sizes = {3: 4, 4: 4, 5: 8, 6: 8, 7: 2, 8: 2, 9: 4, 10: 4,
                 11: 4, 12: 4, 15: 3, 16: 2, 17: 4, 18: 4, 19: 2, 20: 2}
        i = 1
        while i < len(pool):
            tag = self.number(1)
            if tag == 1:
                pool[i] = self.take(self.number(2)).decode("utf-8", errors="replace")
            else:
                if tag not in sizes:
                    raise ValueError(f"Unknown constant pool tag: {tag}")
                self.take(sizes[tag])
            i += 2 if tag in {5, 6} else 1
        self.take(6)  # Access, this class, super class.
        self.take(self.number(2) * 2)  # Interfaces.
        for _ in range(self.number(2)):  # Fields.
            self.take(6)
            self.attributes()
        methods = self.number(2)
        for _ in range(methods):
            self.take(6)
            self.attributes()
        source = None
        for name, data in self.attributes():
            if pool[name] == "SourceFile":
                source = pool[struct.unpack(">H", data)[0]]
        return {"source_file": source, "methods": methods}


def inventory(jar):
    classes = []
    packages = Counter()
    with zipfile.ZipFile(jar) as archive:
        for item in archive.infolist():
            if not item.filename.endswith(".class"):
                continue
            metadata = ClassReader(archive.read(item)).metadata()
            path = PurePosixPath(item.filename)
            source = metadata["source_file"] or path.name.split("$")[0].removesuffix(".class") + ".java"
            if PurePosixPath(source).name != source or "\\" in source:
                raise ValueError(f"Unsafe SourceFile attribute in {item.filename}")
            source_path = str(path.parent / source)
            packages[str(path.parent)] += 1
            classes.append({"class": item.filename, "source": source_path,
                            "bytecode_bytes": item.file_size, "methods": metadata["methods"]})
    return {"jar_sha256": digest(jar, "sha256"), "class_count": len(classes),
            "expected_source_files": sorted({c["source"] for c in classes}),
            "method_count": sum(c["methods"] for c in classes),
            "packages": dict(packages.most_common()), "classes": classes}


def prepare(minecraft=None, server_jar=None):
    lock, directory = config()
    manifest, client = import_client(lock, directory, minecraft)
    if "client_mappings" in manifest["downloads"] or "server_mappings" in manifest["downloads"]:
        raise ValueError("This workflow targets unobfuscated releases. Older releases need a remapping stage.")
    print(f"Imported client {manifest['id']} from {minecraft_dir(minecraft)}", flush=True)
    jars = {"client": client, "server": import_server(lock, directory, server_jar)}
    jars["server"], server_libraries = unpack_server(jars["server"], directory, manifest["id"])
    client_libraries = [import_library(artifact, directory / "libraries" / "client", minecraft)
                        for artifact in client_library_artifacts(manifest)]
    vf = lock["vineflower"]
    download(vf["url"], LOCAL / "tools" / f"vineflower-{vf['version']}.jar", vf["sha256"], "sha256")
    for side, jar in jars.items():
        data = inventory(jar)
        write_json(directory / "reports" / f"{side}-inventory.json", data)
        print(f"{side}: {data['class_count']} classes, {data['method_count']} methods", flush=True)
    write_json(directory / "prepared.json", {
        "version": manifest["id"], "jars": {k: str(v.relative_to(ROOT)) for k, v in jars.items()},
        "libraries": {"client": [str(p.relative_to(ROOT)) for p in client_libraries],
                      "server": [str(p.relative_to(ROOT)) for p in server_libraries]},
    })
    print(f"Artifacts: {directory}", flush=True)


def coverage(directory, side, source_root=None):
    source_root = source_root or directory / "sources" / side
    data = json.loads((directory / "reports" / f"{side}-inventory.json").read_text(encoding="utf-8"))
    actual = {path.relative_to(source_root).as_posix() for path in source_root.rglob("*.java")}
    expected = set(data["expected_source_files"])
    placeholders = []
    for path in source_root.rglob("*.java"):
        contents = path.read_text(encoding="utf-8", errors="replace")
        if re.search(r"\$VF:.*(?:couldn't|could not|failed)", contents, re.IGNORECASE):
            placeholders.append(path.relative_to(source_root).as_posix())
    return {"side": side, "input_classes": data["class_count"],
            "expected_source_files": len(expected), "emitted_source_files": len(actual),
            "missing_source_files": sorted(expected - actual),
            "files_with_decompiler_error_markers": sorted(placeholders),
            "coverage_note": "Inner classes share their SourceFile; file coverage does not prove method fidelity or recompilability."}


def decompile(args):
    lock, directory = config()
    prepared = json.loads((directory / "prepared.json").read_text(encoding="utf-8"))
    vf = lock["vineflower"]
    tool = LOCAL / "tools" / f"vineflower-{vf['version']}.jar"
    if digest(tool, "sha256") != vf["sha256"]:
        raise ValueError("Decompiler checksum changed; rerun prepare")
    java = shutil.which(args.java)
    if not java:
        raise ValueError("Java not found; install JDK 25 or provide --java /path/to/java")
    version = subprocess.run([java, "-version"], capture_output=True, text=True, check=True)
    match = re.search(r'version "(\d+)', version.stderr + version.stdout)
    if not match or int(match[1]) < lock["minecraft"]["java_major"]:
        raise ValueError(f"JDK {lock['minecraft']['java_major']} or newer is required")
    sides = ["client", "server"] if args.side == "both" else [args.side]
    incomplete = False
    for side in sides:
        destination = directory / "sources" / side
        jar = ROOT / prepared["jars"][side]
        data = json.loads((directory / "reports" / f"{side}-inventory.json").read_text(encoding="utf-8"))
        if digest(jar, "sha256") != data["jar_sha256"]:
            raise ValueError(f"{side} input changed; rerun prepare")
        if destination.exists():
            raise ValueError(f"Output already exists: {destination}. Move it aside before rerunning; it may contain your edits.")
        destination.parent.mkdir(parents=True, exist_ok=True)
        stage = Path(tempfile.mkdtemp(prefix=f"{side}-incomplete-", dir=destination.parent))
        command = [java, f"-Xmx{args.heap}", "-jar", str(tool), "--folder",
                   "--skip-extra-files=1", "--log-level=warn", f"--thread-count={args.threads}"]
        command += [f"--add-external={ROOT / p}" for p in prepared["libraries"][side]]
        command += [str(jar), str(stage)]
        log = directory / "reports" / f"{side}-decompiler.log"
        print(f"Decompiling {side}; log: {log}", flush=True)
        with log.open("w", encoding="utf-8") as stream:
            result = subprocess.run(command, stdout=stream, stderr=subprocess.STDOUT)
        if result.returncode:
            raise ValueError(f"Decompiler exited {result.returncode}. Partial output retained at {stage}; see {log}")
        stage.rename(destination)
        report = coverage(directory, side)
        report["java_version"] = (version.stderr + version.stdout).strip()
        report["vineflower_version"] = vf["version"]
        report["command"] = command
        write_json(directory / "reports" / f"{side}-coverage.json", report)
        print(json.dumps({k: v for k, v in report.items() if k not in {"command", "java_version"}}, indent=2), flush=True)
        incomplete |= bool(report["missing_source_files"] or report["files_with_decompiler_error_markers"])
    return 2 if incomplete else 0


def status():
    _, directory = config()
    incomplete = False
    for side in ["client", "server"]:
        report = coverage(directory, side)
        print(json.dumps(report, indent=2))
        incomplete |= bool(report["missing_source_files"] or report["files_with_decompiler_error_markers"])
    return 2 if incomplete else 0


def add_game_arguments(parser):
    parser.add_argument("--minecraft-dir", type=Path,
                        help="official launcher game directory with your installed copy (default: the launcher's own)")
    parser.add_argument("--server-jar", type=Path, help="the pinned server jar you downloaded yourself")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    preparer = commands.add_parser("prepare", help="Import your own pinned game files, fetch the decompiler; inventory classes")
    add_game_arguments(preparer)
    decompiler = commands.add_parser("decompile", help="Decompile locally and check source-file coverage")
    decompiler.add_argument("--side", choices=["client", "server", "both"], default="both")
    decompiler.add_argument("--java", default="java")
    decompiler.add_argument("--threads", type=int, default=4)
    decompiler.add_argument("--heap", default="6G")
    commands.add_parser("status", help="Check source coverage and decompiler error markers")
    args = parser.parse_args()
    if getattr(args, "threads", 1) < 1:
        parser.error("--threads must be positive")
    if not re.fullmatch(r"[1-9][0-9]*[MGmg]", getattr(args, "heap", "6G")):
        parser.error("--heap must look like 6G or 512M")
    try:
        if args.command == "prepare":
            prepare(args.minecraft_dir, args.server_jar)
            return 0
        if args.command == "decompile":
            return decompile(args)
        return status()
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError, zipfile.BadZipFile) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
