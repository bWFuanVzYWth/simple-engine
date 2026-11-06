//! 离散功率 alias 表与任意 quad 的两三角面积采样协议。
//!
//! 本 crate 不拥有纹理、材质、光源身份或设备，不决定发光功率和发射侧。
//! 调用方按真实发光面的面积、亮度及侧数提供相对权重；双面灯仍是一份几何。
//! 所有者 [`AliasTable`] 独占不可变结果；构建串行执行，不隐藏线程、调度或递归。
//! [`QuadArea`] 是面积分布的固定值，不复制四角或保留输入借用。
//!
//! 正反向概率以同一个 24-bit 均匀整数选择器为准。正权至少获得一个选择器，
//! 因而不会承诺与理想浮点权重完全成比例；MIS 须使用编码中的实际 PMF。
//! 对应 shader 为 `shaders/pt/light_sampling.slang`，独立于场景和 BSDF。
//! [`emitter`]补充资源期纹理texel裁剪与静态alias；不决定空间距离proposal或可见性。

#![forbid(unsafe_code)]

mod alias;
pub mod emitter;
mod quad;

pub use alias::{AliasSample, AliasTable};
pub use quad::QuadArea;

/// CPU 与 shader 共享的均匀整数选择器总数。
pub const SELECTORS: u32 = 1 << 24;

/// 沿几何法线正侧发射，对应 shader `LIGHT_FRONT`。
pub const FRONT_SIDE: u32 = 1;
/// 沿几何法线负侧发射，对应 shader `LIGHT_BACK`。
pub const BACK_SIDE: u32 = 2;
/// 正反两侧发射，对应 shader `LIGHT_BOTH`；不复制几何或加倍面积密度。
pub const BOTH_SIDES: u32 = FRONT_SIDE | BACK_SIDE;

/// 资源期的线性 BT.709 发光亮度估计；材质与纹理解码由应用完成。
///
/// RGB和幅度须有限非负。本值仅决定提案，不替代真实辐射；忽略动态tint和当前动画帧。
/// 零值与正数下溢保留MIN_POSITIVE支持，避免潜在发光源在静态提案中消失。
/// # Errors
/// 输入无效或乘积超过有限f32域时返回[`Error::Power`]。
pub const fn emission_weight(rgb: [f32; 3], amplitude: f32) -> Result<f32, Error> {
    let inputs = [rgb[0], rgb[1], rgb[2], amplitude];
    let mut index = 0;
    while index < inputs.len() {
        if !inputs[index].is_finite() || inputs[index] < 0. {
            return Err(Error::Power);
        }
        index += 1;
    }
    let coefficients = [0.2126, 0.7152, 0.0722];
    let mut luminance = 0.;
    index = 0;
    while index < rgb.len() {
        luminance += rgb[index] as f64 * coefficients[index];
        index += 1;
    }
    positive_power(luminance * amplitude as f64)
}

/// 以真实总面积、静态亮度估计和实际发射侧数建立BVH功率。
///
/// area须正且有限，weight须正且有限，sides属于FRONT/BACK/BOTH；双面不复制几何。
/// 只影响位置提案，正反PDF仍使用同一棵树的真实量化PMF。
/// # Errors
/// 输入无效或功率乘积超出有限f32域时返回[`Error::Power`]。
pub const fn emitter_power(area: f64, weight: f32, sides: u32) -> Result<f32, Error> {
    if !area.is_finite()
        || area <= 0.
        || !weight.is_finite()
        || weight <= 0.
        || sides == 0
        || sides > 3
    {
        return Err(Error::Power);
    }
    positive_power(area * weight as f64 * sides.count_ones() as f64)
}

const fn positive_power(value: f64) -> Result<f32, Error> {
    let value = value as f32;
    if !value.is_finite() {
        return Err(Error::Power);
    }
    Ok(if value < f32::MIN_POSITIVE {
        f32::MIN_POSITIVE
    } else {
        value
    })
}

