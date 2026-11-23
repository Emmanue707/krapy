use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;

use krapy_core::device::{Identity, PairedDevice, Store};
use krapy_transport::adb;
use krapy_transport::{Advertiser, Listener};

use super::doctor;
use crate::mirror::{self, MirrorOpts};

fn resolve_device<'a>(store: &'a Store, device: Option<&str>) -> Result<&'a PairedDevice> {
    match device {
        Some(d) => store.find(d).with_context(|| format!("unknown device {d}")),
        None => match store.devices().len() {
            0 => bail!("no paired devices — run krapy pair first"),
            1 => Ok(&store.devices()[0]),
            n => bail!("{n} paired devices — pass device id or alias"),
        },
    }
}

struct ReverseGuard {
    serial: String,
    port: u16,
}

impl Drop for ReverseGuard {
    fn drop(&mut self) {
        let _ = adb::reverse_remove(&self.serial, self.port);
    }
}

fn default_desktop_name() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "krapy-desktop".into())
}

fn usb_pair_with_key(
    listener: Listener,
    store: Store,
    port: u16,
    name: &str,
    desktop_id: &str,
    key: [u8; 32],
    opts: MirrorOpts,
) -> Result<()> {
    let key_b64 = URL_SAFE_NO_PAD.encode(key);
    let uri = format!(
        "krapy://pair?v=1&h=127.0.0.1&p={port}&k={key_b64}&id={desktop_id}&n={}",
        urlencoding::encode(name)
    );
    println!("uri: {uri}");
    println!("waiting for the phone on 127.0.0.1:{port} (tcp+udp). keep this running.");

    let connected = Arc::new(AtomicBool::new(false));
    let connected_timer = Arc::clone(&connected);
    let port_timer = port;
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(20));
        if !connected_timer.load(Ordering::Relaxed) {
            println!("{}", doctor::pair_still_waiting_hint(port_timer));
        }
    });

    if opts.no_window {
        mirror::headless_pair(listener, store, key, opts, connected)
    } else {
        mirror::window_pair(listener, store, key, name, &uri, opts, connected)
    }
}

pub fn run_usb(
    device: Option<String>,
    serial: Option<String>,
    port: u16,
    opts: MirrorOpts,
) -> Result<()> {
    adb::require_on_path()?;
    let serial = adb::wait_usb_device(serial.as_deref())?;
    adb::reverse(&serial, port, port).context("adb reverse")?;
    let _reverse = ReverseGuard {
        serial: serial.clone(),
        port,
    };

    let dir = Store::config_dir()?;
    let identity = Identity::load(&dir)?;
    let desktop_id = mirror::desktop_id_bytes(&identity.id())?;
    let listener = Listener::bind(port, desktop_id).context("bind listener")?;
    let _adv = Advertiser::advertise(&identity.id(), port).context("mdns advertise")?;

    let store = Store::open_default()?;
    if store.devices().is_empty() {
        let name = default_desktop_name();
        let mut key = [0u8; 32];
        rand::rng().fill_bytes(&mut key);
        let key_b64 = URL_SAFE_NO_PAD.encode(key);
        let id = identity.id();
        let n = urlencoding::encode(&name);
        let uri = format!("krapy://pair?v=1&h=127.0.0.1&p={port}&k={key_b64}&id={id}&n={n}");
        adb::launch_app(&serial, &uri).context("launch pair uri on phone")?;
        println!("tap \"Start now\" on the phone");
        return usb_pair_with_key(listener, store, port, &name, &id, key, opts);
    }

    let id = identity.id();
    let uri = format!("krapy://connect?id={id}&h=127.0.0.1&p={port}");
    adb::launch_app(&serial, &uri).context("launch connect uri on phone")?;
    println!("tap \"Start now\" on the phone");

    let dev = resolve_device(&store, device.as_deref())?;
    let title = dev
        .alias
        .as_deref()
        .unwrap_or(dev.name.as_str())
        .to_string();

    if opts.no_window {
        mirror::headless_connect(listener, store, opts)
    } else {
        mirror::window_connect(listener, store, &title, opts)
    }
}

pub fn run(device: Option<String>, port: u16, opts: MirrorOpts) -> Result<()> {
    let store = Store::open_default()?;
    let dev = resolve_device(&store, device.as_deref())?;
    let title = dev
        .alias
        .as_deref()
        .unwrap_or(dev.name.as_str())
        .to_string();

    let dir = Store::config_dir()?;
    let identity = Identity::load(&dir)?;
    let desktop_id = mirror::desktop_id_bytes(&identity.id())?;
    let listener = Listener::bind(port, desktop_id).context("bind listener")?;
    let _adv = Advertiser::advertise(&identity.id(), port).context("mdns advertise")?;

    if opts.no_window {
        mirror::headless_connect(listener, store, opts)
    } else {
        mirror::window_connect(listener, store, &title, opts)
    }
}
