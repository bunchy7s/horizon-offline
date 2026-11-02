//! The machine as an item describes it ([`System`]): what the OS says, read the same way on
//! every start. The game adds the GPU (its name, maker, kind, driver and backend, then
//! [`vram_mb`] for it) and the screen.
//!
//! Only what explains performance: no serial number, host name, user name or path is read into
//! an item. The slower probes (a command to run) are read once per process.
use std::path::Path;
use std::sync::OnceLock;

use crate::payload::System;

/// What `program args` prints, trimmed, when it succeeds and prints something. On Windows the
/// program opens no console window: a GUI launcher or a game started without one would flash a
/// terminal for every probe.
pub fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let mut command = std::process::Command::new(program);
    command.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let out = command.output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (out.status.success() && !text.is_empty()).then_some(text)
}

/// `Windows 11 (26100.2033)`: the build numbers from the registry. `cmd /C ver`
/// prints the display language's own word for "Version" in the console's OEM
/// code page, so the same build read as UTF-8 carries U+FFFD in it and differs
/// between two machines running the same build.
pub fn windows_version() -> Option<String> {
    if !cfg!(windows) {
        return None;
    }
    const KEY: &str = r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    let value = |name: &str| command_output("reg", &["query", KEY, "/v", name]);
    windows_version_text(&value("CurrentBuild")?, value("UBR").as_deref())
}

/// The build and its update revision from the last column of two `reg query`
/// replies (`CurrentBuild    REG_SZ    26100`, `UBR    REG_DWORD    0x7f1`).
/// Build 22000 is the first Windows 11.
fn windows_version_text(build: &str, ubr: Option<&str>) -> Option<String> {
    let build = number(last_column(build)?)?;
    let release = if build >= 22_000 { "11" } else { "10" };
    Some(match ubr.and_then(last_column).and_then(number) {
        Some(ubr) => format!("Windows {release} ({build}.{ubr})"),
        None => format!("Windows {release} ({build})"),
    })
}

/// The value a `reg query` reply prints in its last column.
fn last_column(reply: &str) -> Option<&str> {
    reply.split_whitespace().last()
}

/// A decimal, or the `0x`-prefixed hex a `REG_DWORD` prints.
fn number(text: &str) -> Option<u32> {
    match text.strip_prefix("0x") {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => text.parse().ok(),
    }
}

/// The CPU's name from a Linux `/proc/cpuinfo`: x86's `model name`; else an ARM core named by
/// its implementer and part (an arm64 kernel prints only `CPU implementer: 0x41` and `CPU part:
/// 0xd0c`, as `lscpu` maps them: `ARM Neoverse-N1`); else the board's `Model` (a Raspberry Pi's
/// downstream kernel).
pub fn linux_cpu_name(cpuinfo: &str) -> Option<String> {
    if let Some(name) = field(cpuinfo, "model name").filter(|n| !n.is_empty()) {
        return Some(name);
    }
    let hex = |key: &str| {
        let text = field(cpuinfo, key)?;
        u32::from_str_radix(text.strip_prefix("0x")?, 16).ok()
    };
    if let (Some(implementer), Some(part)) = (hex("CPU implementer"), hex("CPU part")) {
        return Some(arm_core_name(implementer, part));
    }
    field(cpuinfo, "Model").filter(|n| !n.is_empty())
}

