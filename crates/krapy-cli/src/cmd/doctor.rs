use std::process;

use anyhow::Result;

use krapy_core::device::{Identity, Store};

use crate::net;

pub fn run(port: u16) -> Result<()> {
    let mut failed = false;

    match net::config_dir_writable() {
        Ok(()) => println!("ok  config dir writable"),
        Err(e) => {
            failed = true;
            println!("fail  config dir writable — {}", e);
        }
    }

    match Store::config_dir().and_then(|dir| Identity::load(&dir)) {
        Ok(id) => println!("ok  identity present ({})", id.id()),
        Err(e) => {
            failed = true;
            println!("fail  identity present — {}", e);
        }
    }

    match net::port_free_udp_tcp(port) {
        Ok(()) => println!("ok  port {} bindable (udp+tcp)", port),
        Err(e) => {
            failed = true;
            println!("fail  port {} bindable — pick another --port or free the port", port);
            println!("      {}", e);
        }
    }

    if net::which("adb") {
        match net::adb_version() {
            Ok(()) => println!("ok  adb on PATH"),
            Err(e) => println!("warn  adb broken — reinstall platform-tools; {}", e),
        }
    } else {
        failed = true;
        println!("fail  adb on PATH — install Android platform-tools");
    }

    match net::primary_lan_ipv4() {
        Ok(ip) => println!("ok  primary LAN IPv4 {}", ip),
        Err(e) => {
            failed = true;
            println!("fail  primary LAN IPv4 — check network; {}", e);
        }
    }

    if failed {
        process::exit(1);
    }
    Ok(())
}