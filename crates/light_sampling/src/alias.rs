use crate::{Error, SELECTORS};

/// 一次离散选择的输入索引与实际量化概率。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AliasSample {
    /// 原始权重切片中的索引；零权项不会返回。
    pub index: u32,
    /// `mass / 2^24`，不是归一化理想权重。
    pub pmf: f32,
}

/// 独占不可变 GPU 表的离散分布；无借用、引用计数或每次查询分配。
#[derive(Debug, Default)]
pub struct AliasTable {
    entries: Vec<[u32; 4]>,
    count: u32,
    shift: u32,
}

impl AliasTable {
    /// 从原索引顺序的有限非负权重构建；空输入或全零输入返回空表。
    ///
    /// 输入最多 `2^24` 项；每个正权保留一个选择器，其余整数质量按累计权重分配。
    /// 先除以最大权重再求和，避免有限输入的总和溢出；极小权仍保留支持。
    /// 这是一种有偏差的离散 proposal，真实 PDF 必须从本表读取。
    ///
    /// 输出列数为输入长度向上取 2 的幂，填充项质量为零且永不被返回。
    /// 构建 O(输入项数+列数)，串行执行；除输入外峰值为每列 16B 输出和 4B 暂存。
    /// # Errors
    /// 负权、NaN/无穷、项数超限或分配失败返回对应错误。
    pub fn build(weights: &[f64]) -> Result<Self, Error> {
        if weights.len() > SELECTORS as usize {
            return Err(Error::Capacity);
        }
        let mut maximum = 0.0_f64;
        let mut positive = 0_u32;
        for (index, &weight) in weights.iter().enumerate() {
            if !weight.is_finite() || weight < 0.0 {
                return Err(Error::Weight(index));
            }
            maximum = maximum.max(weight);
            positive += u32::from(weight > 0.0);
        }
        if positive == 0 {
            return Ok(Self::default());
        }
        let columns = weights.len().next_power_of_two();
        let capacity = SELECTORS / columns as u32;
        let mut entries = Vec::new();
        entries.try_reserve_exact(columns)?;
        entries.resize(columns, [0; 4]);
        let mut scaled_total = 0.0;
        for &weight in weights {
            scaled_total += weight / maximum;
        }
        let available = SELECTORS - positive;
        let mut cumulative = 0.0;
        let mut previous = 0;
        for (index, &weight) in weights.iter().enumerate() {
            if weight > 0.0 {
                cumulative += weight / maximum;
                let target = ((cumulative / scaled_total * f64::from(available)).floor() as u32)
                    .min(available);
                let mass = 1 + target - previous;
                previous = target;
                entries[index] = [mass, index as u32, mass, 0];
            }
        }
        debug_assert_eq!(previous, available);
        let mut work = Vec::new();
        work.try_reserve_exact(columns)?;
        work.resize(columns, 0_u32);
        let mut small = 0;
        let mut large = columns;
        for (index, entry) in entries.iter_mut().enumerate() {
            entry[1] = index as u32;
            if entry[0] < capacity {
                work[small] = index as u32;
                small += 1;
            } else {
                large -= 1;
                work[large] = index as u32;
            }
        }
        while small != 0 && large != columns {
            small -= 1;
            let low = work[small] as usize;
            let high = work[large] as usize;
            large += 1;
            entries[low][1] = high as u32;
            entries[high][0] -= capacity - entries[low][0];
            if entries[high][0] < capacity {
                work[small] = high as u32;
                small += 1;
            } else {
                large -= 1;
                work[large] = high as u32;
            }
        }
        debug_assert_eq!(small, 0);
        Ok(Self {
            entries,
            count: weights.len() as u32,
            shift: capacity.trailing_zeros(),
        })
    }

    /// 原始索引域长度；空或全零输入为 0，不计填充列。
    pub const fn count(&self) -> u32 {
        self.count
    }

    /// 没有正权可供选择时为 true。
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// 每列拥有 `1 << shift` 个选择器；空表此值无意义，shader 须先检查 count。
    pub const fn column_shift(&self) -> u32 {
        self.shift
    }

    /// GPU 的连续 `uint4` 列：`[threshold, alias_index, original_mass, 0]`。
    ///
    /// `column=selector>>shift`，`local=selector & ((1<<shift)-1)`；
    /// `local<threshold` 选列本身，否则选 alias。PMF 从被选原索引的 mass 读取。
    /// 返回借用只在本表存活期间有效，调用方负责上传与 GPU 保活。
    pub fn entries(&self) -> &[[u32; 4]] {
        &self.entries
    }

    /// 用均匀的 `0..2^24` 整数选择一项；合法选择器在空表返回 None。
    /// # Errors
    /// 选择器超出 24-bit 域时返回 [`Error::Selector`]，即使表为空也拒绝。
    pub fn sample(&self, selector: u32) -> Result<Option<AliasSample>, Error> {
        if selector >= SELECTORS {
            return Err(Error::Selector);
        }
        if self.is_empty() {
            return Ok(None);
        }
        let column = selector >> self.shift;
        let local = selector & ((1 << self.shift) - 1);
        let entry = self.entries[column as usize];
        let index = if local < entry[0] { column } else { entry[1] };
        Ok(Some(AliasSample {
            index,
            pmf: self.pmf(index),
        }))
    }

    /// 读取原输入索引的实际 PMF；越界、空表或零权项返回 0。
    pub fn pmf(&self, index: u32) -> f32 {
        if index >= self.count {
            return 0.0;
        }
        self.entries[index as usize][2] as f32 / SELECTORS as f32
    }
}
