use anyhow::{Result, bail};

use krapy_core::device::Store;

pub fn run(device: &str) -> Result<()> {
    let mut store = Store::open_default()?;
    if !store.remove(device)? {
        bail!("device not found: {}", device);
    }
    Ok(())
}