/// `lscpu`'s `hw_implementer` and `arm_part` tables, for the cores that run Horizon.
fn arm_core_name(implementer: u32, part: u32) -> String {
    let (vendor, core) = match (implementer, part) {
        (0x41, 0xd01) => ("ARM", "Cortex-A32"),
        (0x41, 0xd03) => ("ARM", "Cortex-A53"),
        (0x41, 0xd04) => ("ARM", "Cortex-A35"),
        (0x41, 0xd05) => ("ARM", "Cortex-A55"),
        (0x41, 0xd07) => ("ARM", "Cortex-A57"),
        (0x41, 0xd08) => ("ARM", "Cortex-A72"),
        (0x41, 0xd09) => ("ARM", "Cortex-A73"),
        (0x41, 0xd0a) => ("ARM", "Cortex-A75"),
        (0x41, 0xd0b) => ("ARM", "Cortex-A76"),
        (0x41, 0xd0c) => ("ARM", "Neoverse-N1"),
        (0x41, 0xd0d) => ("ARM", "Cortex-A77"),
        (0x41, 0xd40) => ("ARM", "Neoverse-V1"),
        (0x41, 0xd41) => ("ARM", "Cortex-A78"),
        (0x41, 0xd44) => ("ARM", "Cortex-X1"),
        (0x41, 0xd46) => ("ARM", "Cortex-A510"),
        (0x41, 0xd47) => ("ARM", "Cortex-A710"),
        (0x41, 0xd48) => ("ARM", "Cortex-X2"),
        (0x41, 0xd49) => ("ARM", "Neoverse-N2"),
        (0x41, 0xd4d) => ("ARM", "Cortex-A715"),
        (0x41, 0xd4e) => ("ARM", "Cortex-X3"),
        (0x41, 0xd4f) => ("ARM", "Neoverse-V2"),
        (0x41, 0xd80) => ("ARM", "Cortex-A520"),
        (0x41, 0xd81) => ("ARM", "Cortex-A720"),
        (0x41, 0xd82) => ("ARM", "Cortex-X4"),
        (0x41, 0xd84) => ("ARM", "Neoverse-V3"),
        (0x41, 0xd8e) => ("ARM", "Neoverse-N3"),
        (0x41, _) => ("ARM", ""),
        (0x42, _) => ("Broadcom", ""),
        (0x43, _) => ("Cavium", ""),
        (0x46, _) => ("Fujitsu", ""),
        (0x48, _) => ("HiSilicon", ""),
        (0x4e, _) => ("NVIDIA", ""),
        (0x50, _) => ("APM", ""),
        (0x51, _) => ("Qualcomm", ""),
        (0x53, _) => ("Samsung", ""),
        (0x61, _) => ("Apple", ""),
        (0xc0, _) => ("Ampere", ""),
        _ => ("ARM-compatible", ""),
    };
    if core.is_empty() {
        format!("{vendor} (CPU part {part:#05x})")
    } else {
        format!("{vendor} {core}")
    }
}

fn read(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// The value after `key` in a `key: value` or `key=value` listing.
fn field(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let rest = l.strip_prefix(key)?;
        let value = rest.trim_start().strip_prefix([':', '='])?;
        Some(value.trim().trim_matches('"').to_owned())
    })
}

fn os_version() -> Option<String> {
    if cfg!(target_os = "linux") {
        read("/etc/os-release").and_then(|t| field(&t, "PRETTY_NAME"))
    } else if cfg!(target_os = "macos") {
        command_output("sw_vers", &["-productVersion"]).map(|v| format!("macOS {v}"))
    } else if cfg!(windows) {
        windows_version()
    } else {
        None
    }
}

fn cpu() -> Option<String> {
    if cfg!(target_os = "linux") {
        read("/proc/cpuinfo").and_then(|t| linux_cpu_name(&t))
    } else if cfg!(target_os = "macos") {
        command_output("sysctl", &["-n", "machdep.cpu.brand_string"])
    } else if cfg!(windows) {
        windows_cpu()
    } else {
        None
    }
}

/// The CPU's brand string on Windows (`AMD Ryzen 7 7700X 8-Core Processor`), from the registry's
/// `ProcessorNameString`. `PROCESSOR_IDENTIFIER` is only the fallback: it always holds the family,
/// model and stepping (`AMD64 Family 25 Model 97 Stepping 2, AuthenticAMD`), which neither names the
/// chip nor tells two chips of one family apart. The game, its crash report, the benchmark report
/// and telemetry all read the CPU through this. Read once: every telemetry item and benchmark
/// report would run `reg` again.
pub fn windows_cpu() -> Option<String> {
    static CPU: OnceLock<Option<String>> = OnceLock::new();
    CPU.get_or_init(|| {
        command_output(
            "reg",
            &[
                "query",
                r"HKLM\HARDWARE\DESCRIPTION\System\CentralProcessor\0",
                "/v",
                "ProcessorNameString",
            ],
        )
        .and_then(|query| processor_name_from_reg_query(&query))
        .or_else(|| {
            std::env::var("PROCESSOR_IDENTIFIER")
                .ok()
                .map(|id| id.trim().to_owned())
                .filter(|id| !id.is_empty())
        })
    })
    .clone()
}

/// The value of `ProcessorNameString` in `reg query ... /v ProcessorNameString` output
/// (`    ProcessorNameString    REG_SZ    AMD Ryzen 7 7700X 8-Core Processor  `), trimmed: the
/// registry pads some names with spaces. Lines end in `\r\n`, and the key's own line has no value.
fn processor_name_from_reg_query(query: &str) -> Option<String> {
    query.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("ProcessorNameString")?;
        let value = rest.trim_start().strip_prefix("REG_SZ")?.trim();
        (!value.is_empty()).then(|| value.to_owned())
    })
}

