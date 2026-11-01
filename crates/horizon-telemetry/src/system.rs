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
        command_output("cmd", &["/C", "ver"])
    } else {
        None
    }
}

fn cpu() -> Option<String> {
    if cfg!(target_os = "linux") {
        read("/proc/cpuinfo").and_then(|t| field(&t, "model name"))
    } else if cfg!(target_os = "macos") {
        command_output("sysctl", &["-n", "machdep.cpu.brand_string"])
    } else {
        std::env::var("PROCESSOR_IDENTIFIER").ok()
    }
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
