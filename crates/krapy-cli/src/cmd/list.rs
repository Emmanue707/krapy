use anyhow::Result;

use krapy_core::device::Store;
use krapy_transport::adb;

pub fn run() -> Result<()> {
    let store = Store::open_default()?;
    let devices = store.devices();
    let mut any = false;
    if !devices.is_empty() {
        any = true;
        println!("saved devices:");
        for d in devices {
            let alias = d.alias.as_deref().unwrap_or("-");
            let addr = d.last_addr.as_deref().unwrap_or("-");
            let seen = d
                .last_seen
                .map(|s| s.to_string())
                .unwrap_or_else(|| "-".into());
            println!(
                "  {}  alias={}  name={}  last_addr={}  last_seen={}",
                d.id, alias, d.name, addr, seen
            );
        }
    }
    if adb::on_path() {
        if let Ok(devs) = adb::devices()
            && !devs.is_empty()
        {
            any = true;
            println!("adb devices:");
            for d in devs {
                let model = d.model.as_deref().unwrap_or("-");
                println!("  {}  {}  model={}", d.serial, d.state, model);
            }
        }
        if let Ok(svc) = adb::mdns_services()
            && !svc.is_empty()
        {
            any = true;
            println!("adb mdns (wireless debugging):");
            for s in svc {
                let ty = if s.service_type.is_empty() {
                    "-".to_string()
                } else {
                    s.service_type.clone()
                };
                println!("  {}  {}  {}", s.name, ty, s.address);
            }
        }
    } else if !any {
        println!("hint  install Android platform-tools and put adb on PATH");
    } else {
        println!("hint  adb not on PATH — wireless adb sections hidden");
    }
    if !any {
        println!("no saved devices");
    }
    Ok(())
}
