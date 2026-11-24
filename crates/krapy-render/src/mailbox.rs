use krapy_decode::Frame;
use std::sync::{Arc, Mutex};
use winit::event_loop::EventLoopProxy;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Wake {
    NewFrame,
    PhoneClipboard,
    Rotate,
    Pause(bool),
    Settings,
    Stats,
}

struct Inner {
    frame: Mutex<Option<Frame>>,
    phone_clipboard: Mutex<Option<String>>,
    proxy: Mutex<Option<EventLoopProxy<Wake>>>,
    settings: Mutex<crate::overlay::PhoneSettings>,
    stats: Mutex<crate::overlay::MenuStats>,
}

#[derive(Clone)]
pub struct FrameSource {
    inner: Arc<Inner>,
}

impl Default for FrameSource {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameSource {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                frame: Mutex::new(None),
                phone_clipboard: Mutex::new(None),
                proxy: Mutex::new(None),
                settings: Mutex::new(crate::overlay::PhoneSettings::default()),
                stats: Mutex::new(crate::overlay::MenuStats::default()),
            }),
        }
    }

    pub fn set_phone_settings(&self, s: crate::overlay::PhoneSettings) {
        if let Ok(mut g) = self.inner.settings.lock() {
            *g = s;
        }
        self.poke(Wake::Settings);
    }

    pub fn phone_settings(&self) -> crate::overlay::PhoneSettings {
        self.inner.settings.lock().map(|g| *g).unwrap_or_default()
    }

    pub fn set_menu_stats(&self, s: crate::overlay::MenuStats) {
        if let Ok(mut g) = self.inner.stats.lock() {
            *g = s;
        }
        self.poke(Wake::Stats);
    }

    pub fn menu_stats(&self) -> crate::overlay::MenuStats {
        self.inner.stats.lock().map(|g| *g).unwrap_or_default()
    }

    fn poke(&self, wake: Wake) {
        if let Ok(guard) = self.inner.proxy.lock()
            && let Some(p) = guard.as_ref()
        {
            let _ = p.send_event(wake);
        }
    }

    pub fn publish(&self, frame: Frame) {
        if let Ok(mut g) = self.inner.frame.lock() {
            *g = Some(frame);
        }
        if let Ok(guard) = self.inner.proxy.lock()
            && let Some(p) = guard.as_ref()
        {
            let _ = p.send_event(Wake::NewFrame);
        }
    }

    pub fn set_paused(&self, on: bool) {
        if let Ok(guard) = self.inner.proxy.lock()
            && let Some(p) = guard.as_ref()
        {
            let _ = p.send_event(Wake::Pause(on));
        }
    }

    pub fn request_rotate(&self) {
        if let Ok(guard) = self.inner.proxy.lock()
            && let Some(p) = guard.as_ref()
        {
            let _ = p.send_event(Wake::Rotate);
        }
    }

    pub fn phone_clipboard(&self, text: String) {
        if let Ok(mut g) = self.inner.phone_clipboard.lock() {
            *g = Some(text);
        }
        if let Ok(guard) = self.inner.proxy.lock()
            && let Some(p) = guard.as_ref()
        {
            let _ = p.send_event(Wake::PhoneClipboard);
        }
    }

    pub(crate) fn attach_proxy(&self, proxy: EventLoopProxy<Wake>) {
        if let Ok(mut g) = self.inner.proxy.lock() {
            *g = Some(proxy);
        }
    }

    pub(crate) fn take_frame(&self) -> Option<Frame> {
        self.inner.frame.lock().ok().and_then(|mut g| g.take())
    }

    pub(crate) fn take_phone_clipboard(&self) -> Option<String> {
        self.inner
            .phone_clipboard
            .lock()
            .ok()
            .and_then(|mut g| g.take())
    }
}
