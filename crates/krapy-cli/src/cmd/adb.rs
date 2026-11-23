use anyhow::Result;

use krapy_transport::adb;

fn pair_host_ip(host_port: &str) -> &str {
    if let Some(rest) = host_port.strip_prefix('[') {
        rest.split(']').next().unwrap_or(host_port)
    } else {
        host_port
            .rsplit_once(':')
            .map(|(h, _)| h)
            .unwrap_or(host_port)
    }
}

pub fn pair(host_port: &str, code: &str) -> Result<()> {
    adb::pair(host_port, code)?;
    println!("ok  paired {host_port}");
    let ip = pair_host_ip(host_port);
    let services = adb::mdns_services()?;
    let connect_addr = services.iter().find_map(|s| {
        if !s.service_type.contains("_adb-tls-connect._tcp") {
            return None;
        }
        let host = s
            .address
            .rsplit_once(':')
            .map(|(h, _)| h)
            .unwrap_or(&s.address);
        if host == ip {
            Some(s.address.clone())
        } else {
            None
        }
    });
    if let Some(addr) = connect_addr {
        adb::connect(&addr)?;
        println!("ok  connected {addr}");
        println!("next: krapy connect --usb");
    } else {
        println!(
            "next: krapy adb connect <ip:port>  (wireless debugging port on the phone, not the pairing port)"
        );
    }
    Ok(())
}

pub fn connect(host_port: &str) -> Result<()> {
    adb::connect(host_port)?;
    println!("ok  connected {host_port}");
    Ok(())
}

pub fn usb(
    device: Option<String>,
    serial: Option<String>,
    port: u16,
    opts: crate::mirror::MirrorOpts,
) -> Result<()> {
    super::connect::run_usb(device, serial, port, opts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_host_ipv4() {
        assert_eq!(pair_host_ip("192.168.0.12:37123"), "192.168.0.12");
    }
}
