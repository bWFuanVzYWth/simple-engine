//! winit 窗口与 Slang/wgpu 三角形入门应用。
#![forbid(unsafe_code)]

#[cfg(all(feature = "mimalloc", not(target_arch = "wasm32")))]
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod error;
mod renderer;

use std::{num::NonZeroU64, process::ExitCode, sync::OnceLock};

use error::Error;
use renderer::TriangleRenderer;
use simple_engine::gpu_log;
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

struct App<'w> {
    window_slot: &'w OnceLock<Window>,
    window: Option<&'w Window>,
    renderer: Option<TriangleRenderer<'w>>,
    frames_remaining: Option<u64>,
    error: Option<Error>,
}

impl App<'_> {
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: impl Into<Error>) {
        self.error = Some(error.into());
        event_loop.exit();
    }

    fn request_redraw(&self) {
        if self.renderer.is_some()
            && let Some(window) = &self.window
        {
            let size = window.inner_size();
            if size.width != 0 && size.height != 0 {
                window.request_redraw();
            }
        }
    }
}

impl<'w> ApplicationHandler for App<'w> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.renderer.is_some() {
            return;
        }

        let slot = self.window_slot;
        if slot.get().is_none() {
            let attributes = Window::default_attributes()
                .with_title("Hello Triangle")
                .with_inner_size(LogicalSize::new(960, 640));
            match event_loop.create_window(attributes) {
                Ok(window) => {
                    let _ = slot.set(window);
                }
                Err(error) => return self.fail(event_loop, error),
            }
        }
        let window = slot.get().expect("已创建窗口");

        let renderer = match TriangleRenderer::new(window) {
            Ok(renderer) => renderer,
            Err(error) => return self.fail(event_loop, error),
        };
        self.window = Some(window);
        self.renderer = Some(renderer);
        self.request_redraw();
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        // 挂起时释放 surface，恢复后重新创建。
        if let Some(renderer) = &self.renderer
            && let Err(error) = renderer.finish()
        {
            self.error.get_or_insert(error);
        }
        self.renderer = None;
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = &self.window else {
            return;
        };
        if window.id() != window_id {
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                }
                self.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                let size = window.inner_size();
                if size.width == 0 || size.height == 0 {
                    return;
                }
                let Some(renderer) = &mut self.renderer else {
                    return;
                };
                match renderer.draw() {
                    Ok(true) => {}
                    Ok(false) => return,
                    Err(error) => return self.fail(event_loop, error),
                }
                if let Some(remaining) = &mut self.frames_remaining {
                    *remaining -= 1;
                    if *remaining == 0 {
                        event_loop.exit();
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        self.request_redraw();
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(renderer) = &self.renderer
            && let Err(error) = renderer.finish()
        {
            self.error.get_or_insert(error);
        }
        self.renderer = None;
        self.window = None;
    }
}

fn frame_limit() -> Result<Option<u64>, Error> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        None => Ok(None),
        Some("--frames") => {
            let frames = args.next().ok_or(Error::MissingFrameCount)?;
            let frames = frames.parse::<NonZeroU64>()?;
            if args.next().is_some() {
                return Err(Error::Usage);
            }
            Ok(Some(frames.get()))
        }
        _ => Err(Error::Usage),
    }
}

fn run() -> Result<(), Error> {
    let window_slot = OnceLock::new();
    let mut app = App {
        window_slot: &window_slot,
        window: None,
        renderer: None,
        error: None,
        frames_remaining: frame_limit()?,
    };
    EventLoop::new()?.run_app(&mut app)?;
    if let Some(error) = app.error {
        return Err(error);
    }
    Ok(())
}

fn main() -> ExitCode {
    let result = gpu_log::checked(run);
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
