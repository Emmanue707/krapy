mod app;
mod font;
mod gpu;
mod keys;
mod letterbox;
mod mailbox;
mod overlay;
mod pair_qr;

pub use overlay::{MenuStats, PhoneSettings};

use krapy_transport::Control;
use std::sync::mpsc::Sender;

pub use gpu::probe_gpu;
pub use mailbox::{FrameSource, Wake};
pub use pair_qr::pair_qr_frame;

pub fn run(source: FrameSource, device_name: &str, control: Sender<Control>) -> anyhow::Result<()> {
    app::run_event_loop(source, device_name, control)
}
