//! 地形遮挡的太阳体积光。五级 512² 环形网格按极线需求从当前 TLAS 查询最近阻挡深度。
//! 每条极线共享太阳阻挡剖面。相机射线以首个覆盖命中为上限，四波长积分直接写入前 2048 m 的受光辐亮度。
//! 额外的末层保存天空需扣除的近场能量，以及有限远段延用的边界可见度。
//! 晴空天空、远场空气与透射仍由同一模型的常规输出提供。世界单位为米，参数位置与 TLAS 同原点。
//! 调用者独占一份视图缓存，顺序登记、上传参数、录制，再在实际提交后确认。
//! 近场边界外未追踪新的太阳阻挡列，有限远段用最后一个受支持区间的可见度近似。
//!
//! ## 极线算法的来源与差异
//!
//! Intel 的 DX11 样例沿级联阴影图建立每切片的一维 min/max 层级，在选定的屏幕样本上
//! 积分至场景深度，按深度双边权重重建；可再对深度断点的像素补算。它提供多种单次与多次散射模式。
//! [原始着色器](https://github.com/GameTechDev/OutdoorLightScattering/blob/3b31b3b8c1aaad8580dc7249b3b79f6c0993d7a8/fx/LightScattering.fx)
//! 与 [采样细化](https://github.com/GameTechDev/OutdoorLightScattering/blob/3b31b3b8c1aaad8580dc7249b3b79f6c0993d7a8/fx/RefineSampleLocations.fx)。
//!
//! 旧 Prime PT `e0260fc4` 使用五级按需 RT 阻挡列，在极线共享内存中遍历深度剖面，
//! 对 128 个距离层做四波长受光积分，在 post 对相邻切片插值。
//! 引擎复用这套剖面，以当前刷新切片的相机首命中裁剪可见空气段，并按相邻射线的首命中距离重建。
//! 缺少可信近场样本时只合成背景透射，避免跨洞口补入亮侧散射。
use crate::{Error, Parameters, View};
use vulkan::{
    AccelerationStructure,
    interop::RayShader,
    native::{
        Binding, Bindings, Buffer, Commands, ComputePipeline, Context, Image, ImageFormat,
        Recorded, Resource,
    },
};

const RADIAL: u32 = 128;
const SLICES: u32 = 256;
const DEPTH: u32 = 128;
const CELLS_BYTES: u64 = 5 * 512 * 512 * 16;
const PROFILE_BYTES: u64 = 256 * (512 + 4 * 256 + 6) * 4;
const PHASES: u8 = 8;

/// 与已初始化大气所有者共享的物理输入，由同一设备创建，交给体积光管线保活。
pub struct TerrainModel([Binding; 8]);
impl TerrainModel {
    pub(crate) fn new(
        ctx: &mut Context,
        p: &Parameters,
        fields: [Binding; 7],
    ) -> Result<Self, Error> {
        let constants = Buffer::upload(ctx, bytemuck::bytes_of(p))?;
        let [data, tau, source, mean, ground, sampler, high] = fields;
        Ok(Self([
            Binding::UniformBuffer(constants),
            data,
            tau,
            source,
            mean,
            ground,
            sampler,
            high,
        ]))
    }
}

/// 所有者借出的本次上传参数；缓存决策保留到同一所有者录制。
pub struct TerrainUpdate {
    input: [[u32; 4]; 10],
    key: Key,
    full: bool,
    remaining: u8,
}
impl TerrainUpdate {
    /// 每份输入的字节数。
    pub const INPUT_BYTES: usize = std::mem::size_of::<[[u32; 4]; 10]>();
    /// 写到创建所有者时指定的只读 storage buffer，提交先于 record。
    pub fn bytes(&self) -> &[u8] {
        bytemuck::bytes_of(&self.input)
    }
}
#[derive(Clone, Copy, PartialEq)]
struct Key {
    view: View,
    origin: [f32; 3],
    world: [f64; 3],
    active: bool,
    clock: u32,
    view_version: u32,
}

