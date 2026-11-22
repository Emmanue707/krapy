use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use qrcode::QrCode;
use qrcode::render::unicode;
use rand::RngCore;

use krapy_core::Preset;
use krapy_core::device::{Identity, Store};
use krapy_transport::adb;

use crate::net;

pub fn run(port: u16, name: &str, _preset: Preset, adb_serial: Option<String>) -> Result<()> {
    let dir = Store::config_dir()?;
    let identity = Identity::load(&dir)?;
    let mut key = [0u8; 32];
    rand::rng().fill_bytes(&mut key);
    let key_b64 = URL_SAFE_NO_PAD.encode(key);
    let ip = net::primary_lan_ipv4().context("primary LAN IPv4")?;
    let id = identity.id();
    let n = urlencoding::encode(name);
    let uri = format!(
        "krapy://pair?v=1&h={ip}&p={port}&k={key_b64}&id={id}&n={n}",
        ip = ip,
        port = port,
        key_b64 = key_b64,
        id = id,
        n = n
    );
    println!("{uri}");
    if let Some(adb_serial) = adb_serial {
        let pref = if adb_serial.is_empty() {
            None
        } else {
            Some(adb_serial.as_str())
        };
        let s = adb::pick_serial(pref)?;
        adb::launch_app(&s, &uri).context("launch pair uri on phone")?;
        println!("ok  opened pair uri on {s}");
        return Ok(());
    }
    let code = QrCode::new(uri.as_bytes()).context("qr encode")?;
    let image = code
        .render::<unicode::Dense1x2>()
        .dark_color(unicode::Dense1x2::Dark)
        .light_color(unicode::Dense1x2::Light)
        .build();
    println!("{image}");
    println!("pair listener not wired yet — scan the QR when it is");
    Ok(())
}