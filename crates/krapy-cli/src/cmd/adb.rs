use anyhow::Result;

use krapy_transport::adb;

pub fn pair(host_port: &str, code: &str) -> Result<()> {
    adb::pair(host_port, code)?;
    println!("ok  paired {host_port}");
    Ok(())
}

pub fn connect(host_port: &str) -> Result<()> {
    adb::connect(host_port)?;
    println!("ok  connected {host_port}");
    Ok(())
}
