//! 宽带圆孔的空间域近似。近场为原分辨率 5×5 像素面积积分的残差，远翼为九层五阶二项式金字塔。
//! 上采样使用下采样的四倍转置。无限平面上每个输入像素的 RGB 通量逐通道守恒，每层两侧各留两格，接住画外的滤波足迹，最终只裁去落在视图外的光。

#![cfg(not(target_arch = "wasm32"))]
use vulkan::{
    interop::RayShader,
    native::{
        Binding, Bindings, ComputePipeline, Context, Image, ImageFormat, ImagePasses, Resource,
    },
};

include!(concat!(env!("OUT_DIR"), "/levels.rs"));

/// 圆形泛光的设备管线。可独立创建，所有视图共用标定权重，工作图仅随视图持有。
/// 光学参考为 f/4 圆孔、5 µm 像素与引擎三个等能波段。中心 5×5 对像素和波段积分，
/// 远翼用九个尺度拟合宽带 Airy 的径向衰减，长波的光晕更宽。
/// 参见 [Wyant 与 Creath 的圆孔衍射模型](https://wp.optics.arizona.edu/jcwyant/wp-content/uploads/sites/13/2016/08/03-BasicAberrations_and_Optical_Testing.pdf)。
pub struct Filter {
    pipelines: [ComputePipeline; 5],
}

fn bindings(
    ctx: &mut Context,
    source: Image,
    output: Image,
    detail: Image,
) -> Result<Bindings, vulkan::Error> {
    Bindings::new(
        ctx,
        &[
            Binding::SampledImage(source),
            Binding::StorageImage(output),
            Binding::SampledImage(detail),
        ],
    )
}

impl Filter {
    /// 缩小、重建与全分辨率合成的生产 Slang 程序，按录制阶段排列。
    pub const SHADERS: [RayShader<'static>; 5] = [
        RayShader {
            spirv: include_bytes!(concat!(env!("OUT_DIR"), "/circular_quarter.spv")),
            entry: c"circular_quarter",
        },
        RayShader {
            spirv: include_bytes!(concat!(env!("OUT_DIR"), "/circular_down.spv")),
            entry: c"circular_down",
        },
        RayShader {
            spirv: include_bytes!(concat!(env!("OUT_DIR"), "/circular_up.spv")),
            entry: c"circular_up",
        },
        RayShader {
            spirv: include_bytes!(concat!(env!("OUT_DIR"), "/circular_up_pair.spv")),
            entry: c"circular_up_pair",
        },
        RayShader {
            spirv: include_bytes!(concat!(env!("OUT_DIR"), "/circular_composite.spv")),
            entry: c"circular_composite",
        },
    ];

    /// 创建设备管线，RGB 积分系数已在构建时写入着色器。
    /// # Errors
    /// Vulkan 图像、描述符或管线准备失败时返回原始错误。
    pub fn new(ctx: &mut Context) -> Result<Self, vulkan::Error> {
        let placeholder = Image::new(ctx, [1; 3], ImageFormat::Rgba32Float)?;
        let bindings = bindings(ctx, placeholder, placeholder, placeholder)?;
        let [quarter, down, up, up_pair, composite] = Self::SHADERS;
        Ok(Self {
            pipelines: [
                ComputePipeline::new(ctx, bindings, quarter)?,
                ComputePipeline::new(ctx, bindings, down)?,
                ComputePipeline::new(ctx, bindings, up)?,
                ComputePipeline::new(ctx, bindings, up_pair)?,
                ComputePipeline::new(ctx, bindings, composite)?,
            ],
        })
    }

    /// 创建与二维输入同尺寸的输出与本视图独占的远翼工作图。所有原像素参加滤波，保留有效像素的 alpha。
    /// 含非有限分量或负 alpha 的像素原样输出，对邻域贡献为零。有效 RGB 保留符号，
    /// 工作值须在 f32 可表示范围内。采用黑色画外，落到视图外的光量随裁切丢失。
    /// # Errors
    /// 设备、布局、分配或绑定不成功时返回错误，未发布的资源由 Context 回收。
    pub fn bind(&self, ctx: &mut Context, input: Image) -> Result<ImagePasses, vulkan::Error> {
        let [width, height, depth] = input.raw(ctx)?.extent;
        if depth != 1 {
            return Err(vulkan::Error::InvalidArgument("圆形泛光输入须为二维图像"));
        }
        let output = Image::new(ctx, input.extent(), ImageFormat::Rgba32Float)?;
        let [quarter, down, up, up_pair, composite] = self.pipelines;
        // 第一轮读取原图。第一层在最终工作组内重算，从第二层开始分配图像。
        let mut levels = [input; LEVELS + 1];
        let mut passes = Vec::new();
        passes.try_reserve_exact(LEVELS + 1 + (LEVELS - 3).div_ceil(2))?;
        let mut nominal = [width, height].map(|n| n.div_ceil(2));
        for i in 2..=LEVELS {
            nominal = nominal.map(|n| n.div_ceil(2));
            // 两侧各两格，保留画外分析足迹，所有已分配层的逻辑原点均为 (−2,−2)。
            let [w, h] = nominal.map(|n| n + 4);
            levels[i] = Image::new(ctx, [w, h, 1], ImageFormat::Rgba32Float)?;
            let set = bindings(ctx, levels[i - 1], levels[i], input)?;
            let (pipeline, rows) = if i == 2 { (quarter, 4) } else { (down, 8) };
            passes.push((
                pipeline.rebind(ctx, set)?,
                [w.div_ceil(8), h.div_ceil(rows), 1],
            ));
        }
        let mut coarse = LEVELS;
        while coarse > 2 {
            // 最深一级仍是未加权的缩小图，第一次重建同时给它分配通量。
            let (fine, pipeline, detail) = if coarse == LEVELS || coarse == 3 {
                (coarse - 1, up, input)
            } else {
                (coarse - 2, up_pair, levels[coarse - 1])
            };
            let [w, h, _] = levels[fine].extent();
            let set = bindings(ctx, levels[coarse], levels[fine], detail)?;
            passes.push((
                pipeline.rebind(ctx, set)?,
                [w.div_ceil(16), h.div_ceil(16), 1],
            ));
            coarse = fine;
        }
        let set = bindings(ctx, levels[2], output, input)?;
        passes.push((
            composite.rebind(ctx, set)?,
            [width.div_ceil(16), height.div_ceil(16), 1],
        ));
        Ok(ImagePasses::new(output, passes))
    }

    /// Context 回收时须保留的设备管线。视图工作图由各自的帧绑定保活。
    pub fn roots(&self) -> impl Iterator<Item = Resource> + '_ {
        self.pipelines.map(Resource::from).into_iter()
    }
}