/// 一台设备上一份视图的极线光照与太阳阻挡缓存。每个输出 texel 对应一条极线、径向样本与累积深度。
pub struct TerrainLight {
    frame: Buffer,
    cells: Buffer,
    profiles: Buffer,
    radiance: Image,
    first_hits: Buffer,
    prepare: ComputePipeline,
    demand: ComputePipeline,
    integrate: ComputePipeline,
    history: History,
    planned: Option<TerrainUpdate>,
}
fn program(
    ctx: &mut Context,
    bindings: &[Binding],
    spirv: &[u8],
    entry: &std::ffi::CStr,
) -> Result<ComputePipeline, Error> {
    let set = Bindings::new(ctx, bindings)?;
    Ok(ComputePipeline::new(ctx, set, RayShader { spirv, entry })?)
}
impl TerrainLight {
    /// 创建常驻资源与计算程序。input 须是同设备的 storage buffer，容量至少为 [`TerrainUpdate::INPUT_BYTES`]。
    /// # Errors
    /// 设备分配、绑定或程序创建失败返回底层错误。
    pub fn new(ctx: &mut Context, model: TerrainModel, input: Buffer) -> Result<Self, Error> {
        Self::build(ctx, model, Binding::StorageBuffer(input), input.len())
    }
    /// 使用同队列的共享 storage 输入；应用可在上一帧在途时通过 wgpu queue 写入新参数。
    /// # Errors
    /// 输入长度不足、设备不匹配、资源或程序创建失败时返回错误。
    #[cfg(feature = "gpu")]
    pub fn new_shared(
        ctx: &mut Context,
        model: TerrainModel,
        input: &vulkan::interop::SharedBuffer,
    ) -> Result<Self, Error> {
        Self::build(
            ctx,
            model,
            Binding::SharedBuffer(input.clone()),
            input.len(),
        )
    }
    fn build(
        ctx: &mut Context,
        model: TerrainModel,
        input: Binding,
        len: u64,
    ) -> Result<Self, Error> {
        if len < TerrainUpdate::INPUT_BYTES as u64 {
            return Err(Error::Limit("体积光输入参数长度不足"));
        }
        let frame = Buffer::new(ctx, 192, false)?;
        let cells = Buffer::new(ctx, CELLS_BYTES, false)?;
        let profiles = Buffer::new(ctx, PROFILE_BYTES, false)?;
        let radiance = Image::new(ctx, [RADIAL, SLICES, DEPTH + 1], ImageFormat::Rgba16Float)?;
        let first_hits = Buffer::new(ctx, u64::from(RADIAL * SLICES) * 4, false)?;
        let prepare = program(
            ctx,
            &[input, Binding::StorageBuffer(frame)],
            include_bytes!(concat!(env!("OUT_DIR"), "/shadow_prepare.spv")),
            c"shadow_prepare",
        )?;
        let demand = program(
            ctx,
            &[Binding::StorageBuffer(frame), Binding::StorageBuffer(cells)],
            include_bytes!(concat!(env!("OUT_DIR"), "/shadow_demand.spv")),
            c"shadow_demand",
        )?;
        let mut fields = Vec::from(model.0);
        fields.extend([
            Binding::StorageBuffer(frame),
            Binding::StorageBuffer(cells),
            Binding::StorageBuffer(profiles),
            Binding::StorageImage(radiance),
            Binding::StorageBuffer(first_hits),
        ]);
        let integrate = program(
            ctx,
            &fields,
            include_bytes!(concat!(env!("OUT_DIR"), "/terrain_integrate.spv")),
            c"terrain_integrate",
        )?;
        Ok(Self {
            frame,
            cells,
            profiles,
            radiance,
            first_hits,
            prepare,
            demand,
            integrate,
            history: History::new(),
            planned: None,
        })
    }
    /// 使用应用提供的候选三角覆盖规则建立最近命中查询。已有程序只重绑描述符。
    /// shader 的前四个绑定依次为 frame、太阳深度列、TLAS、相机首命中列，其余由 coverage 按声明顺序提供。
    /// # Errors
    /// 各绑定、模板或 TLAS 的设备归属和接口不匹配，或分配失败时返回错误。
    pub fn resolver(
        &self,
        ctx: &mut Context,
        previous: Option<ComputePipeline>,
        shader: RayShader<'_>,
        top: AccelerationStructure,
        coverage: &[Binding],
    ) -> Result<ComputePipeline, Error> {
        let mut bindings = Vec::new();
        bindings.try_reserve_exact(4 + coverage.len())?;
        bindings.extend([
            Binding::StorageBuffer(self.frame),
            Binding::StorageBuffer(self.cells),
            Binding::Acceleration(top),
            Binding::StorageBuffer(self.first_hits),
        ]);
        bindings.extend_from_slice(coverage);
        let set = Bindings::new(ctx, &bindings)?;
        Ok(if let Some(p) = previous {
            p.rebind(ctx, set)?
        } else {
            ComputePipeline::new(ctx, set, shader)?
        })
    }
    /// 普通编辑、LOD 和材质覆盖变化请求一轮自然刷新，保持全部阻挡列与已发布切片可用。
    pub fn request_refresh(&mut self) {
        self.history.refresh_requested = true;
    }
    /// 显式全局事件。返回此前是否有已提交的体积光，下次实际提交的空气视图重建整个缓存。
    pub fn reset_history(&mut self) -> bool {
        let valid = !self.history.cold;
        self.history = History::new();
        self.planned = None;
        valid
    }
    /// 选择本次更新。初次使用或显式重置构建全部切片，其余每帧轮转八分之一。
    /// 相机和太阳移动时继续使用旧历史。固定视图下局部编辑至多经过两个轮转便已覆盖全部切片，
    /// 随后停止调度。world 是高精度世界坐标，origin 是同一位置的 TLAS 相对坐标，单位均为米。
    /// # Errors
    /// 视图或位置不合法时返回错误，已提交缓存继续有效。
    pub fn prepare(
        &mut self,
        view: View,
        origin: [f32; 3],
        world: [f64; 3],
        active: bool,
    ) -> Result<Option<&TerrainUpdate>, Error> {
        self.planned = None;
        self.planned = self.history.prepare(view, origin, world, active)?;
        Ok(self.planned.as_ref())
    }
    /// 录制需求、最近命中与积分。先上传 update 的参数，提交后调用 submitted，取消时不确认。
    /// # Errors
    /// 设备、程序或资源不匹配或录制失败时保留上次提交键。
    pub fn record(
        &mut self,
        ctx: &mut Context,
        resolve: ComputePipeline,
    ) -> Result<Recorded, Error> {
        let update = self
            .planned
            .take()
            .ok_or(Error::View("体积光尚无待录制的更新"))?;
        let mut cmds = Commands::new(ctx)?;
        cmds.barrier();
        cmds.dispatch(self.prepare, [1; 3])?;
        if update.key.active {
            if update.full {
                cmds.clear_buffer(self.cells)?;
            }
            let slices = if update.full {
                SLICES
            } else {
                SLICES / u32::from(PHASES)
            };
            cmds.barrier();
            cmds.dispatch(self.demand, [1, slices, 1])?;
            cmds.barrier();
            cmds.dispatch(resolve, [64, 64, 5])?;
            cmds.barrier();
            cmds.dispatch(self.integrate, [1, slices, 1])?;
        }
        cmds.barrier();
        let recorded = cmds.finish(ctx)?;
        self.history.recorded(&update);
        Ok(recorded)
    }
    /// 接受实际已提交的记录，随后可复用其列与剖面。
    pub fn submitted(&mut self) {
        self.history.submitted();
    }
    /// 取消尚未提交的更新。下一次 prepare 仍以上次实际提交为准。
    pub fn cancel(&mut self) {
        self.planned = None;
        self.history.pending = None;
    }
    /// 供 post 使用的极线坐标、受光积分纹理和相机首命中列。绑定与本次提交的缓存相同。
    pub const fn outputs(&self) -> (Buffer, Image, Buffer) {
        (self.frame, self.radiance, self.first_hits)
    }
    /// 常驻 GPU payload 字节数，排除共享物理模型、应用输入与驱动对齐。
    pub const fn bytes(&self) -> u64 {
        CELLS_BYTES
            + PROFILE_BYTES
            + RADIAL as u64 * SLICES as u64 * (DEPTH + 1) as u64 * 8
            + RADIAL as u64 * SLICES as u64 * 4
            + 192
            + 400
    }
    /// 场景收集时保活的资源；计算程序间接保活绑定与共享物理输入。
    pub fn roots(&self) -> impl Iterator<Item = Resource> {
        [
            self.prepare.into(),
            self.demand.into(),
            self.integrate.into(),
            self.frame.into(),
            self.radiance.into(),
            self.first_hits.into(),
            self.profiles.into(),
            self.cells.into(),
        ]
        .into_iter()
    }
}

