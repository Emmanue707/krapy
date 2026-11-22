use anyhow::Result;

use krapy_core::Preset;

pub fn run(device: &str, preset: Preset, port: u16) -> Result<()> {
    println!(
        "connect to {} on port {} with preset {} (stream not wired yet)",
        device,
        port,
        preset
    );
    Ok(())
}