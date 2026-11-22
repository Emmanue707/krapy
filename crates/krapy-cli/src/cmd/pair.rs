use std::io::IsTerminal;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use qrcode::QrCode;
use qrcode::render::unicode;
use rand::RngCore;

use krapy_core::device::{Identity, Store};
use krapy_transport::adb;
use krapy_transport::{Advertiser, Listener};

use crate::mirror::{self, MirrorOpts};
use crate::net;

use super::doctor;

const QR_LINE_START: &str = "\x1b[30;47m";
const QR_LINE_END: &str = "\x1b[0m";

fn terminal_qr_ansi() -> bool {
    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

fn terminal_qr_lines(image: &str, ansi: bool) -> Vec<String> {
    image
        .lines()
        .map(|line| {
            if ansi {
                format!("{QR_LINE_START}{line}{QR_LINE_END}")
            } else {
                line.to_string()
            }
        })
        .collect()
}

pub fn run(port: u16, name: &str, opts: MirrorOpts, adb_serial: Option<String>) -> Result<()> {
    let dir = Store::config_dir()?;
    let identity = Identity::load(&dir)?;
    let desktop_id = mirror::desktop_id_bytes(&identity.id())?;
    let listener = Listener::bind(port, desktop_id).context("bind listener")?;
    let _adv = Advertiser::advertise(&identity.id(), port).context("mdns advertise")?;

    let mut key = [0u8; 32];
    rand::rng().fill_bytes(&mut key);
    let key_b64 = URL_SAFE_NO_PAD.encode(key);
    let ip = net::primary_lan_ipv4().context("primary LAN IPv4")?;
    let id = identity.id();
    let n = urlencoding::encode(name);
    let uri = format!("krapy://pair?v=1&h={ip}&p={port}&k={key_b64}&id={id}&n={n}");
    println!("uri: {uri}");
    if let Some(adb_serial) = adb_serial {
        let pref = if adb_serial.is_empty() {
            None
        } else {
            Some(adb_serial.as_str())
        };
        let s = adb::pick_serial(pref)?;
        adb::launch_app(&s, &uri).context("launch pair uri on phone")?;
        println!("ok  opened pair uri on {s}");
    } else {
        let code = QrCode::new(uri.as_bytes()).context("qr encode")?;
        let image = code
            .render::<unicode::Dense1x2>()
            .quiet_zone(true)
            .dark_color(unicode::Dense1x2::Dark)
            .light_color(unicode::Dense1x2::Light)
            .build();
        let ansi = terminal_qr_ansi();
        for line in terminal_qr_lines(&image, ansi) {
            println!("{line}");
        }
    }

    println!("waiting for the phone on {ip}:{port} (tcp+udp). keep this running.");

    let connected = Arc::new(AtomicBool::new(false));
    let connected_timer = Arc::clone(&connected);
    let port_timer = port;
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(20));
        if !connected_timer.load(Ordering::Relaxed) {
            println!("{}", doctor::pair_still_waiting_hint(port_timer));
        }
    });

    let store = Store::open_default()?;
    if opts.no_window {
        mirror::headless_pair(listener, store, key, opts, connected)
    } else {
        mirror::window_pair(listener, store, key, name, &uri, opts, connected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_lines_ansi_wrap() {
        let lines = terminal_qr_lines("ab\ncd", true);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], format!("{QR_LINE_START}ab{QR_LINE_END}"));
        assert_eq!(lines[1], format!("{QR_LINE_START}cd{QR_LINE_END}"));
    }

    #[test]
    fn qr_lines_plain_without_ansi() {
        let lines = terminal_qr_lines("x\ny", false);
        assert_eq!(lines, vec!["x".to_string(), "y".to_string()]);
    }
}
