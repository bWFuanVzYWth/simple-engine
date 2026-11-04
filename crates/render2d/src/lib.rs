//! 有序 2D 场景、Unicode 文字与可选 wgpu 渲染器。
//!
//! [`DrawList`] 保存连续几何数据和有序操作流；仅相邻兼容批次可合并，透明叠加顺序不变。
//! [`Canvas`] 用借用限定子裁剪范围，无需配对 push/pop。坐标以逻辑像素计，颜色为
//! linear sRGB、premultiplied alpha；HDR 和目标输出变换由应用决定。
//!
//! 默认仅提供 CPU 场景与 [`plot`]。`text` 启用固定字体集排版和独占正文的编辑模型，
//! `gpu` 独立启用 Slang/wgpu 编码，文字绘制须同时启用 `text`；本 crate 不拥有窗口、
//! 呈现循环或应用模型。
//! CPU 场景消费者应在输入边界调用 [`DrawList::validate`]，GPU 准备会执行同样校验。
//!
//! ```
//! use render2d::{Color, DrawList, Rect};
//!
//! let mut draw = DrawList::new();
//! let bounds = Rect::new(0.0, 0.0, 320.0, 180.0);
//! draw.canvas(bounds).rounded_rect(bounds.inset(8.0), 6.0, Color::WHITE);
//! draw.validate()?;
//! # Ok::<(), render2d::SceneError>(())
//! ```
#![forbid(unsafe_code)]
#![recursion_limit = "256"]

mod scene;
pub use scene::*;
pub mod plot;

#[cfg(feature = "text")]
pub mod text;

#[cfg(feature = "gpu")]
pub mod gpu;
