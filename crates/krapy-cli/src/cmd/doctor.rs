use std::process;
use std::time::Duration;

use anyhow::Result;

use krapy_core::device::{Identity, Store};
use krapy_transport::{adb, browse};

use crate::net;

fn firewall_hint() -> &'static str {
    if cfg!(target_os = "linux") {
        "if sessions fail to connect: sudo ufw allow 27183/udp && sudo ufw allow 27183/tcp (or firewall-cmd --add-port=27183/udp --add-port=27183/tcp)"
    } else if cfg!(target_os = "macos") {
        "if sessions fail to connect: allow krapy in System Settings → Network → Firewall"
    } else if cfg!(target_os = "windows") {
        "if sessions fail to connect: netsh advfirewall firewall add rule name=krapy dir=in action=allow protocol=UDP localport=27183"
    } else {
        "if sessions fail to connect: allow UDP/TCP on your krapy port in the local firewall"
    }
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
            println!("info  firewall — {}", firewall_hint());
        }
        false => {
            failed = true;
            println!("fail  port {} bindable — pick another --port or free the port", port);
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