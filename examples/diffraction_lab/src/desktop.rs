//! 事件循环拥有窗口和显示设备。工作线程独占制核设备，事件代理唤醒空闲窗口。
use crate::{
    controls::{Control, Field, Model},
    panel,
    preview::THUMB,
    recipe::{Recipe, Result},
    surface::Screen,
    worker::{Event, Job, work},
};
use simple_engine::render2d::{Point, Rect, text::TextSystem};
use std::{
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{Key, ModifiersState, NamedKey},
    window::{Window, WindowId},
};

struct App<'w> {
    slot: &'w OnceLock<Window>,
    screen: Option<Screen<'w>>,
    text: TextSystem,
    model: Model,
    generations: &'w [AtomicU64; 2],
    jobs: mpsc::Sender<Job>,
    deadline: Option<Instant>,
    dirty_image: bool,
    dirty_pupil: bool,
    modifiers: ModifiersState,
    error: Option<Box<dyn std::error::Error>>,
}
impl App<'_> {
    fn redraw(&self) {
        if let Some(w) = self.slot.get() {
            w.request_redraw();
        }
    }
    fn generation(&self, export: bool) -> u64 {
        self.generations[usize::from(export)].load(Ordering::Relaxed)
    }
    fn advance(&self, export: bool) -> u64 {
        self.generations[usize::from(export)].fetch_add(1, Ordering::Relaxed) + 1
    }
    fn schedule(&mut self) {
        self.advance(false);
        self.deadline = Some(Instant::now() + Duration::from_millis(100));
        self.model.preview_status = "正在更新预览…".into();
        self.redraw();
    }
    fn submit(&self, destination: Option<PathBuf>) -> Result<()> {
        let generation = self.generation(destination.is_some());
        self.jobs
            .send(Job {
                generation,
                parameters: self.model.recipe.parameters.clone(),
                destination,
            })
            .map_err(|_| "计算线程已退出")?;
        Ok(())
    }
    fn fail(&mut self, events: &ActiveEventLoop, e: Box<dyn std::error::Error>) {
        self.error = Some(e);
        events.exit();
    }
    fn load(&mut self, path: &Path) -> Result<()> {
        if path
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("png"))
        {
            self.model.recipe.parameters.import_png(path)?;
            self.model.tab = 0;
        } else {
            self.model.recipe = Recipe::open(path)?;
        }
        self.model.edit = None;
        self.model.scroll = 0.;
        self.model.file_status = format!("已打开 {}", path.display());
        self.schedule();
        Ok(())
    }
    fn parameter_changed(&mut self) {
        if self
            .model
            .active_layer()
            .is_none_or(|layer| layer.enabled(&self.model.recipe.parameters))
        {
            self.schedule();
        } else {
            self.redraw();
        }
    }
    fn set(&mut self, f: Field, value: f64) {
        if value.is_finite() {
            f.set(&mut self.model.recipe, value);
            if f == Field::Exposure {
                self.dirty_image = true;
                self.redraw();
            } else {
                self.parameter_changed();
            }
        }
    }
    fn commit_edit(&mut self) -> Result<()> {
        if let Some((f, value)) = self.model.edit.take() {
            let v = value.parse::<f64>()?;
            let (_, lo, hi, _) = f.spec();
            if !(lo..=hi).contains(&v) {
                return Err(format!("数值范围为 {lo} 至 {hi}").into());
            }
            self.set(f, v);
        }
        Ok(())
    }
    fn action(&mut self, action: Control, rect: Rect) -> Result<()> {
        use Control::*;
        self.commit_edit()?;
        let window = self.slot.get().expect("窗口接收控件操作");
        let r = &mut self.model.recipe;
        let p = &mut r.parameters;
        match action {
            Tab(t) => {
                self.model.tab = t;
                self.model.scroll = 0.;
            }
            Shape(s) => {
                p.shape = s;
                self.model.scroll = 0.;
                self.parameter_changed();
            }
            Light(s) => {
                p.source = s;
                self.schedule();
            }
            Channels(v) => {
                p.response = v;
                self.schedule();
            }
            Precision(v) => p.quality = v,
            Size(v) => {
                p.side = v;
                self.schedule();
            }
            FieldOfView(v) => {
                p.preview_side = v;
                self.schedule();
            }
            View(v) => {
                r.view = v;
                self.dirty_image = true;
            }
            Zoom(v) => r.zoom = v,
            Slider(f) => {
                self.model.drag = Some((f, rect));
                self.slide(f, rect);
            }
            Edit(f) => {
                self.model.edit = Some((f, f.format(r)));
                self.model.replace_text = true;
            }
            Seed(f) => {
                let seed = (f.value(r) as u32)
                    .wrapping_mul(1664525)
                    .wrapping_add(1013904223);
                f.set(r, f64::from(seed));
                self.parameter_changed();
            }
            Enable(layer, enabled) => {
                if layer.enabled(p) != enabled {
                    layer.set(p, enabled);
                    self.schedule();
                }
            }
            BladeEdge(edge) => {
                p.blades.edge = edge;
                self.parameter_changed();
            }
            Reset => {
                self.model.reset_layer();
                self.parameter_changed();
            }
            Open => {
                if let Some(path) = rfd::FileDialog::new()
                    .set_parent(window)
                    .add_filter("光瞳参数", &["json"])
                    .pick_file()
                {
                    self.load(&path)?;
                }
            }
            Mask => {
                if let Some(path) = rfd::FileDialog::new()
                    .set_parent(window)
                    .add_filter("光瞳振幅", &["png"])
                    .pick_file()
                {
                    self.load(&path)?;
                }
            }
            Save => {
                if let Some(path) = rfd::FileDialog::new()
                    .set_parent(window)
                    .set_file_name("光瞳参数.json")
                    .add_filter("光瞳参数", &["json"])
                    .save_file()
                {
                    r.save(&path)?;
                    self.model.file_status = format!("已保存 {}", path.display());
                }
            }
            Export => {
                if !self.model.exporting
                    && let Some(path) = rfd::FileDialog::new()
                        .set_parent(window)
                        .set_file_name("diffraction.ktx2")
                        .add_filter("KTX2 无损浮点核", &["ktx2"])
                        .add_filter("原始小端 RGB 浮点核", &["rgb32le"])
                        .save_file()
                {
                    self.advance(true);
                    self.submit(Some(path))?;
                    self.model.exporting = true;
                    self.model.file_status = "正在导出…".into();
                }
            }
            Cancel => {
                self.advance(true);
                self.model.exporting = false;
                self.model.file_status = "已取消导出".into();
            }
        }
        self.redraw();
        Ok(())
    }
    fn slide(&mut self, f: Field, r: Rect) {
        let (_, lo, hi, _) = f.spec();
        let t = f64::from(((self.model.cursor.x - r.x) / r.width).clamp(0., 1.));
        self.set(f, lo + t * (hi - lo));
    }
    fn draw(&mut self) -> Result<()> {
        let window = self.slot.get().expect("绘制事件来自已创建窗口");
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        let Some(screen) = &mut self.screen else {
            return Ok(());
        };
        if let Some(p) = &self.model.preview {
            if self.dirty_image {
                screen.upload(
                    0,
                    p.side,
                    &p.rgba(self.model.recipe.view, self.model.recipe.exposure),
                )?;
                self.dirty_image = false;
            }
            if self.dirty_pupil {
                let rgba: Vec<_> = p.pupil.iter().flat_map(|&v| [v, v, v, 255]).collect();
                screen.upload(1, THUMB, &rgba)?;
                self.dirty_pupil = false;
            }
        }
        let scale = window.scale_factor();
        let logical = size.to_logical::<f32>(scale);
        let bounds = Rect::new(0., 0., logical.width, logical.height);
        let scene = panel::draw(
            &mut self.model,
            &mut self.text,
            bounds,
            scale as f32,
            screen.keys(),
        )?;
        if !screen.draw([size.width, size.height], bounds, &scene, &mut self.text)? {
            window.request_redraw();
        }
        Ok(())
    }
    fn key(&mut self, key: &winit::event::KeyEvent) -> Result<()> {
        if key.state != ElementState::Pressed {
            return Ok(());
        }
        if self.model.edit.is_some() {
            match &key.logical_key {
                Key::Named(NamedKey::Escape) => self.model.edit = None,
                Key::Named(NamedKey::Enter) => self.commit_edit()?,
                Key::Named(NamedKey::Backspace) => {
                    let v = &mut self.model.edit.as_mut().expect("活动数值编辑").1;
                    if self.model.replace_text {
                        v.clear();
                    } else {
                        v.pop();
                    }
                    self.model.replace_text = false;
                }
                _ => {
                    if let Some(t) = key.text.as_ref().filter(|s| {
                        s.chars()
                            .all(|c| c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E'))
                    }) {
                        let v = &mut self.model.edit.as_mut().expect("活动数值编辑").1;
                        if self.model.replace_text {
                            v.clear();
                            self.model.replace_text = false;
                        }
                        if v.len() < 30 {
                            v.push_str(t);
                        }
                    }
                }
            }
        } else if self.modifiers.control_key()
            && let Key::Character(ch) = &key.logical_key
        {
            let action = match ch.to_ascii_lowercase().as_str() {
                "o" => Control::Open,
                "s" => Control::Save,
                "e" => Control::Export,
                _ => return Ok(()),
            };
            self.action(action, Rect::default())?;
        }
        self.redraw();
        Ok(())
    }
}
impl ApplicationHandler<Event> for App<'_> {
    fn resumed(&mut self, events: &ActiveEventLoop) {
        let init = || -> Result<_> {
            if self.slot.get().is_none() {
                let window = events.create_window(
                    Window::default_attributes()
                        .with_title("衍射核工作台 · Simple Engine")
                        .with_inner_size(LogicalSize::new(1240, 770))
                        .with_min_inner_size(LogicalSize::new(820, 520)),
                )?;
                let _ = self.slot.set(window);
            }
            Screen::new(self.slot.get().expect("窗口已建立"))
        };
        match init() {
            Ok(s) => {
                self.screen = Some(s);
                self.dirty_image = true;
                self.dirty_pupil = true;
                self.redraw();
            }
            Err(e) => self.fail(events, e),
        }
    }
    fn suspended(&mut self, _: &ActiveEventLoop) {
        self.screen = None;
    }
    fn about_to_wait(&mut self, events: &ActiveEventLoop) {
        if let Some(time) = self.deadline {
            if Instant::now() >= time {
                self.deadline = None;
                if let Err(e) = self.submit(None) {
                    return self.fail(events, e);
                }
            } else {
                events.set_control_flow(ControlFlow::WaitUntil(time));
                return;
            }
        }
        events.set_control_flow(ControlFlow::Wait);
    }
    fn user_event(&mut self, _: &ActiveEventLoop, event: Event) {
        match event {
            Event::Progress(id, export, f) if self.generation(export) == id => {
                let text = format!(
                    "正在{}  {:.0}%",
                    if export { "导出" } else { "计算" },
                    100. * f
                );
                if export {
                    self.model.file_status = text;
                } else {
                    self.model.preview_status = text;
                }
            }
            Event::Preview(id, p, status) if self.generation(false) == id => {
                self.model.preview = Some(p);
                self.model.preview_status = status;
                self.dirty_image = true;
                self.dirty_pupil = true;
            }
            Event::Export(id, staged, path, start) if self.generation(true) == id => {
                self.model.exporting = false;
                self.model.file_status = match staged.persist(&path) {
                    Ok(_) => format!(
                        "已导出 {}  ·  {:.2} s",
                        path.display(),
                        start.elapsed().as_secs_f64()
                    ),
                    Err(e) => e.to_string(),
                };
            }
            Event::Failed(id, export, error) if self.generation(export) == id => {
                if export {
                    self.model.exporting = false;
                    self.model.file_status = error;
                } else {
                    self.model.preview_status = error;
                }
            }
            _ => return,
        }
        self.redraw();
    }
    fn window_event(&mut self, events: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(window) = self.slot.get() else {
            return;
        };
        let scale = window.scale_factor();
        let result: Result<()> = match event {
            WindowEvent::CloseRequested => {
                events.exit();
                Ok(())
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = self.draw() {
                    self.fail(events, e);
                }
                return;
            }
            WindowEvent::ScaleFactorChanged { .. } | WindowEvent::Resized(_) => {
                self.redraw();
                Ok(())
            }
            WindowEvent::ModifiersChanged(v) => {
                self.modifiers = v.state();
                Ok(())
            }
            WindowEvent::KeyboardInput { event, .. } => self.key(&event),
            WindowEvent::DroppedFile(path) => self.load(&path),
            WindowEvent::CursorMoved { position, .. } => {
                let v = position.to_logical::<f32>(scale);
                self.model.cursor = Point::new(v.x, v.y);
                if let Some((f, r)) = self.model.drag {
                    self.slide(f, r);
                }
                self.redraw();
                Ok(())
            }
            WindowEvent::MouseInput {
                button: MouseButton::Left,
                state: ElementState::Pressed,
                ..
            } => {
                if let Some(h) = self
                    .model
                    .hits
                    .iter()
                    .rev()
                    .find(|h| h.rect.contains(self.model.cursor))
                    .copied()
                {
                    self.action(h.action, h.rect)
                } else {
                    self.commit_edit()
                }
            }
            WindowEvent::MouseInput {
                button: MouseButton::Left,
                state: ElementState::Released,
                ..
            }
            | WindowEvent::Focused(false) => {
                self.model.drag = None;
                Ok(())
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let amount = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y * 36.,
                    MouseScrollDelta::PixelDelta(p) => p.to_logical::<f32>(scale).y,
                };
                if self.model.screen_rect.contains(self.model.cursor) {
                    self.model.recipe.zoom =
                        (self.model.recipe.zoom * 2_f32.powf(amount / 160.)).clamp(0.25, 8.);
                } else {
                    self.model.scroll =
                        (self.model.scroll - amount).clamp(0., self.model.max_scroll);
                }
                self.redraw();
                Ok(())
            }
            _ => Ok(()),
        };
        if let Err(e) = result {
            self.model.file_status = e.to_string();
            self.redraw();
        }
    }
}
pub(crate) fn run(recipe: Recipe, adapter: usize) -> Result<()> {
    let events = EventLoop::<Event>::with_user_event().build()?;
    let proxy = events.create_proxy();
    let text = TextSystem::new(
        [include_bytes!("../../../assets/fonts/NotoSansSC-Regular.otf").to_vec()],
        Default::default(),
    )?;
    let generations = [AtomicU64::new(1), AtomicU64::new(0)];
    let slot = OnceLock::new();
    let (jobs, rx) = mpsc::channel();
    std::thread::scope(|scope| -> Result<()> {
        let thread = std::thread::Builder::new()
            .name("衍射计算".into())
            .spawn_scoped(scope, || work(rx, proxy, &generations, adapter))?;
        let mut app = App {
            slot: &slot,
            screen: None,
            text,
            model: Model::new(recipe),
            generations: &generations,
            jobs,
            deadline: Some(Instant::now()),
            dirty_image: true,
            dirty_pupil: true,
            modifiers: ModifiersState::empty(),
            error: None,
        };
        let loop_result = events.run_app(&mut app);
        app.advance(false);
        app.advance(true);
        let error = app.error.take();
        drop(app);
        thread.join().map_err(|_| "衍射计算线程异常退出")?;
        loop_result?;
        if let Some(e) = error {
            return Err(e);
        }
        Ok(())
    })
}
