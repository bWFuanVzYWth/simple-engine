//! 队列中预览可被新版本替换，导出持有点击时的快照，另有取消代次。
use crate::{
    preview::{Preview, THUMB},
    recipe::{Parameters, Result, stage_kernel},
};
use simple_engine::{fft_bloom::diffraction::Solver, vulkan::interop::WgpuContext};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::Receiver,
    },
    time::Instant,
};
use winit::event_loop::EventLoopProxy;

pub(crate) struct Job {
    pub generation: u64,
    pub parameters: Parameters,
    pub destination: Option<PathBuf>,
}
pub(crate) enum Event {
    Progress(u64, bool, f32),
    Preview(u64, Preview, String),
    Export(u64, tempfile::NamedTempFile, PathBuf, Instant),
    Failed(u64, bool, String),
}
pub(crate) fn work(
    rx: Receiver<Job>,
    proxy: EventLoopProxy<Event>,
    generations: &[AtomicU64; 2],
    adapter: usize,
) {
    let mut solver = None;
    for job in rx {
        let export = job.destination.is_some();
        let valid = || generations[usize::from(export)].load(Ordering::Relaxed) == job.generation;
        if !valid() {
            continue;
        }
        let mut task = || -> Result<Option<Event>> {
            let started = Instant::now();
            let p = &job.parameters;
            let request = p.request(!export);
            let pupil = p.pupil()?;
            if solver.is_none() {
                solver = Some(Solver::new(pollster::block_on(WgpuContext::with_index(
                    adapter, false,
                ))?)?);
            }
            let frame =
                solver
                    .as_mut()
                    .expect("工作线程拥有求解器")
                    .solve(&pupil, request, |q| {
                        valid()
                            && proxy
                                .send_event(Event::Progress(job.generation, export, q))
                                .is_ok()
                    })?;
            if !valid() {
                return Ok(None);
            }
            if let Some(path) = &job.destination {
                let staged = stage_kernel(path, &frame)?;
                return Ok(Some(Event::Export(
                    job.generation,
                    staged,
                    path.clone(),
                    started,
                )));
            }
            let image = pupil.thumbnail(
                THUMB,
                if p.source == crate::recipe::Lamp::Line {
                    p.wavelength
                } else {
                    550.
                },
            )?;
            let Some(preview) = Preview::new(&frame, p, image, valid) else {
                return Ok(None);
            };
            Ok(Some(Event::Preview(
                job.generation,
                preview,
                format!(
                    "GPU 重算  ·  {:.0} ms",
                    started.elapsed().as_secs_f64() * 1000.
                ),
            )))
        };
        let event = match task() {
            Ok(v) => v,
            Err(e) if valid() => Some(Event::Failed(job.generation, export, e.to_string())),
            Err(_) => None,
        };
        if let Some(event) = event
            && proxy.send_event(event).is_err()
        {
            break;
        }
    }
}
