use crate::gpu::Gpu;
use crate::keys::{self, KEY_BACK, KEY_HOME, UiAction};
use crate::letterbox::{display_scale, effective_video, letterbox_viewport, window_to_video};
use crate::mailbox::{FrameSource, Wake};
use crate::overlay::{
    OverlayQuad, PhoneSettings, build_menu_quads, build_paused_tag, hit_test, panel_layout,
    point_in_panel, row_action,
};
use krapy_decode::Frame;
use krapy_transport::Control;
use krapy_transport::wire::SET_KEY_ROTATE;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
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
    rotation: u8,
    settings: PhoneSettings,
    menu_open: bool,
    overlay_quads: Vec<OverlayQuad>,
    synced_rotation: bool,
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
            rotation: 0,
            settings: PhoneSettings::default(),
            menu_open: false,
            overlay_quads: Vec::new(),
            synced_rotation: false,
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

    fn size_window_to_frame(&mut self, width: u32, height: u32) {
        if self.sized {
            return;
        }
        let Some(w) = &self.window else {
            return;
        };
        let monitor = w.current_monitor();
        let target_h = monitor
            .map(|m| m.size().height as f64 * 0.85)
            .unwrap_or(720.0);
        let aspect = width as f64 / height.max(1) as f64;
        let w_log = target_h * aspect;
        let _ = w.request_inner_size(LogicalSize::new(w_log, target_h));
        self.sized = true;
    }

    fn refresh_title(&self) {
        let Some(w) = &self.window else {
            return;
        };
        let base = format!("krapy - {}", self.device_name);
        let title = if self.settings.paused {
            format!("{base} - paused")
        } else {
            base
        };
        w.set_title(&title);
    }

    fn pull_settings(&mut self) {
        let s = self.source.phone_settings();
        if s.rotation != self.rotation && self.synced_rotation {
            self.rotation = s.rotation;
        }
        if !self.synced_rotation && s.rotation <= 3 {
            self.rotation = s.rotation;
            self.synced_rotation = true;
        }
        self.settings = s;
        self.refresh_title();
    }

    fn rebuild_overlay(&mut self) {
        let Some((ww, wh, _, _)) = self.win_metrics() else {
            return;
        };
        if self.menu_open {
            let stats = self.source.menu_stats();
            build_menu_quads(&mut self.overlay_quads, &self.settings, &stats, ww, wh);
        } else if self.settings.paused {
            build_paused_tag(&mut self.overlay_quads, ww, wh);
        } else {
            self.overlay_quads.clear();
        }
        if let Some(gpu) = self.gpu.as_mut() {
            if self.overlay_quads.is_empty() {
                gpu.clear_overlay();
            } else {
                gpu.set_overlay_quads(&self.overlay_quads, ww, wh);
            }
        }
    }

    fn redraw(&mut self) {
        if let Some(frame) = self.source.take_frame() {
            self.last_frame = Some(frame);
        }
        let Some((fw, fh)) = self.last_frame.as_ref().map(|f| (f.width, f.height)) else {
            return;
        };
        if self.video != Some((fw, fh)) {
            self.video = Some((fw, fh));
            self.size_window_to_frame(fw, fh);
        }
        let metrics = self.win_metrics();
        let rot = self.rotation;
        if let Some(gpu) = self.gpu.as_mut()
            && let Some(frame) = self.last_frame.as_ref()
        {
            gpu.upload(frame);
            if let Some((ww, wh, vw, vh)) = metrics {
                let (evw, evh) = effective_video(vw, vh, rot);
                let vp = letterbox_viewport(ww, wh, evw, evh);
                let scale = display_scale(vp.w, vp.h, vw as f32, vh as f32);
                let _ = gpu.draw(vp, vw, vh, rot, scale);
            }
        }
    }

    fn pointer_at(&self, x: f32, y: f32) -> Option<(u16, u16)> {
        let (ww, wh, vw, vh) = self.win_metrics()?;
        window_to_video(x, y, ww, wh, vw, vh, self.rotation)
    }

    fn touch(&self, action: u8, x: u16, y: u16) {
        self.send(Control::Touch {
            action,
            pointer: 0,
            x,
            y,
        });
    }

    fn local_rotate(&mut self) {
        self.rotation = self.rotation.wrapping_add(1) & 3;
        if let Some(w) = &self.window {
            w.request_redraw();
        }
        self.rebuild_overlay();
    }

    fn apply_setting(&mut self, mut ctrl: Control) {
        ctrl = keys::resolve_toggle_setting(ctrl, &self.settings);
        if let Control::SetSetting { key, .. } = ctrl
            && key == SET_KEY_ROTATE
        {
            self.local_rotate();
        }
        self.send(ctrl);
    }

    fn toggle_menu(&mut self) {
        self.menu_open = !self.menu_open;
        self.rebuild_overlay();
        if let Some(w) = &self.window {
            w.request_redraw();
        }
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
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let gpu = Gpu::new(window.clone()).expect("gpu");
        self.window = Some(window);
        self.gpu = Some(gpu);
        self.pull_settings();
        self.rebuild_overlay();
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
            Wake::Rotate => self.local_rotate(),
            Wake::Settings => {
                self.pull_settings();
                self.rebuild_overlay();
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }
            Wake::Pause(on) => {
                if let Some(w) = self.window.as_ref() {
                    let base = format!("krapy - {}", self.device_name);
                    let title = if on { format!("{base} (paused)") } else { base };
                    w.set_title(&title);
                }
            }
            Wake::Stats => {
                if self.menu_open {
                    self.rebuild_overlay();
                    if let Some(w) = self.window.as_ref() {
                        w.request_redraw();
                    }
                }
            }
        }
        let _ = event_loop;
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.resize(size.width, size.height);
                }
                self.rebuild_overlay();
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
    fn on_key(&mut self, event: KeyEvent) {
        if event.state != ElementState::Pressed || event.repeat {
            return;
        }
        if self.menu_open && matches!(event.logical_key, Key::Named(NamedKey::Escape)) {
            self.menu_open = false;
            self.rebuild_overlay();
            return;
        }
        if let Some(act) = keys::pc_shortcut(&event, self.modifiers) {
            match act {
                UiAction::ToggleMenu => self.toggle_menu(),
                UiAction::LocalRotate => self.apply_setting(Control::SetSetting {
                    key: SET_KEY_ROTATE,
                    value: 0,
                }),
                UiAction::Setting(c) => self.apply_setting(c),
            }
            return;
        }
        if keys::consumes_pc_shortcut(&event, self.modifiers) {
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
        if self.menu_open
            && button == MouseButton::Left
            && state == ElementState::Pressed
            && let Some((px, py)) = self.cursor
        {
            let (ww, wh, _, _) = self.win_metrics().unwrap_or((800.0, 600.0, 0, 0));
            let layout = panel_layout(ww, wh);
            if point_in_panel(px, py, &layout) {
                if let Some(row) = hit_test(px, py, &layout)
                    && let Some(ctrl) = row_action(row, &self.settings)
                {
                    self.apply_setting(ctrl);
                }
                return;
            }
        }
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
