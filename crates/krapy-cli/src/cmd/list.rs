use anyhow::Result;

use krapy_core::device::Store;

pub fn run() -> Result<()> {
    let store = Store::open_default()?;
    let devices = store.devices();
    if devices.is_empty() {
        println!("no saved devices");
        return Ok(());
    }
    for d in devices {
        let alias = d.alias.as_deref().unwrap_or("-");
        let addr = d.last_addr.as_deref().unwrap_or("-");
        let seen = d
            .last_seen
            .map(|s| s.to_string())
            .unwrap_or_else(|| "-".into());
        println!(
            "{}  alias={}  name={}  last_addr={}  last_seen={}",
            d.id, alias, d.name, addr, seen
        );
    }
    Ok(())
}