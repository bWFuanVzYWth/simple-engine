use crate::{MAX_COORD, NoiseError, PerlinNoise};

impl PerlinNoise {
    /// 按输入顺序采样，使用调用方提供的输出；空批合法。
    ///
    /// 整个输入校验成功后才修改输出。尾部补齐为坐标零，仍走八路内核，只复制有效 lane。
    /// 准备与采样都在调用线程执行，不分配。
    ///
    /// # Errors
    /// 四个切片长度不同，或任意坐标不是 finite、超出 `[-MAX_COORD, MAX_COORD]` 时失败。
    pub fn sample_3d_batch(
        &self,
        x: &[f32],
        y: &[f32],
        z: &[f32],
        output: &mut [f32],
    ) -> Result<(), NoiseError> {
        if x.len() != y.len() || x.len() != z.len() || x.len() != output.len() {
            return Err(NoiseError::LengthMismatch);
        }
        for (axis, coords) in [x, y, z].into_iter().enumerate() {
            if let Some(index) = coords
                .iter()
                .position(|coord| !coord.is_finite() || !(-MAX_COORD..=MAX_COORD).contains(coord))
            {
                return Err(NoiseError::InvalidCoordinate { axis, index });
            }
        }
        let mut offset = 0;
        while output.len() - offset >= 8 {
            let values = self.noise_3d_x8(
                std::array::from_fn(|lane| x[offset + lane]),
                std::array::from_fn(|lane| y[offset + lane]),
                std::array::from_fn(|lane| z[offset + lane]),
            );
            output[offset..offset + 8].copy_from_slice(&values);
            offset += 8;
        }
        let remaining = output.len() - offset;
        if remaining != 0 {
            let mut xs = [0.0; 8];
            let mut ys = [0.0; 8];
            let mut zs = [0.0; 8];
            xs[..remaining].copy_from_slice(&x[offset..]);
            ys[..remaining].copy_from_slice(&y[offset..]);
            zs[..remaining].copy_from_slice(&z[offset..]);
            let values = self.noise_3d_x8(xs, ys, zs);
            output[offset..].copy_from_slice(&values[..remaining]);
        }
        Ok(())
    }
}
