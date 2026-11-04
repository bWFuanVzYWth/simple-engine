//! 批量直方图分箱与有序图表几何；坐标和颜色沿用 [`Canvas`]。
//!
//! 绘制函数不分配图表状态，不检查所有样本；调用者须提供有效矩形、颜色及有限非负值。

use crate::{Canvas, Color, Rect};

/// 固定分箱和数值范围，重复替换样本时复用计数数组。
pub struct Histogram {
    bins: Vec<usize>,
    minimum: f32,
    maximum: f32,
}

/// 直方图参数校验失败。
#[derive(Debug, thiserror::Error)]
pub enum PlotError {
    /// 分箱为空，范围非有限、不递增或跨度溢出。
    #[error("直方图需要非空分箱与有限递增范围")]
    InvalidRange,
}

impl Histogram {
    /// 创建等宽分箱；范围包含两端，上端点归入末箱。
    pub fn new(bins: usize, minimum: f32, maximum: f32) -> Result<Self, PlotError> {
        if bins == 0
            || !minimum.is_finite()
            || !maximum.is_finite()
            || minimum >= maximum
            || !(maximum - minimum).is_finite()
        {
            return Err(PlotError::InvalidRange);
        }
        Ok(Self {
            bins: vec![0; bins],
            minimum,
            maximum,
        })
    }

    /// 非有限／范围外样本不计数；maximum 属于末箱。
    pub fn replace(&mut self, samples: &[f32]) {
        self.bins.fill(0);
        let scale = self.bins.len() as f64 / (f64::from(self.maximum) - f64::from(self.minimum));
        for &sample in samples {
            if sample.is_finite() && sample >= self.minimum && sample <= self.maximum {
                let bin = ((f64::from(sample) - f64::from(self.minimum)) * scale) as usize;
                let bin = bin.min(self.bins.len() - 1);
                self.bins[bin] += 1;
            }
        }
    }

    /// 按数值递增顺序读取各箱计数。
    pub fn bins(&self) -> &[usize] {
        &self.bins
    }
}

/// 最大箱填满高度，柱宽占箱宽的 85%；空计数不绘制，全部图形裁剪到 bounds。
pub fn histogram(canvas: &mut Canvas<'_>, bounds: Rect, bins: &[usize], color: Color) {
    let maximum = bins.iter().copied().max().unwrap_or(0);
    if maximum == 0 || bins.is_empty() {
        return;
    }
    let mut canvas = canvas.clipped(bounds);
    let width = bounds.width / bins.len() as f32;
    for (i, &count) in bins.iter().enumerate() {
        let height = bounds.height * count as f32 / maximum as f32;
        canvas.fill_rect(
            Rect::new(
                bounds.x + i as f32 * width,
                bounds.bottom() - height,
                width * 0.85,
                height,
            ),
            color,
        );
    }
}

/// 按样本顺序绘制自下而上的堆叠柱，maximum 对应 bounds 全高，超出部分裁剪。
///
/// 值须有限且非负，maximum 须有限且为正，柱宽为样本间距的 85%。
/// 用于耗时图时，各段应属于同一时间域且互不重叠，不将异步 GPU 时间与 CPU 时间相加。
pub fn stacked_bars<const N: usize>(
    canvas: &mut Canvas<'_>,
    bounds: Rect,
    samples: &[[f32; N]],
    colors: &[Color; N],
    maximum: f32,
) {
    if samples.is_empty() || maximum <= 0.0 {
        return;
    }
    let mut canvas = canvas.clipped(bounds);
    let width = bounds.width / samples.len() as f32;
    for (i, sample) in samples.iter().enumerate() {
        let mut y = bounds.bottom();
        for (&value, &color) in sample.iter().zip(colors) {
            let height = value / maximum * bounds.height;
            y -= height;
            canvas.fill_rect(
                Rect::new(bounds.x + i as f32 * width, y, width * 0.85, height),
                color,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_keeps_represented_sample_count() {
        let mut histogram = Histogram::new(4, 0.0, 4.0).unwrap();
        histogram.replace(&[0.0, 0.5, 1.0, 2.0, 4.0]);
        assert_eq!(histogram.bins(), [2, 1, 1, 1]);
    }

    #[test]
    fn stacked_segments_share_one_baseline() {
        let mut list = crate::DrawList::new();
        let mut canvas = Canvas::new(&mut list);
        stacked_bars(
            &mut canvas,
            Rect::new(0.0, 0.0, 10.0, 100.0),
            &[[2.0, 3.0]],
            &[Color::WHITE; 2],
            10.0,
        );
        let bars = list.shapes();
        assert!((bars[0].rect.bottom() - 100.0).abs() < 1e-4);
        assert!((bars[1].rect.bottom() - bars[0].rect.y).abs() < 1e-4);
    }
}
