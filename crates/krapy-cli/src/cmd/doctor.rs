use std::process;
use std::time::Duration;

use anyhow::Result;

use krapy_core::device::{Identity, Store};
use krapy_transport::{adb, browse};

use crate::net;

#[derive(Copy, Clone, Eq, PartialEq)]
enum LinuxDistro {
    Fedora,
    Ubuntu,
    Other,
}

fn linux_distro() -> LinuxDistro {
    let Ok(raw) = std::fs::read_to_string("/etc/os-release") else {
        return LinuxDistro::Other;
    };
    let id = raw.lines().find_map(|l| {
        l.strip_prefix("ID=")
            .map(|v| v.trim_matches('"').to_ascii_lowercase())
    });
    match id.as_deref() {
        Some("fedora") => LinuxDistro::Fedora,
        Some("ubuntu") => LinuxDistro::Ubuntu,
        _ => LinuxDistro::Other,
    }
}

fn linux_firewall_allow(port: u16) -> String {
    match linux_distro() {
        LinuxDistro::Fedora => format!(
            "sudo firewall-cmd --add-port={port}/tcp --add-port={port}/udp (add --permanent and firewall-cmd --reload to keep)"
        ),
        LinuxDistro::Ubuntu => format!("sudo ufw allow {port}/tcp && sudo ufw allow {port}/udp"),
        LinuxDistro::Other => format!(
            "sudo firewall-cmd --add-port={port}/tcp --add-port={port}/udp (Fedora) or sudo ufw allow {port} (Ubuntu)"
        ),
    }
}

pub(crate) fn firewall_hint(port: u16) -> String {
    if cfg!(target_os = "linux") {
        format!(
            "if sessions fail to connect: {}",
            linux_firewall_allow(port)
        )
    } else if cfg!(target_os = "macos") {
        "if sessions fail to connect: allow krapy in System Settings → Network → Firewall"
            .to_string()
    } else if cfg!(target_os = "windows") {
        format!(
            "if sessions fail to connect: netsh advfirewall firewall add rule name=krapy dir=in action=allow protocol=TCP localport={port} && netsh advfirewall firewall add rule name=krapy-udp dir=in action=allow protocol=UDP localport={port}"
        )
    } else {
        format!("if sessions fail to connect: allow UDP/TCP port {port} in the local firewall")
    }
}

pub(crate) fn pair_still_waiting_hint(port: u16) -> String {
    let allow = if cfg!(target_os = "linux") {
        linux_firewall_allow(port)
    } else if cfg!(target_os = "windows") {
        format!(
            "netsh advfirewall firewall add rule name=krapy dir=in action=allow protocol=TCP localport={port} and the same for UDP"
        )
    } else if cfg!(target_os = "macos") {
        "allow krapy in System Settings → Network → Firewall".to_string()
    } else {
        format!("allow TCP/UDP port {port} in the local firewall")
    };
    format!(
        "still waiting. same Wi-Fi? if the phone says it can't reach the PC, allow the port: {allow}"
    )
}

pub fn run(port: u16) -> Result<()> {
    let mut failed = false;

    match net::config_dir_writable() {
        Ok(()) => println!("ok  config dir writable"),
        Err(e) => {
            failed = true;
            println!("fail  config dir writable — {}", e);
        }
    }

    match Store::config_dir().and_then(|dir| Identity::load(&dir)) {
        Ok(id) => println!("ok  identity present ({})", id.id()),
        Err(e) => {
            failed = true;
            println!("fail  identity present — {}", e);
        }
    }

    let port_ok = net::port_free_udp_tcp(port).is_ok();
    match port_ok {
        true => {
            println!("ok  port {} bindable (udp+tcp)", port);
            println!("info  firewall — {}", firewall_hint(port));
        }
        false => {
            failed = true;
            println!(
                "fail  port {} bindable — pick another --port or free the port",
                port
            );
            if let Err(e) = net::port_free_udp_tcp(port) {
                println!("      {}", e);
            }
        }
    }

    let decoders = krapy_decode::available();
    if decoders.is_empty() {
        failed = true;
        println!("fail  video decoders — rebuild with ffmpeg or openh264 feature");
    } else {
        println!("ok  video decoders: {}", decoders.join(", "));
    }

    match browse(Duration::from_millis(200)) {
        Ok(_) => println!("ok  mDNS multicast socket"),
        Err(e) => {
            failed = true;
            println!("fail  mDNS multicast socket — {}", e);
        }
    }

    if adb::on_path() {
        match adb::version() {
            Ok(v) => println!("ok  adb on PATH ({v})"),
            Err(e) => println!("warn  adb broken — reinstall platform-tools; {e}"),
        }
        match adb::devices() {
            Ok(devs) if devs.is_empty() => println!(
                "info  wireless debugging — enable on phone, then: krapy adb pair <host:port> <code>"
            ),
            Ok(_) => {}
            Err(e) => println!("warn  adb devices — {e}"),
        }
    } else {
        failed = true;
        println!("fail  adb on PATH — install Android platform-tools");
    }

    match net::primary_lan_ipv4() {
        Ok(ip) => println!("ok  primary LAN IPv4 {ip}"),
        Err(e) => {
            failed = true;
            println!("fail  primary LAN IPv4 — check network; {e}");
        }
    }

    match krapy_render::probe_gpu() {
        Some(desc) => println!("ok  gpu adapter {desc}"),
        None => println!("warn  gpu adapter — install/update GPU drivers for windowed mirroring"),
    }

    if failed {
        process::exit(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn firewall_hint_includes_port() {
        let h = firewall_hint(4242);
        assert!(h.contains("4242"));
    }
}