fn ram_bytes() -> Option<u64> {
    if cfg!(target_os = "linux") {
        let kib = read("/proc/meminfo").and_then(|t| field(&t, "MemTotal"))?;
        kib.split_whitespace()
            .next()?
            .parse::<u64>()
            .ok()
            .map(|k| k * 1024)
    } else if cfg!(target_os = "macos") {
        command_output("sysctl", &["-n", "hw.memsize"])?
            .parse()
            .ok()
    } else if cfg!(windows) {
        command_output(
            "powershell",
            &[
                "-NoProfile",
                "-Command",
                "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory",
            ],
        )?
        .parse()
        .ok()
    } else {
        None
    }
}

/// The machine's hardware threads. On Linux the processors /proc/cpuinfo lists, as `cores` counts
/// them: `available_parallelism` is this process's share (its CPU affinity, a cgroup's quota), so a
/// game pinned to some CPUs would report fewer threads than cores.
fn threads() -> u16 {
    let processors = if cfg!(target_os = "linux") {
        read("/proc/cpuinfo").map_or(0, |t| {
            t.lines()
                .filter(|l| {
                    l.split_once(':')
                        .is_some_and(|(k, _)| k.trim() == "processor")
                })
                .count() as u16
        })
    } else {
        0
    };
    processors.max(std::thread::available_parallelism().map_or(0, |n| n.get() as u16))
}

/// Physical cores.
fn cores() -> Option<u16> {
    if cfg!(target_os = "linux") {
        cores_from_cpuinfo(&read("/proc/cpuinfo")?)
    } else if cfg!(target_os = "macos") {
        command_output("sysctl", &["-n", "hw.physicalcpu"])?
            .parse()
            .ok()
    } else if cfg!(windows) {
        powershell("(Get-CimInstance Win32_Processor | Measure-Object NumberOfCores -Sum).Sum")?
            .parse()
            .ok()
    } else {
        None
    }
}

/// Distinct (package, core) pairs; a listing without them (ARM boards) counts its processors.
fn cores_from_cpuinfo(text: &str) -> Option<u16> {
    let mut pairs = std::collections::HashSet::new();
    let (mut package, mut processors) = (String::new(), 0u16);
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        match key.trim() {
            "processor" => processors += 1,
            "physical id" => package = value.trim().to_owned(),
            "core id" => {
                pairs.insert((package.clone(), value.trim().to_owned()));
            }
            _ => {}
        }
    }
    let n = if pairs.is_empty() {
        processors
    } else {
        pairs.len() as u16
    };
    (n > 0).then_some(n)
}

/// The base and top boost clocks in MHz, where the OS gives them (not on Apple silicon).
fn cpu_mhz() -> (Option<u32>, Option<u32>) {
    let khz = |path: &str| {
        read(path)
            .and_then(|t| t.trim().parse::<u32>().ok())
            .map(|k| k / 1000)
    };
    if cfg!(target_os = "linux") {
        let base = khz("/sys/devices/system/cpu/cpu0/cpufreq/base_frequency").or_else(|| {
            read("/sys/devices/system/cpu/cpu0/acpi_cppc/nominal_freq")
                .and_then(|t| t.trim().parse().ok())
        });
        (
            base,
            khz("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq"),
        )
    } else if cfg!(target_os = "macos") {
        let hz = |key: &str| {
            command_output("sysctl", &["-n", key])
                .and_then(|t| t.parse::<u64>().ok())
                .map(|hz| (hz / 1_000_000) as u32)
        };
        (hz("hw.cpufrequency"), hz("hw.cpufrequency_max"))
    } else if cfg!(windows) {
        let base =
            powershell("(Get-CimInstance Win32_Processor | Select-Object -First 1).MaxClockSpeed")
                .and_then(|t| t.parse().ok());
        (base, None)
    } else {
        (None, None)
    }
}

fn powershell(command: &str) -> Option<String> {
    command_output("powershell", &["-NoProfile", "-Command", command])
}

