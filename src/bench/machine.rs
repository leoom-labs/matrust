//! The machine line of a result: OS, CPU, cores and RAM, read without new
//! crates. A field that cannot be read is null; reading never fails the run.
use serde::{Deserialize, Serialize};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Machine {
    pub os: Option<String>,
    pub os_version: Option<String>,
    pub arch: String,
    pub cpu: Option<String>,
    pub logical_cores: Option<usize>,
    pub ram_gib: Option<u64>,
}

impl Machine {
    /// `Ubuntu 24.04, x86_64, AMD Ryzen 9 7950X, 32 logical cores, 62 GiB RAM`.
    pub fn line(&self) -> String {
        let os = match (&self.os, &self.os_version) {
            (Some(os), Some(version)) => format!("{os} {version}"),
            (Some(os), None) => os.clone(),
            (None, _) => "unknown OS".into(),
        };
        let mut parts = vec![os, self.arch.clone()];
        parts.push(self.cpu.clone().unwrap_or_else(|| "unknown CPU".into()));
        if let Some(cores) = self.logical_cores {
            parts.push(format!("{cores} logical cores"));
        }
        if let Some(ram) = self.ram_gib {
            parts.push(format!("{ram} GiB RAM"));
        }
        parts.join(", ")
    }
}

pub fn detect() -> Machine {
    let mut machine = Machine {
        arch: std::env::consts::ARCH.into(),
        ..Machine::default()
    };
    if cfg!(target_os = "linux") {
        let read = |path: &str| std::fs::read_to_string(path).unwrap_or_default();
        apply_linux(
            &mut machine,
            &read("/proc/cpuinfo"),
            &read("/proc/meminfo"),
            &read("/etc/os-release"),
        );
    } else if cfg!(target_os = "macos") {
        let output = |program: &str, arg: &str| {
            command_output(program, &[arg]).and_then(|text| clean(&text))
        };
        let sysctl = |name: &str| command_output("sysctl", &["-n", name]).and_then(|t| clean(&t));
        machine.os = output("sw_vers", "-productName").or(Some("macOS".into()));
        machine.os_version = output("sw_vers", "-productVersion");
        machine.cpu = sysctl("machdep.cpu.brand_string");
        machine.logical_cores = sysctl("hw.logicalcpu").and_then(|n| n.parse().ok());
        machine.ram_gib = sysctl("hw.memsize").and_then(|n| n.parse().ok()).map(gib);
    } else if cfg!(windows) {
        // One PowerShell call: OS caption, version, CPU name, logical cores, RAM
        // bytes. UTF-8 whatever the console code page; [string] prints an empty
        // line for a missing value, so the later values keep their lines.
        let script = "[Console]::OutputEncoding=[Text.Encoding]::UTF8; \
                      $o=Get-CimInstance Win32_OperatingSystem; \
                      $p=@(Get-CimInstance Win32_Processor)[0]; \
                      $s=Get-CimInstance Win32_ComputerSystem; \
                      [string]($o.Caption); [string]($o.Version); [string]($p.Name); \
                      [string]($s.NumberOfLogicalProcessors); [string]($s.TotalPhysicalMemory)";
        let output = command_output(
            "powershell",
            &["-NoProfile", "-NonInteractive", "-Command", script],
        );
        apply_windows(&mut machine, output.as_deref().unwrap_or_default());
    }
    if machine.logical_cores.is_none() {
        machine.logical_cores = std::thread::available_parallelism().ok().map(usize::from);
    }
    machine
}

/// Bytes to whole GiB, rounded.
fn gib(bytes: u64) -> u64 {
    (bytes as f64 / f64::from(1u32 << 30)).round() as u64
}

/// A printable one-line value, or `None`.
fn clean(text: &str) -> Option<String> {
    let text = text.trim().trim_matches('"').trim();
    (!text.is_empty() && text.len() <= 200 && !text.chars().any(char::is_control))
        .then(|| text.split_whitespace().collect::<Vec<_>>().join(" "))
}

