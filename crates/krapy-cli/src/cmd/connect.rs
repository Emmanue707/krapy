use anyhow::{Context, Result, bail};

use krapy_core::device::{Identity, PairedDevice, Store};
use krapy_transport::{Advertiser, Listener};

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