/// `nvme`, `ssd` or `hdd` for the drive `path` is on, where it can be read.
pub fn storage(path: &Path) -> Option<&'static str> {
    if cfg!(target_os = "linux") {
        let source = mount_source(&read("/proc/self/mountinfo")?, path)?;
        let name = source.strip_prefix("/dev/")?;
        // /dev/mapper/x and /dev/dm-0 (LUKS, LVM) sit on the disks in their slaves/.
        let name = std::fs::canonicalize(format!("/dev/{name}"))
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| name.to_owned());
        linux_disk_kind(&name, 0)
    } else if cfg!(target_os = "macos") {
        let df = command_output("df", &[&path.to_string_lossy()])?;
        let device = df.lines().nth(1)?.split_whitespace().next()?.to_owned();
        let info = command_output("diskutil", &["info", &device])?;
        disk_kind_from_diskutil(&info)
    } else if cfg!(windows) {
        let letter = path.to_string_lossy().chars().next()?.to_ascii_uppercase();
        if !letter.is_ascii_alphabetic() {
            return None;
        }
        let out = powershell(&format!(
            "$d = (Get-Partition -DriveLetter {letter}).DiskNumber; \
             $p = Get-PhysicalDisk | Where-Object DeviceId -eq $d; \"$($p.BusType) $($p.MediaType)\""
        ))?;
        let out = out.to_ascii_lowercase();
        if out.contains("nvme") {
            Some("nvme")
        } else if out.contains("ssd") {
            Some("ssd")
        } else if out.contains("hdd") {
            Some("hdd")
        } else {
            None
        }
    } else {
        None
    }
}

/// The device mounted at the longest mount point above `path`, from `/proc/self/mountinfo`.
fn mount_source(mountinfo: &str, path: &Path) -> Option<String> {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
    mountinfo
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let point = fields.get(4)?;
            let dash = fields.iter().position(|f| *f == "-")?;
            let source = fields.get(dash + 2)?;
            path.starts_with(point)
                .then(|| (point.len(), source.to_string()))
        })
        .max_by_key(|(len, _)| *len)
        .map(|(_, source)| source)
}

fn linux_disk_kind(name: &str, depth: u8) -> Option<&'static str> {
    if depth > 4 {
        return None;
    }
    let class = Path::new("/sys/class/block").join(name);
    // A mapper device: the first disk under it.
    if let Ok(slaves) = std::fs::read_dir(class.join("slaves"))
        && let Some(slave) = slaves.flatten().next()
    {
        return linux_disk_kind(&slave.file_name().to_string_lossy(), depth + 1);
    }
    // A partition: its disk is the parent directory.
    let device = std::fs::canonicalize(&class).ok()?;
    let disk = if device.join("partition").exists() {
        device.parent()?.to_owned()
    } else {
        device
    };
    let disk_name = disk.file_name()?.to_string_lossy();
    if disk_name.starts_with("nvme") {
        return Some("nvme");
    }
    match std::fs::read_to_string(disk.join("queue/rotational"))
        .ok()?
        .trim()
    {
        "0" => Some("ssd"),
        "1" => Some("hdd"),
        _ => None,
    }
}

fn disk_kind_from_diskutil(info: &str) -> Option<&'static str> {
    let solid = field_spaced(info, "Solid State")?;
    if solid != "Yes" {
        return Some("hdd");
    }
    let protocol = field_spaced(info, "Protocol").unwrap_or_default();
    Some(
        if ["PCI-Express", "Apple Fabric", "NVMe"]
            .iter()
            .any(|p| protocol.contains(p))
        {
            "nvme"
        } else {
            "ssd"
        },
    )
}

/// `   Key:     value` listings (diskutil, system_profiler).
fn field_spaced(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        (k.trim() == key).then(|| v.trim().to_owned())
    })
}

/// `ac` or `battery`; `None` where it can't be read.
pub fn power() -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        let batt = command_output("pmset", &["-g", "batt"])?;
        return Some(if batt.contains("'Battery Power'") {
            "battery"
        } else {
            "ac"
        });
    }
    if cfg!(windows) {
        // BatteryStatus 1: discharging.
        let status =
            powershell("(Get-CimInstance Win32_Battery | Select-Object -First 1).BatteryStatus")?;
        return Some(if status.trim() == "1" {
            "battery"
        } else {
            "ac"
        });
    }
    let mut on_battery = None;
    for entry in std::fs::read_dir("/sys/class/power_supply").ok()?.flatten() {
        let read =
            |name: &str| std::fs::read_to_string(entry.path().join(name)).unwrap_or_default();
        match read("type").trim() {
            "Mains" if read("online").trim() == "1" => return Some("ac"),
            "Battery" => on_battery = Some("battery"),
            _ => {}
        }
    }
    on_battery
}

