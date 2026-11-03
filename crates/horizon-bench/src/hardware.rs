//! The machine a report was measured on. CPU, RAM and OS are detected here; the GPU and the display
//! come from the program that rendered (the game knows its adapter, the launcher its window).

use horizon_telemetry::system::command_output as output;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Hardware {
    /// `macos`, `linux`, `windows`.
    pub os: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_version: Option<String>,
    pub arch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_threads: Option<usize>,
    /// Physical cores.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_cores: Option<u16>,
    /// Base and top boost clock, where the OS gives them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_base_mhz: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_boost_mhz: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ram_mb: Option<u64>,
    /// The drive under the game folder: `nvme`, `ssd` or `hdd`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<String>,
    /// `laptop` or `desktop`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu: Option<Gpu>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<Display>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Gpu {
    pub name: String,
    pub vendor: String,
    pub driver: String,
    /// `metal`, `vulkan`, `dx12`, `gl`.
    pub backend: String,
    /// `integrated`, `discrete`, `virtual`, `cpu`, `other`.
    pub kind: String,
    /// Its own memory in MiB (rounded to 256), for a discrete GPU whose driver says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vram_mb: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Display {
    /// Physical pixels.
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_hz: Option<u32>,
}

impl Hardware {
    pub fn detect() -> Hardware {
        let probes = probes();
        Hardware {
            os: std::env::consts::OS.to_owned(),
            os_version: os_version(),
            arch: std::env::consts::ARCH.to_owned(),
            cpu: cpu(),
            cpu_threads: std::thread::available_parallelism().ok().map(|n| n.get()),
            cpu_cores: probes.cores,
            cpu_base_mhz: probes.base,
            cpu_boost_mhz: probes.boost,
            ram_mb: ram_bytes().map(|b| b >> 20),
            storage: None,
            form: probes.form,
            gpu: None,
            display: None,
        }
    }

    /// [`Hardware::detect`] with the kind of drive `game_dir` is on.
    pub fn detect_in(game_dir: &std::path::Path) -> Hardware {
        Hardware {
            storage: horizon_telemetry::system::storage(game_dir).map(str::to_owned),
            ..Hardware::detect()
        }
    }
}

/// What the telemetry probes read (`horizon_telemetry::system`).
struct Probes {
    cores: Option<u16>,
    base: Option<u32>,
    boost: Option<u32>,
    form: Option<String>,
}

fn probes() -> Probes {
    let s = horizon_telemetry::payload::System::detect();
    let some = |n: u32| (n > 0).then_some(n);
    Probes {
        cores: (s.cores > 0).then_some(s.cores),
        base: some(s.cpu_base_mhz),
        boost: some(s.cpu_boost_mhz),
        form: (!s.form.is_empty()).then_some(s.form),
    }
}

/// The one-minute load average: above about the number of threads the machine was busy.
pub fn load_average() -> Option<f64> {
    if cfg!(target_os = "macos") {
        // "{ 1.23 1.10 1.00 }"
        output("sysctl", &["-n", "vm.loadavg"])?
            .trim_start_matches('{')
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    } else {
        std::fs::read_to_string("/proc/loadavg")
            .ok()?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    }
}

/// `ac` or `battery`; `None` where it can't be read (and on machines without a battery on Linux).
pub fn power_source() -> Option<&'static str> {
    horizon_telemetry::system::power()
}

/// Resident memory of a process, in bytes (macOS, Linux).
pub fn rss_bytes(pid: u32) -> Option<u64> {
    if cfg!(target_os = "macos") {
        return output("ps", &["-o", "rss=", "-p", &pid.to_string()])?
            .parse::<u64>()
            .ok()
            .map(|kb| kb * 1024);
    }
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let line = status.lines().find(|l| l.starts_with("VmRSS:"))?;
    line.split_whitespace()
        .nth(1)?
        .parse::<u64>()
        .ok()
        .map(|kb| kb * 1024)
}

