use anyhow::{Result, bail};

use krapy_core::device::Store;

pub fn run(device: &str, name: &str) -> Result<()> {
    let mut store = Store::open_default()?;
    if !store.set_alias(device, Some(name.to_string()))? {
        bail!("device not found: {}", device);
    }
    Ok(())
}