/// `laptop` or `desktop`: the firmware's chassis type, else whether there is a battery.
fn form() -> Option<&'static str> {
    if cfg!(target_os = "linux") {
        if let Some(kind) =
            read("/sys/class/dmi/id/chassis_type").and_then(|t| t.trim().parse::<u8>().ok())
        {
            match kind {
                8 | 9 | 10 | 14 | 30 | 31 | 32 => return Some("laptop"),
                3..=7 | 13 | 15 | 16 | 35 | 36 => return Some("desktop"),
                _ => {}
            }
        }
        let battery = std::fs::read_dir("/sys/class/power_supply")
            .ok()?
            .flatten()
            .any(|e| {
                std::fs::read_to_string(e.path().join("type")).is_ok_and(|t| t.trim() == "Battery")
            });
        Some(if battery { "laptop" } else { "desktop" })
    } else if cfg!(target_os = "macos") {
        let batt = command_output("pmset", &["-g", "batt"])?;
        Some(if batt.contains("InternalBattery") {
            "laptop"
        } else {
            "desktop"
        })
    } else if cfg!(windows) {
        let n = powershell("@(Get-CimInstance Win32_Battery).Count")?;
        Some(if n.trim() == "0" { "desktop" } else { "laptop" })
    } else {
        None
    }
}

/// The dedicated memory of the GPU from `vendor` (`nvidia`, `amd`, `intel`, `apple`), in MiB
/// rounded to 256, where the driver says; read once. `None` for a GPU sharing the system's.
pub fn vram_mb(vendor: &str) -> Option<u32> {
    static CACHE: OnceLock<std::sync::Mutex<std::collections::HashMap<String, Option<u32>>>> =
        OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    *cache.entry(vendor.to_ascii_lowercase()).or_insert_with(|| {
        read_vram_mb(&vendor.to_ascii_lowercase()).map(|mb| mb.div_ceil(256) * 256)
    })
}

fn read_vram_mb(vendor: &str) -> Option<u32> {
    if cfg!(target_os = "linux") {
        if vendor.contains("nvidia") {
            return command_output(
                "nvidia-smi",
                &["--query-gpu=memory.total", "--format=csv,noheader,nounits"],
            )?
            .lines()
            .filter_map(|l| l.trim().parse::<u32>().ok())
            .max();
        }
        // amdgpu (and Intel's discrete xe/i915 cards) in sysfs.
        std::fs::read_dir("/sys/class/drm")
            .ok()?
            .flatten()
            .filter_map(|e| {
                let dev = e.path().join("device");
                let bytes = read(&dev.join("mem_info_vram_total").to_string_lossy())
                    .or_else(|| read(&dev.join("lmem_total_bytes").to_string_lossy()))?;
                bytes.trim().parse::<u64>().ok().map(|b| (b >> 20) as u32)
            })
            .filter(|mb| *mb >= 512)
            .max()
    } else if cfg!(target_os = "macos") {
        // Apple silicon shares the system's memory: no "VRAM" line.
        let text = command_output("system_profiler", &["SPDisplaysDataType"])?;
        text.lines()
            .filter_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.trim().starts_with("VRAM").then(|| size_mb(v))?
            })
            .max()
    } else if cfg!(windows) {
        // AdapterRAM stops at 4 GB; the driver's registry value doesn't.
        let out = powershell(
            "Get-ItemProperty 'HKLM:\\SYSTEM\\ControlSet001\\Control\\Class\\{4d36e968-e325-11ce-bfc1-08002be10318}\\0*' \
             -ErrorAction SilentlyContinue | ForEach-Object { \"$($_.DriverDesc)|$($_.'HardwareInformation.qwMemorySize')\" }",
        )?;
        out.lines()
            .filter_map(|l| {
                let (name, bytes) = l.split_once('|')?;
                let ok = vendor.is_empty()
                    || name.to_ascii_lowercase().contains(vendor)
                    || (vendor == "nvidia" && name.contains("GeForce"))
                    || (vendor == "amd" && name.contains("Radeon"));
                ok.then(|| bytes.trim().parse::<u64>().ok().map(|b| (b >> 20) as u32))?
            })
            .max()
    } else {
        None
    }
}

/// "8 GB", "1536 MB" -> MiB.
fn size_mb(text: &str) -> Option<u32> {
    let mut parts = text.split_whitespace();
    let n: f64 = parts.next()?.parse().ok()?;
    match parts.next()? {
        "GB" => Some((n * 1024.0) as u32),
        "MB" => Some(n as u32),
        _ => None,
    }
}

/// What doesn't change while the game runs, read once.
struct Fixed {
    cores: u16,
    base: u32,
    boost: u32,
    form: &'static str,
}

