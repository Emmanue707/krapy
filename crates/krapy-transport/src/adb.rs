use std::ffi::OsStr;
use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

const RUN_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub serial: String,
    pub state: String,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdnsService {
    pub name: String,
    pub service_type: String,
    pub address: String,
}

pub fn on_path() -> bool {
    let paths = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&paths) {
        if dir.join(if cfg!(windows) { "adb.exe" } else { "adb" }).is_file() {
            return true;
        }
    }
    false
}

fn run(args: impl IntoIterator<Item = impl AsRef<OsStr>>, timeout: Duration) -> Result<Output> {
    let mut child = Command::new("adb")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawn adb")?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().context("adb wait")? {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            if let Some(mut out) = child.stdout.take() {
                out.read_to_end(&mut stdout).ok();
            }
            if let Some(mut err) = child.stderr.take() {
                err.read_to_end(&mut stderr).ok();
            }
            return Ok(Output {
                status,
                stdout,
                stderr,
            });
        }
        if start.elapsed() >= timeout {
            child.kill().ok();
            let _ = child.wait();
            bail!("adb timed out");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn require_ok(out: Output, ctx: &str) -> Result<String> {
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        Ok(stdout)
    } else {
        let msg = if stderr.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            stderr.trim().to_string()
        };
        bail!("{ctx}: {msg}");
    }
}

pub fn version() -> Result<String> {
    let out = run(["version"], RUN_TIMEOUT)?;
    let text = require_ok(out, "adb version")?;
    Ok(text.lines().next().unwrap_or(&text).trim().to_string())
}

pub fn devices() -> Result<Vec<Device>> {
    let out = run(["devices", "-l"], RUN_TIMEOUT)?;
    let text = require_ok(out, "adb devices")?;
    Ok(parse_devices_list(&text))
}

pub fn mdns_services() -> Result<Vec<MdnsService>> {
    let out = run(["mdns", "services"], RUN_TIMEOUT)?;
    let text = require_ok(out, "adb mdns")?;
    Ok(parse_mdns_services(&text))
}

pub fn pair(host_port: &str, code: &str) -> Result<()> {
    let out = run(["pair", host_port, code], RUN_TIMEOUT)?;
    require_ok(out, "adb pair")?;
    Ok(())
}

pub fn connect(host_port: &str) -> Result<()> {
    let out = run(["connect", host_port], RUN_TIMEOUT)?;
    require_ok(out, "adb connect")?;
    Ok(())
}

pub fn launch_app(serial: &str, uri: &str) -> Result<()> {
    let out = run(
        [
            "-s",
            serial,
            "shell",
            "am",
            "start",
            "-a",
            "android.intent.action.VIEW",
            "-d",
            uri,
        ],
        RUN_TIMEOUT,
    )?;
    require_ok(out, "am start")?;
    Ok(())
}

pub fn pick_serial(preferred: Option<&str>) -> Result<String> {
    if let Some(s) = preferred.filter(|s| !s.is_empty()) {
        return Ok(s.to_string());
    }
    let devs = devices()?;
    let ready: Vec<_> = devs.iter().filter(|d| d.state == "device").collect();
    match ready.len() {
        0 => bail!("no adb device in 'device' state"),
        1 => Ok(ready[0].serial.clone()),
        n => bail!("{n} adb devices — pass --adb <serial> or krapy pair --adb SERIAL"),
    }
}

fn tag_value<'a>(tokens: impl IntoIterator<Item = &'a str>, key: &str) -> Option<String> {
    tokens
        .into_iter()
        .find_map(|t| t.strip_prefix(key).map(|v| v.to_string()))
        .filter(|v| !v.is_empty())
}

pub fn parse_devices_list(text: &str) -> Vec<Device> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("List of devices") {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(serial) = parts.next() else {
            continue;
        };
        let Some(state) = parts.next() else {
            continue;
        };
        let rest: Vec<&str> = parts.collect();
        let model = tag_value(rest.iter().copied(), "model:");
        out.push(Device {
            serial: serial.to_string(),
            state: state.to_string(),
            model,
        });
    }
    out
}

pub fn parse_mdns_services(text: &str) -> Vec<MdnsService> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with("List of")
            || line.eq_ignore_ascii_case("mdns services:")
        {
            continue;
        }
        let cols: Vec<&str> = line.split_whitespace().collect();
        let address = cols
            .iter()
            .rev()
            .find(|c| c.contains(':') && !c.starts_with('_'))
            .map(|s| s.to_string());
        let Some(address) = address else {
            continue;
        };
        let (name, service_type) = match cols.len() {
            0..=1 => continue,
            2 => (cols[0].to_string(), String::new()),
            _ => {
                let name = cols[0].to_string();
                let ty = cols
                    .get(1)
                    .filter(|t| t.starts_with('_'))
                    .unwrap_or(&"")
                    .to_string();
                (name, ty)
            }
        };
        out.push(MdnsService {
            name,
            service_type,
            address,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEVICES_SAMPLE: &str = "List of devices attached\n\
emulator-5554          device product:sdk model:sdk_gphone device:generic transport_id:1\n\
192.168.0.50:5555      offline product:pixel model:Pixel_8 device:shiba transport_id:2\n\
ZX1G12345            unauthorized usb:2-1 model:Pixel_7\n\n";

    const MDNS_SAMPLE: &str = "List of discovered mdns services\n\
pixel-8    _adb-tls-pairing._tcp    192.168.0.12:37123\n\
pixel-8    _adb-tls-connect._tcp    192.168.0.12:37124\n";

    #[test]
    fn parse_devices_sample() {
        let devs = parse_devices_list(DEVICES_SAMPLE);
        assert_eq!(devs.len(), 3);
        assert_eq!(
            devs[0],
            Device {
                serial: "emulator-5554".into(),
                state: "device".into(),
                model: Some("sdk_gphone".into()),
            }
        );
        assert_eq!(devs[1].state, "offline");
        assert_eq!(devs[1].model.as_deref(), Some("Pixel_8"));
        assert_eq!(devs[2].state, "unauthorized");
    }

    #[test]
    fn parse_mdns_sample() {
        let svc = parse_mdns_services(MDNS_SAMPLE);
        assert_eq!(svc.len(), 2);
        assert_eq!(svc[0].address, "192.168.0.12:37123");
        assert_eq!(svc[0].service_type, "_adb-tls-pairing._tcp");
        assert_eq!(svc[1].address, "192.168.0.12:37124");
    }

    #[test]
    fn parse_devices_skips_banner() {
        assert!(parse_devices_list("List of devices attached\n").is_empty());
    }
}