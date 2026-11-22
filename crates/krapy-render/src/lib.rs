mod app;
mod gpu;
mod keys;
mod letterbox;
mod mailbox;

use krapy_transport::Control;
use std::sync::mpsc::Sender;

pub use mailbox::FrameSource;

pub fn run(source: FrameSource, device_name: &str, control: Sender<Control>) -> anyhow::Result<()> {
    app::run_event_loop(source, device_name, control)
}