fn fixed() -> &'static Fixed {
    static FIXED: OnceLock<Fixed> = OnceLock::new();
    FIXED.get_or_init(|| {
        let (base, boost) = cpu_mhz();
        Fixed {
            cores: cores().unwrap_or(0),
            base: base.unwrap_or(0),
            boost: boost.unwrap_or(0),
            form: form().unwrap_or(""),
        }
    })
}

impl System {
    /// This machine, without the GPU and the screen. `home` is the game folder, for the kind of
    /// drive it is on.
    pub fn detect_in(home: Option<&Path>) -> Self {
        const GIB: f64 = (1u64 << 30) as f64;
        static STORAGE: OnceLock<&'static str> = OnceLock::new();
        let fixed = fixed();
        Self {
            os: std::env::consts::OS.to_owned(),
            os_version: os_version().unwrap_or_default(),
            arch: std::env::consts::ARCH.to_owned(),
            cpu: cpu().unwrap_or_default(),
            threads: threads(),
            cores: fixed.cores,
            cpu_base_mhz: fixed.base,
            cpu_boost_mhz: fixed.boost,
            ram_gb: ram_bytes().map_or(0, |b| (b as f64 / GIB).round() as u16),
            storage: home
                .map(|h| *STORAGE.get_or_init(|| storage(h).unwrap_or("")))
                .unwrap_or("")
                .to_owned(),
            power: power().unwrap_or("").to_owned(),
            form: fixed.form.to_owned(),
            ..Self::default()
        }
    }

    /// [`System::detect_in`] without the drive.
    pub fn detect() -> Self {
        Self::detect_in(None)
    }
}

/// The GPU's maker from its name and the vendor ID wgpu reports (0: none, as on Metal).
pub fn gpu_vendor(name: &str, vendor_id: u32) -> &'static str {
    match vendor_id {
        0x10de => return "nvidia",
        0x1002 | 0x1022 => return "amd",
        0x8086 => return "intel",
        0x106b => return "apple",
        0x5143 => return "qualcomm",
        _ => {}
    }
    let name = name.to_ascii_lowercase();
    [
        ("apple", "apple"),
        ("nvidia", "nvidia"),
        ("geforce", "nvidia"),
        ("radeon", "amd"),
        ("amd", "amd"),
        ("intel", "intel"),
        ("adreno", "qualcomm"),
        ("llvmpipe", "software"),
        ("lavapipe", "software"),
    ]
    .into_iter()
    .find(|(needle, _)| name.contains(needle))
    .map_or("", |(_, v)| v)
}

#[cfg(test)]
mod tests {
    #[test]
    fn an_arm64_kernels_cpuinfo_names_the_core_by_implementer_and_part() {
        // An arm64 kernel prints no `model name` (a Neoverse-N1 and a Cortex-A72 Pi 4).
        let neoverse = "processor\t: 0\nBogoMIPS\t: 50.00\nFeatures\t: fp asimd\nCPU implementer\t: 0x41\nCPU architecture: 8\nCPU variant\t: 0x3\nCPU part\t: 0xd0c\nCPU revision\t: 1\n";
        assert_eq!(linux_cpu_name(neoverse).as_deref(), Some("ARM Neoverse-N1"));
        let pi = "processor\t: 3\nCPU implementer\t: 0x41\nCPU part\t: 0xd08\nCPU revision\t: 3\n\nRevision\t: c03114\nModel\t\t: Raspberry Pi 4 Model B Rev 1.4\n";
        assert_eq!(linux_cpu_name(pi).as_deref(), Some("ARM Cortex-A72"));
        // An implementer the table has, a part it does not: the part is kept.
        let apple = "CPU implementer\t: 0x61\nCPU part\t: 0x023\n";
        assert_eq!(
            linux_cpu_name(apple).as_deref(),
            Some("Apple (CPU part 0x023)")
        );
    }

    #[test]
    fn x86_keeps_its_model_name_and_a_board_without_either_its_model() {
        let ryzen = "processor\t: 0\nmodel name\t: AMD Ryzen 9 7950X 16-Core Processor\n";
        assert_eq!(
            linux_cpu_name(ryzen).as_deref(),
            Some("AMD Ryzen 9 7950X 16-Core Processor")
        );
        assert_eq!(
            linux_cpu_name("Model\t: Some Board\n").as_deref(),
            Some("Some Board")
        );
        assert_eq!(linux_cpu_name("processor\t: 0\n"), None);
    }

    use super::*;