struct History {
    last: Option<Key>,
    pending: Option<(Key, u8)>,
    cold: bool,
    refresh_requested: bool,
    remaining: u8,
}
impl History {
    const fn new() -> Self {
        Self {
            last: None,
            pending: None,
            cold: true,
            refresh_requested: false,
            remaining: 0,
        }
    }
    fn recorded(&mut self, update: &TerrainUpdate) {
        self.pending = Some((update.key, update.remaining));
    }
    fn submitted(&mut self) {
        if let Some((key, remaining)) = self.pending.take() {
            self.last = Some(key);
            self.remaining = remaining;
            if key.active {
                self.cold = false;
                self.refresh_requested = false;
            }
        }
    }
    fn prepare(
        &mut self,
        view: View,
        origin: [f32; 3],
        world: [f64; 3],
        active: bool,
    ) -> Result<Option<TerrainUpdate>, Error> {
        let view = view.validated()?;
        if !origin.iter().all(|v| v.is_finite()) || !world.iter().all(|v| v.is_finite()) {
            return Err(Error::View("体积光相机坐标非有限"));
        }
        let active = active && view.radiance_scale.iter().any(|&x| x > 0.);
        self.pending = None;
        if !active && self.last.is_some_and(|k| !k.active) {
            return Ok(None);
        }
        let full = active && self.cold;
        let view_changed = self
            .last
            .is_none_or(|k| k.view != view || k.world != world || k.active != active);
        let changed =
            view_changed || self.refresh_requested || self.last.is_none_or(|k| k.origin != origin);
        if active && !full && !changed && self.remaining == 0 {
            return Ok(None);
        }
        let remaining = if full || !active {
            0
        } else if changed {
            2 * PHASES - 1
        } else {
            self.remaining - 1
        };
        let clock = self
            .last
            .map_or(0, |k| k.clock.wrapping_add(1) & 0x7fffffff);
        let version = self
            .last
            .map_or(1, |k| k.view_version.wrapping_add(u32::from(view_changed)));
        let sun = view.sun_direction;
        let [x, y, z] = sun.map(f64::from);
        let mut u = if y < -0.9999999 {
            [1., 0., 0.]
        } else {
            [1. - x * x / (1. + y), -x, -x * z / (1. + y)]
        };
        let norm = u.iter().map(|v| v * v).sum::<f64>().sqrt();
        u = u.map(|v| v / norm);
        let v = [
            y * u[2] - z * u[1],
            z * u[0] - x * u[2],
            x * u[1] - y * u[0],
        ];
        let dot = |dir: [f64; 3]| world.into_iter().zip(dir).map(|(a, b)| a * b).sum::<f64>();
        let depth = dot([x, y, z]);
        let plane = ((depth / 2048.).floor() + 0.5) * 2048.;
        let uv = [dot(u), dot(v)];
        if !plane.is_finite() || !uv.iter().all(|a| a.is_finite()) {
            return Err(Error::View("体积光世界投影越界"));
        }
        // 先以 f64 减去最大的环形周期，GPU 的 DDA 始终在小坐标中步进。整数格标签跨 anchor 保持稳定。
        let tile = uv.map(|a| (a / 4096.).floor().rem_euclid(4294967296.) as u32);
        let row = |v: [f32; 3], w: f32| [v[0], v[1], v[2], w].map(f32::to_bits);
        let state = [
            clock,
            u32::from(active),
            version,
            (clock & 7) | (u32::from(full) << 3),
        ];
        Ok(Some(TerrainUpdate {
            input: [
                row(origin, view.altitude_km),
                row(view.forward, 0.),
                row(view.right, 0.),
                row(view.up, 0.),
                row(sun, 0.),
                row(view.radiance_scale, 0.),
                state,
                row(u.map(|a| a as f32), (plane - depth) as f32),
                row(v.map(|a| a as f32), plane.rem_euclid(32768.) as f32),
                [
                    (uv[0].rem_euclid(4096.) as f32).to_bits(),
                    (uv[1].rem_euclid(4096.) as f32).to_bits(),
                    tile[0],
                    tile[1],
                ],
            ],
            key: Key {
                view,
                origin,
                world,
                active,
                clock,
                view_version: version,
            },
            full,
            remaining,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordinary_changes_keep_history_and_refresh_every_slice() -> Result<(), Error> {
        let mut history = History::new();
        let mut view = View::default();
        let start = history.prepare(view, [0.; 3], [0.; 3], true)?.unwrap();
        assert!(start.full);
        history.recorded(&start);
        history.submitted();
        view.sun_direction = [0.6, 0.8, 0.];
        history.refresh_requested = true;
        let mut visited = [0; 8];
        for _ in 0..16 {
            let u = history
                .prepare(view, [8.; 3], [30000008.; 3], true)?
                .unwrap();
            assert!(!u.full);
            visited[(u.key.clock & 7) as usize] += 1;
            history.recorded(&u);
            history.submitted();
        }
        assert_eq!(visited, [2; 8]);
        assert!(
            history
                .prepare(view, [8.; 3], [30000008.; 3], true)?
                .is_none()
        );
        Ok(())
    }
    #[test]
    fn cancelled_upload_retries_the_same_phase() -> Result<(), Error> {
        let mut h = History::new();
        let view = View::default();
        let first = h.prepare(view, [0.; 3], [0.; 3], true)?.unwrap();
        h.recorded(&first);
        h.submitted();
        h.refresh_requested = true;
        let dropped = h.prepare(view, [0.; 3], [0.; 3], true)?.unwrap();
        h.recorded(&dropped);
        h.pending = None;
        let retry = h.prepare(view, [0.; 3], [0.; 3], true)?.unwrap();
        assert_eq!(dropped.input, retry.input);
        Ok(())
    }

    #[test]
    fn rebasing_the_tlas_keeps_world_shadow_columns() -> Result<(), Error> {
        let mut history = History::new();
        let view = View::default();
        let world = [30_000_000.25, 96.5, -29_999_995.75];
        let first = history.prepare(view, [0.; 3], world, true)?.unwrap();
        history.recorded(&first);
        history.submitted();
        let rebased = history
            .prepare(view, [64., 0., -64.], world, true)?
            .unwrap();
        assert_eq!(first.input[7..], rebased.input[7..]);
        assert_eq!(first.key.view_version, rebased.key.view_version);
        assert!(!rebased.full);
        Ok(())
    }
}
