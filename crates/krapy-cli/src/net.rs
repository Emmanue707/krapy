use std::io;
use std::net::{Ipv4Addr, TcpListener, UdpSocket};

use anyhow::{Context, Result};

pub fn primary_lan_ipv4() -> Result<Ipv4Addr> {
    let socket = UdpSocket::bind("0.0.0.0:0").context("bind udp")?;
    socket
        .connect("1.1.1.1:80")
        .context("udp connect for route")?;
    let local = socket.local_addr().context("local addr")?;
    match local.ip() {
        std::net::IpAddr::V4(v4) => Ok(v4),
        std::net::IpAddr::V6(_) => anyhow::bail!("no ipv4 route"),
    }
}

pub fn port_free_udp_tcp(port: u16) -> Result<()> {
    let udp = UdpSocket::bind(("0.0.0.0", port)).context("bind udp port")?;
    drop(udp);
    let tcp = TcpListener::bind(("0.0.0.0", port)).context("bind tcp port")?;
    drop(tcp);
    Ok(())
}

pub fn config_dir_writable() -> Result<()> {
    let dir = krapy_core::device::Store::config_dir()?;
    std::fs::create_dir_all(&dir).context("create config dir")?;
    let probe = dir.join(".write_probe");
    std::fs::write(&probe, b"x").context("write probe file")?;
    std::fs::remove_file(probe).or_else(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            Ok(())
        } else {
            Err(e)
        }
    })
    .context("remove probe file")?;
    Ok(())
}