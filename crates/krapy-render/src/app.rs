use crate::gpu::Gpu;
use crate::keys::{self, KEY_BACK, KEY_HOME};
use crate::letterbox::{letterbox_viewport, window_to_video};
use crate::mailbox::{FrameSource, Wake};
use krapy_decode::Frame;
use krapy_transport::Control;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, ModifiersState};
use winit::window::{Window, WindowId};

pub struct KrapyApp {
    source: FrameSource,
    device_name: String,
    control: Sender<Control>,
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    video: Option<(u32, u32)>,
    pointer_down: bool,
    modifiers: ModifiersState,
    sized: bool,
    last_frame: Option<Frame>,
    cursor: Option<(f32, f32)>,
}

impl KrapyApp {
    fn new(source: FrameSource, device_name: String, control: Sender<Control>) -> Self {
        Self {
            source,
            device_name,
            control,
            window: None,
            gpu: None,
            video: None,
            pointer_down: false,
            modifiers: ModifiersState::empty(),
            sized: false,
            last_frame: None,
            cursor: None,
        }
    }

    fn send(&self, c: Control) {
        let _ = self.control.send(c);
    }

    fn win_metrics(&self) -> Option<(f32, f32, u32, u32)> {
        let w = self.window.as_ref()?;
        let s = w.inner_size();
        let (vw, vh) = self.video?;
        Some((s.width as f32, s.height as f32, vw, vh))
    }

    fn apply_phone_clipboard(&self) {
        if let Some(text) = self.source.take_phone_clipboard()
            && let Ok(mut cb) = arboard::Clipboard::new()
        {
            let _ = cb.set_text(text);
        }
    }

    fn redraw(&mut self) {
        if let Some(frame) = self.source.take_frame() {
            self.last_frame = Some(frame);
        }
        let Some(frame) = self.last_frame.as_ref() else {
            return;
        };
        if self.video != Some((frame.width, frame.height)) {
            self.video = Some((frame.width, frame.height));
            if let Some(w) = &self.window
                && !self.sized
            {
                let base = 480.0f64;
                let w_log = base;
                let h_log = base * frame.height as f64 / frame.width.max(1) as f64;
                let _ = w.request_inner_size(LogicalSize::new(w_log, h_log));
                self.sized = true;
            }
        }
        let metrics = self.win_metrics();
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.upload(frame);
            if let Some((ww, wh, vw, vh)) = metrics {
                let vp = letterbox_viewport(ww, wh, vw as f32, vh as f32);
                let _ = gpu.draw(vp);
            }
        }
    }

    fn pointer_at(&self, x: f32, y: f32) -> Option<(u16, u16)> {
        let (ww, wh, vw, vh) = self.win_metrics()?;
        window_to_video(x, y, ww, wh, vw, vh)
    }

    fn touch(&self, action: u8, x: u16, y: u16) {
        self.send(Control::Touch {
            action,
            pointer: 0,
            x,
            y,
        });
    }
}

impl ApplicationHandler<Wake> for KrapyApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title(format!("krapy - {}", self.device_name))
            .with_inner_size(LogicalSize::new(480.0, 854.0))
            .with_resizable(true);
        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .expect("create window"),
        );
        let gpu = Gpu::new(window.clone()).expect("gpu");
        self.window = Some(window);
        self.gpu = Some(gpu);
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: Wake) {
        match event {
            Wake::NewFrame => {
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }
            Wake::PhoneClipboard => self.apply_phone_clipboard(),
        }
        let _ = event_loop;
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.resize(size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::ModifiersChanged(m) => {
                self.modifiers = m.state();
            }
            WindowEvent::KeyboardInput { event, .. } => self.on_key(event),
            WindowEvent::MouseInput { state, button, .. } => self.on_mouse_button(state, button),
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = Some((position.x as f32, position.y as f32));
                if self.pointer_down
                    && let Some((x, y)) = self.pointer_at(position.x as f32, position.y as f32)
                {
                    self.touch(1, x, y);
                }
            }
            _ => {}
        }
    }
}

impl KrapyApp {
    fn on_key(&self, event: KeyEvent) {
        if event.state != ElementState::Pressed || event.repeat {
            return;
        }
        if self.modifiers.control_key()
            && matches!(event.logical_key, Key::Character(ref s) if s.eq_ignore_ascii_case("v"))
            && let Ok(mut cb) = arboard::Clipboard::new()
            && let Ok(text) = cb.get_text()
            && !text.is_empty()
        {
            self.send(Control::Clipboard { text });
            return;
        }
        if let Some(code) = keys::named_key_action(&event.logical_key) {
            self.send(Control::Key { code });
            return;
        }
        if let Some(t) = event.text
            && !t.is_empty()
            && !t.chars().all(|c| c.is_control())
        {
            self.send(Control::Text {
                text: t.to_string(),
            });
        }
    }

    fn on_mouse_button(&mut self, state: ElementState, button: MouseButton) {
        match button {
            MouseButton::Left => {
                self.pointer_down = state == ElementState::Pressed;
                let action = match state {
                    ElementState::Pressed => 0u8,
                    ElementState::Released => 2u8,
                };
                if let Some((px, py)) = self.cursor
                    && let Some((x, y)) = self.pointer_at(px, py)
                {
                    self.touch(action, x, y);
                }
            }
            MouseButton::Right if state == ElementState::Pressed => {
                self.send(Control::Key { code: KEY_BACK });
            }
            MouseButton::Middle if state == ElementState::Pressed => {
                self.send(Control::Key { code: KEY_HOME });
            }
            _ => {}
        }
    }
}

pub fn run_event_loop(
    source: FrameSource,
    device_name: &str,
    control: Sender<Control>,
) -> anyhow::Result<()> {
    let event_loop = EventLoop::<Wake>::with_user_event().build()?;
    let proxy = event_loop.create_proxy();
    source.attach_proxy(proxy);
    let mut app = KrapyApp::new(source, device_name.to_string(), control);
    event_loop.run_app(&mut app)?;
    Ok(())
}