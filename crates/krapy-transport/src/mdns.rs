use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;

const SERVICE_TYPE: &str = "_krapy._tcp.local.";

pub struct Advertiser {
    _daemon: ServiceDaemon,
}

impl Advertiser {
    pub fn advertise(desktop_id: &str, port: u16) -> Result<Self, mdns_sd::Error> {
        let daemon = ServiceDaemon::new()?;
        let host = format!("{}.local.", desktop_id);
        let mut props = HashMap::new();
        props.insert("id".into(), desktop_id.into());
        props.insert("v".into(), "1".into());
        let info = ServiceInfo::new(
            SERVICE_TYPE,
            desktop_id,
            &host,
            IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
            port,
            Some(props),
        )?;
        daemon.register(info)?;
        Ok(Self { _daemon: daemon })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered {
    pub id: String,
    pub host: String,
    pub port: u16,
}

pub fn browse(timeout: Duration) -> Result<Vec<Discovered>, mdns_sd::Error> {
    let daemon = ServiceDaemon::new()?;
    let receiver = daemon.browse(SERVICE_TYPE)?;
    let mut found = Vec::new();
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match receiver.recv_timeout(remaining.min(Duration::from_millis(200))) {
            Ok(ServiceEvent::ServiceResolved(info)) => {
                let id = info.get_property_val_str("id").unwrap_or("").to_string();
                let port = info.get_port();
                let host = info.get_hostname().to_string();
                if !id.is_empty() {
                    let entry = Discovered { id, host, port };
                    if !found
                        .iter()
                        .any(|e: &Discovered| e.id == entry.id && e.port == entry.port)
                    {
                        found.push(entry);
                    }
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    Ok(found)
}