    #[test]
    fn fields_are_read_from_listings() {
        let os_release =
            "NAME=\"Fedora Linux\"\nPRETTY_NAME=\"Fedora Linux 43 (Workstation Edition)\"\n";
        assert_eq!(
            field(os_release, "PRETTY_NAME").as_deref(),
            Some("Fedora Linux 43 (Workstation Edition)")
        );
        let cpuinfo = "processor\t: 0\nmodel name\t: AMD Ryzen 9 7950X 16-Core Processor\n";
        assert_eq!(
            field(cpuinfo, "model name").as_deref(),
            Some("AMD Ryzen 9 7950X 16-Core Processor")
        );
        assert_eq!(
            field("MemTotal:       32788148 kB", "MemTotal").as_deref(),
            Some("32788148 kB")
        );
        assert_eq!(field("NAME=x", "PRETTY_NAME"), None);
    }

    #[test]
    fn the_windows_version_comes_from_the_build_numbers() {
        assert_eq!(
            windows_version_text(
                "    CurrentBuild    REG_SZ    26100",
                Some("    UBR    REG_DWORD    0x7f1")
            )
            .as_deref(),
            Some("Windows 11 (26100.2033)")
        );
        assert_eq!(
            windows_version_text("    CurrentBuild    REG_SZ    19045", None).as_deref(),
            Some("Windows 10 (19045)")
        );
        assert_eq!(windows_version_text("", None), None);
    }

    #[test]
    fn cores_are_counted_once_per_package() {
        let two_cores_four_threads = "processor : 0\nphysical id : 0\ncore id : 0\n\nprocessor : 1\nphysical id : 0\ncore id : 1\n\nprocessor : 2\nphysical id : 0\ncore id : 0\n\nprocessor : 3\nphysical id : 0\ncore id : 1\n";
        assert_eq!(cores_from_cpuinfo(two_cores_four_threads), Some(2));
        assert_eq!(
            cores_from_cpuinfo("processor : 0\nprocessor : 1\n"),
            Some(2)
        );
        assert_eq!(cores_from_cpuinfo(""), None);
    }

    #[test]
    fn the_game_folders_mount_is_found() {
        let mountinfo = "22 1 259:2 / / rw - ext4 /dev/nvme0n1p2 rw\n30 22 8:1 / /mnt/games rw - ext4 /dev/sda1 rw\n";
        assert_eq!(
            mount_source(mountinfo, Path::new("/home/x/.horizon")).as_deref(),
            Some("/dev/nvme0n1p2")
        );
        assert_eq!(
            mount_source(mountinfo, Path::new("/mnt/games/horizon")).as_deref(),
            Some("/dev/sda1")
        );
    }

    #[test]
    fn diskutil_and_sizes_are_read() {
        let info = "   Device Node:               /dev/disk3s1\n   Protocol:                  Apple Fabric\n   Solid State:               Yes\n";
        assert_eq!(disk_kind_from_diskutil(info), Some("nvme"));
        assert_eq!(
            disk_kind_from_diskutil("   Solid State:  No\n"),
            Some("hdd")
        );
        assert_eq!(
            disk_kind_from_diskutil("   Protocol: SATA\n   Solid State: Yes\n"),
            Some("ssd")
        );
        assert_eq!(size_mb(" 8 GB"), Some(8192));
        assert_eq!(size_mb(" 1536 MB"), Some(1536));
    }

    #[test]
    fn the_registry_cpu_name_is_read_from_reg_query() {
        let name = "AMD Ryzen 7 7700X 8-Core Processor";
        let key = r"HKEY_LOCAL_MACHINE\HARDWARE\DESCRIPTION\System\CentralProcessor\0";
        let query = format!("\r\n{key}\r\n    ProcessorNameString    REG_SZ    {name}\r\n\r\n");
        // CRLF line endings, as `reg query` prints them.
        assert_eq!(processor_name_from_reg_query(&query).as_deref(), Some(name));
        // The same with LF only.
        assert_eq!(
            processor_name_from_reg_query(&query.replace("\r\n", "\n")).as_deref(),
            Some(name)
        );
        // The registry pads some names with spaces; the name comes without them.
        let padded = format!("{key}\n    ProcessorNameString    REG_SZ        {name}      \n");
        assert_eq!(
            processor_name_from_reg_query(&padded).as_deref(),
            Some(name)
        );
        assert_eq!(
            processor_name_from_reg_query(&format!(
                "
{key}
    ProcessorNameString    REG_SZ    13th Gen Intel(R) Core(TM) i7-13700F
"
            ))
            .as_deref(),
            Some("13th Gen Intel(R) Core(TM) i7-13700F")
        );
        // Spaces inside the name stay as they are, runs of them too (older Intel names).
        assert_eq!(
            processor_name_from_reg_query(
                "    ProcessorNameString    REG_SZ    Intel(R) Core(TM) i7 CPU         920  @ 2.67GHz\r\n"
            )
            .as_deref(),
            Some("Intel(R) Core(TM) i7 CPU         920  @ 2.67GHz")
        );
        // Columns split by tabs.
        assert_eq!(
            processor_name_from_reg_query(&format!("\tProcessorNameString\tREG_SZ\t{name}\r\n"))
                .as_deref(),
            Some(name)
        );
    }