/// CPU time a process has used, user and system, in seconds (macOS, Linux).
pub fn cpu_seconds(pid: u32) -> Option<f64> {
    if cfg!(target_os = "macos") {
        // "[[dd-]hh:]mm:ss.cc"
        let time = output("ps", &["-o", "time=", "-p", &pid.to_string()])?;
        let (rest, days) = match time.split_once('-') {
            Some((d, rest)) => (rest.to_owned(), d.parse::<f64>().ok()?),
            None => (time, 0.0),
        };
        let seconds = rest.split(':').try_fold(0.0, |acc, part| {
            part.parse::<f64>().ok().map(|v| acc * 60.0 + v)
        })?;
        return Some(days * 86_400.0 + seconds);
    }
    // utime and stime, fields 14 and 15, in clock ticks (100 per second on Linux).
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    let ticks: f64 = fields.get(11)?.parse::<f64>().ok()? + fields.get(12)?.parse::<f64>().ok()?;
    Some(ticks / 100.0)
}

#[cfg(target_os = "macos")]
fn os_version() -> Option<String> {
    output("sw_vers", &["-productVersion"])
}

#[cfg(target_os = "macos")]
fn cpu() -> Option<String> {
    output("sysctl", &["-n", "machdep.cpu.brand_string"])
}

#[cfg(target_os = "macos")]
fn ram_bytes() -> Option<u64> {
    output("sysctl", &["-n", "hw.memsize"])?.parse().ok()
}

#[cfg(target_os = "linux")]
fn os_version() -> Option<String> {
    let release = std::fs::read_to_string("/etc/os-release").ok()?;
    let name = key_value(&release, "PRETTY_NAME", '=')?
        .trim_matches('"')
        .to_owned();
    Some(match output("uname", &["-r"]) {
        Some(kernel) => format!("{name} (kernel {kernel})"),
        None => name,
    })
}

#[cfg(target_os = "linux")]
fn cpu() -> Option<String> {
    let info = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    // x86 names the CPU per core; arm64 only gives the core's implementer and part.
    horizon_telemetry::system::linux_cpu_name(&info)
}

#[cfg(target_os = "linux")]
fn ram_bytes() -> Option<u64> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb = key_value(&info, "MemTotal", ':')?
        .trim_end_matches("kB")
        .trim();
    kb.parse::<u64>().ok().map(|kb| kb * 1024)
}

#[cfg(target_os = "linux")]
fn key_value<'a>(text: &'a str, key: &str, sep: char) -> Option<&'a str> {
    text.lines().find_map(|line| {
        let (k, v) = line.split_once(sep)?;
        (k.trim() == key).then(|| v.trim())
    })
}

#[cfg(target_os = "windows")]
fn os_version() -> Option<String> {
    horizon_telemetry::system::windows_version()
}

#[cfg(target_os = "windows")]
fn cpu() -> Option<String> {
    horizon_telemetry::system::windows_cpu()
}

#[cfg(target_os = "windows")]
fn ram_bytes() -> Option<u64> {
    let out = output(
        "powershell",
        &[
            "-NoProfile",
            "-Command",
            "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory",
        ],
    )?;
    out.parse().ok()
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn os_version() -> Option<String> {
    None
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn cpu() -> Option<String> {
    None
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn ram_bytes() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_this_machine() {
        let hw = Hardware::detect();
        assert_eq!(hw.os, std::env::consts::OS);
        assert!(hw.cpu_threads.unwrap() >= 1);
        if cfg!(any(target_os = "macos", target_os = "linux")) {
            assert!(hw.cpu.is_some(), "{hw:?}");
            assert!(hw.ram_mb.unwrap() > 256, "{hw:?}");
            assert!(hw.os_version.is_some(), "{hw:?}");
            assert!(load_average().is_some());
            assert!(rss_bytes(std::process::id()).unwrap() > 1 << 20);
            assert!(cpu_seconds(std::process::id()).unwrap() >= 0.0);
        }
    }

    /// `hardware.cpu` is the brand name, not the family and model `PROCESSOR_IDENTIFIER` holds.
    #[cfg(windows)]
    #[test]
    fn windows_reports_the_cpu_brand() {
        let cpu = Hardware::detect().cpu.expect("a CPU name");
        assert!(
            !cpu.is_empty() && !cpu.contains(" Family ") && !cpu.contains("Stepping"),
            "family and model, not a name: {cpu:?}"
        );
        assert_eq!(cpu, cpu.trim());
    }
}