pub(crate) fn apply_linux(machine: &mut Machine, cpuinfo: &str, meminfo: &str, os_release: &str) {
    let field = |text: &str, key: &str, separator: char| {
        text.lines().find_map(|line| {
            let (name, value) = line.split_once(separator)?;
            (name.trim() == key).then(|| clean(value)).flatten()
        })
    };
    machine.cpu = field(cpuinfo, "model name", ':');
    let processors = cpuinfo
        .lines()
        .filter(|line| line.split(':').next().map(str::trim) == Some("processor"))
        .count();
    machine.logical_cores = (processors > 0).then_some(processors);
    machine.ram_gib = field(meminfo, "MemTotal", ':')
        .and_then(|value| value.strip_suffix(" kB")?.parse::<u64>().ok())
        .map(|kib| gib(kib.saturating_mul(1024)));
    machine.os = field(os_release, "NAME", '=').or(Some("Linux".into()));
    machine.os_version = field(os_release, "VERSION_ID", '=');
}

pub(crate) fn apply_windows(machine: &mut Machine, output: &str) {
    let lines: Vec<&str> = output.lines().map(str::trim).collect();
    let line = |index: usize| lines.get(index).and_then(|line| clean(line));
    machine.os = line(0).or(Some("Windows".into()));
    machine.os_version = line(1);
    machine.cpu = line(2);
    machine.logical_cores = line(3).and_then(|n| n.parse().ok());
    machine.ram_gib = line(4).and_then(|n| n.parse().ok()).map(gib);
}

/// Standard output of a short helper program, lines kept in place; `None`
/// when empty, on any failure or after 20 s.
fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut child.stdout.take()?, &mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    (!text.trim().is_empty()).then(|| text.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_files() {
        let mut machine = Machine::default();
        apply_linux(
            &mut machine,
            "processor\t: 0\nmodel name\t: AMD Ryzen 9  7950X 16-Core Processor\n\n\
             processor\t: 1\nmodel name\t: AMD Ryzen 9  7950X 16-Core Processor\n",
            "MemTotal:       65762340 kB\nMemFree:  1 kB\n",
            "PRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\nNAME=\"Ubuntu\"\nVERSION_ID=\"24.04\"\n",
        );
        assert_eq!(
            machine,
            Machine {
                os: Some("Ubuntu".into()),
                os_version: Some("24.04".into()),
                arch: String::new(),
                cpu: Some("AMD Ryzen 9 7950X 16-Core Processor".into()),
                logical_cores: Some(2),
                ram_gib: Some(63),
            }
        );
        let mut bare = Machine::default();
        apply_linux(&mut bare, "", "", "");
        assert_eq!(
            (
                bare.os.as_deref(),
                bare.cpu,
                bare.ram_gib,
                bare.logical_cores
            ),
            (Some("Linux"), None, None, None)
        );
    }

    #[test]
    fn windows_output() {
        let mut machine = Machine::default();
        apply_windows(
            &mut machine,
            "Microsoft Windows 11 Pro\r\n10.0.22631\r\nIntel(R) Core(TM) i7-8700 CPU @ 3.20GHz\r\n12\r\n34211835904\r\n",
        );
        assert_eq!(machine.os.as_deref(), Some("Microsoft Windows 11 Pro"));
        assert_eq!(machine.os_version.as_deref(), Some("10.0.22631"));
        assert_eq!(
            machine.cpu.as_deref(),
            Some("Intel(R) Core(TM) i7-8700 CPU @ 3.20GHz")
        );
        assert_eq!(
            (machine.logical_cores, machine.ram_gib),
            (Some(12), Some(32))
        );
        let mut gaps = Machine::default();
        apply_windows(
            &mut gaps,
            "Microsoft Windows 10 Pro\r\n10.0.19045\r\n\r\n8\r\n17179869184\r\n",
        );
        assert_eq!(
            (gaps.cpu, gaps.logical_cores, gaps.ram_gib),
            (None, Some(8), Some(16))
        );
        let mut failed = Machine::default();
        apply_windows(&mut failed, "");
        assert_eq!((failed.os.as_deref(), failed.cpu), (Some("Windows"), None));
    }

    #[test]
    fn machine_line_and_detection() {
        let machine = Machine {
            os: Some("Ubuntu".into()),
            os_version: Some("24.04".into()),
            arch: "x86_64".into(),
            cpu: None,
            logical_cores: Some(8),
            ram_gib: None,
        };
        assert_eq!(
            machine.line(),
            "Ubuntu 24.04, x86_64, unknown CPU, 8 logical cores"
        );
        let here = detect();
        assert_eq!(here.arch, std::env::consts::ARCH);
        assert!(here.logical_cores.is_some_and(|n| n > 0));
    }
}