/// 无表地近似均匀选择活动索引；不准备alias或分配任何缓冲。
///
/// 每项获得floor/ceil(2^24/count)个格点，前余数项多一格；实际PMF由同一分割给出。
/// count=0返回None；不能把任意count的有限格点分布假称为精确1/count。
/// # Errors
/// count>2^24或selector不在24-bit域返回对应错误。
pub const fn uniform_sample(count: u32, selector: u32) -> Result<Option<AliasSample>, Error> {
    if count > SELECTORS {
        return Err(Error::Capacity);
    }
    if selector >= SELECTORS {
        return Err(Error::Selector);
    }
    if count == 0 {
        return Ok(None);
    }
    let base = SELECTORS / count;
    let extra = SELECTORS % count;
    let boundary = extra * (base + 1);
    let index = if selector < boundary {
        selector / (base + 1)
    } else {
        extra + (selector - boundary) / base
    };
    let mass = base + if index < extra { 1 } else { 0 };
    Ok(Some(AliasSample {
        index,
        pmf: mass as f32 / SELECTORS as f32,
    }))
}

/// 无表近似均匀选择的实际逆PMF；空域或越界索引为0。
/// # Errors
/// count超过2^24返回容量错误；其余语义同uniform_sample。
pub const fn uniform_pmf(count: u32, index: u32) -> Result<f32, Error> {
    if count > SELECTORS {
        return Err(Error::Capacity);
    }
    if index >= count {
        return Ok(0.0);
    }
    let mass = SELECTORS / count + if index < SELECTORS % count { 1 } else { 0 };
    Ok(mass as f32 / SELECTORS as f32)
}

/// 将真实四角连续编码为三个 float4，供 shader `light_quad_vertices` 解码。
///
/// 只改变存储，不验证几何；调用方须另外通过 [`QuadArea::new`] 建立采样前提。
/// 记录为 `[a.xyz,b.x]`、`[b.yz,c.xy]`、`[c.z,d.xyz]`，不隐含平行四边形。
pub const fn pack_vertices([a, b, c, d]: [[f32; 3]; 4]) -> [[f32; 4]; 3] {
    [
        [a[0], a[1], a[2], b[0]],
        [b[1], b[2], c[0], c[1]],
        [c[2], d[0], d[1], d[2]],
    ]
}

/// 解码 [`pack_vertices`] 的三个连续记录，不重建或近似第四角。
///
/// 所有 f32 位模式原样移动；不验证几何，也不产生额外分配。
pub const fn unpack_vertices([p, q, r]: [[f32; 4]; 3]) -> [[f32; 3]; 4] {
    [
        [p[0], p[1], p[2]],
        [p[3], q[0], q[1]],
        [q[2], q[3], r[0]],
        [r[1], r[2], r[3]],
    ]
}

#[cfg(test)]
mod emitter_tests;
#[cfg(test)]
mod tests;

/// 输入、容量或构建分配失败；失败不产生部分有效的表。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 提案亮度、面积或功率不能建立有限的正支持。
    #[error("光源提案功率无效")]
    Power,
    /// 输入项数超过可为每个正权保留支持的 24-bit 上限。
    #[error("光源 alias 项数超过 2^24")]
    Capacity,
    /// 静态发光提案的尺寸、权重或覆盖率切片布局无效。
    #[error("静态发光提案输入无效")]
    Emission,
    /// 指定权重为负或不是有限数。
    #[error("光源 alias 权重 {0} 无效")]
    Weight(usize),
    /// 选择器不属于 `0..2^24`。
    #[error("光源选择器超过 24-bit 范围")]
    Selector,
    /// 坐标、两三角形或面积密度不满足 shader 的有限 f32 域。
    #[error("光源 quad 的几何或面积无效")]
    Quad,
    /// 输出或构建暂存无法申请，保留底层分配来源。
    #[error(transparent)]
    Allocation(#[from] std::collections::TryReserveError),
}