    #[test]
    fn a_reg_query_without_a_name_gives_none() {
        // The value is missing: `reg query` prints an error and nothing on stdout, or only the key.
        assert_eq!(processor_name_from_reg_query(""), None);
        assert_eq!(
            processor_name_from_reg_query(
                "HKEY_LOCAL_MACHINE\\HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0\r\n"
            ),
            None
        );
        // Present but empty.
        assert_eq!(
            processor_name_from_reg_query("    ProcessorNameString    REG_SZ    \r\n"),
            None
        );
        assert_eq!(
            processor_name_from_reg_query("    ProcessorNameString    REG_SZ"),
            None
        );
        // Another value, or another type, is not the name.
        assert_eq!(
            processor_name_from_reg_query(
                "    Identifier    REG_SZ    AMD64 Family 25 Model 97 Stepping 2\r\n"
            ),
            None
        );
        assert_eq!(
            processor_name_from_reg_query("    ProcessorNameString    REG_DWORD    0x1\r\n"),
            None
        );
        // Garbage.
        assert_eq!(processor_name_from_reg_query("REG_SZ\n\n   \n"), None);
        assert_eq!(
            processor_name_from_reg_query("\u{0}\u{fffd}\u{fffd}REG_SZ"),
            None
        );
        assert_eq!(
            processor_name_from_reg_query(
                "ERROR: The system was unable to find the specified registry key or value."
            ),
            None
        );
    }

    /// The CPU is the registry's name for it, as PowerShell reads it (not through `reg`), trimmed;
    /// `PROCESSOR_IDENTIFIER`'s family and model only where the registry has no name, as on some
    /// VMs. No CI runner tests this crate on Windows: it runs on a contributor's machine.
    #[cfg(windows)]
    #[test]
    fn windows_names_the_cpu_by_its_brand() {
        let registry = powershell(
            r"(Get-ItemProperty 'HKLM:\HARDWARE\DESCRIPTION\System\CentralProcessor\0').ProcessorNameString",
        );
        let expected = registry.or_else(|| {
            std::env::var("PROCESSOR_IDENTIFIER")
                .ok()
                .map(|id| id.trim().to_owned())
        });
        assert_eq!(cpu(), expected);
    }

    #[test]
    fn gpu_makers() {
        assert_eq!(gpu_vendor("NVIDIA GeForce RTX 4080", 0x10de), "nvidia");
        assert_eq!(gpu_vendor("Apple M4 Pro", 0), "apple");
        assert_eq!(gpu_vendor("AMD Radeon RX 7800 XT", 0), "amd");
        assert_eq!(
            gpu_vendor("Mesa Intel(R) HD Graphics 620 (KBL GT2)", 0x8086),
            "intel"
        );
        assert_eq!(gpu_vendor("llvmpipe (LLVM 19)", 0x10005), "software");
    }

    /// `cargo test -p horizon-telemetry -- --ignored --nocapture what_this_machine_sends`
    #[test]
    #[ignore]
    fn what_this_machine_sends() {
        let home = std::env::current_dir().unwrap();
        let mut system = System::detect_in(Some(&home));
        system.vram_mb = vram_mb("nvidia").or_else(|| vram_mb("amd")).unwrap_or(0);
        println!("{}", serde_json::to_string_pretty(&system).unwrap());
    }

    #[test]
    fn this_machine_is_described() {
        let system = System::detect();
        assert_eq!(system.os, std::env::consts::OS);
        assert!(system.threads > 0);
        assert!(system.gpu.is_empty(), "the game fills the GPU in");
        assert!(system.cores == 0 || system.cores <= system.threads);
        assert!(["", "ac", "battery"].contains(&system.power.as_str()));
        assert!(["", "laptop", "desktop"].contains(&system.form.as_str()));
        let dir = std::env::temp_dir();
        assert!(
            ["", "nvme", "ssd", "hdd"].contains(&System::detect_in(Some(&dir)).storage.as_str())
        );
    }
}
