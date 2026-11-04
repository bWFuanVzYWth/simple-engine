//! 带种子的 SipHash-64；标量消息与八路独立消息使用相同 little-endian 编码。
//!
//! [`SipHash`] 只保存四个初始化状态字；每次调用使用局部状态，不分配、不修改实例，
//! 也不调度线程。`C` 为每消息块压缩轮数，`D` 为结束轮数；零轮也有定义。
//! [`SipHash::hash_u32x3`] 等价于三个 `u32::to_le_bytes()` 的 12 字节拼接。
//! 算法来源为作者的 [SipHash 参考实现](https://github.com/veorq/SipHash/blob/32d067603b93b47828700880649198e0bfbbcffa/siphash.c)
//! 的 64-bit 输出分支；密钥字节为 `seed.0.to_le_bytes() || seed.1.to_le_bytes()`，
//! 返回整数的 `to_le_bytes()` 对应参考输出。`||` 表示字节拼接。
//!
//! # 算法规范
//!
//! 以下定义与执行步骤的存储形式无关；状态 `S = (v0, v1, v2, v3)` 为四个 64-bit 字。
//! `+` 按模 `2^64` 相加，`^`、`|` 为逐位 XOR、OR，`rol(x, n)` 为 64-bit 循环左移。
//! 初始化 `I(k0, k1)` 为：
//!
//! ```text
//! (0x736f6d6570736575 ^ k0, 0x646f72616e646f6d ^ k1,
//!  0x6c7967656e657261 ^ k0, 0x7465646279746573 ^ k1)
//! ```
//!
//! 单轮变换 `R(S)` 的各赋值依次执行，右侧使用最新状态：
//!
//! ```text
//! v0 = v0 + v1; v1 = rol(v1, 13) ^ v0; v0 = rol(v0, 32)
//! v2 = v2 + v3; v3 = rol(v3, 16) ^ v2
//! v0 = v0 + v3; v3 = rol(v3, 21) ^ v0
//! v2 = v2 + v1; v1 = rol(v1, 17) ^ v2; v2 = rol(v2, 32)
//! ```
//!
//! `R^n` 表示恰好应用 `n` 次 `R`，`R^0(S) = S`。
//! `Xj(S, m)` 只将状态第 `j` 个分量与 `m` XOR，分量从 0 编号；
//! `xor(S)` 为四个分量的 XOR。压缩和结束定义为：
//!
//! ```text
//! B_C(S, m) = X0(R^C(X3(S, m)), m)
//! F_D(S) = xor(R^D(X2(S, 0xff)))
//! ```
//!
//! 对任意有效字节切片 `M`，令 `L = len(M)`、`q = L / 8` 向下取整、`r = L % 8`。
//! 完整块 `m_i = sum(M[8*i+j] * 2^(8*j), 0 <= j < 8)`，其中 `0 <= i < q`。
//! 必须另行压缩一个尾块，即使 `r = 0` 或 `L = 0`：
//!
//! ```text
//! b = ((L mod 256) * 2^56) | sum(M[8*q+j] * 2^(8*j), 0 <= j < r)
//! S_0 = I(k0, k1)
//! S_(i+1) = B_C(S_i, m_i), 0 <= i < q
//! H_C,D(k0, k1, M) = F_D(B_C(S_q, b))
//! ```
//!
//! 求和在数学整数上定义，空和为 0；尾字节仅占低 56 位，与长度字节不重叠。
//! 长度模 256 只影响尾块最高字节，所有完整块仍参与计算，消息不截断为 255 字节。
//!
//! # 形式化验证
//!
//! `scripts/verify_sip_hash.ps1` 直接装配生产源码，验证下列性质及违规反例；
//! `perlin_noise` 实际调用的 `SipHash<1, 3>::hash_u32x3_simd` 包含在同源范围内。
//!
//! | 范围 | 性质 | 排除的缺陷 |
//! |---|---|---|
//! | 初始化与轮核 | `new = I`，标量单轮与多轮分别等于 `R`、`R^n` | 密钥字错位、回绕/旋转错误、轮数差一 |
//! | 固定消息 | [`SipHash::hash_u32x3`] 等于上述 `H` 的 12 字节实例 | 坐标字序、零扩展、长度和结束标记错误 |
//! | SIMD | 两个八路入口逐 lane 等于同 seed、同轮数的标量三坐标入口 | lane 串扰/重排、SIMD 算术或编码偏差 |
//! | 通用消息 | [`SipHash::hash`] 对任意有效切片等于上述 `H` | 漏块、尾部越界、整块后漏尾块、长度回绕错误 |
//!
//! 所有性质均量化任意 seed、全部 `u32` 位型及全部 `usize` 轮数 `C, D`，包括零轮；
//! 单轮与多轮引理覆盖任意四字状态，SIMD 轮核覆盖任意八路四字状态。
//! `(1, 3)`、`(2, 4)` 的有限实例不能代替通用结论。每项同时要求正常返回、无越界、
//! 无整数溢出 panic 或非法移位，公开入口不引入新的前置条件；不承诺可接受的执行时长。
//! 执行函数显式声明 `no_unwind` 并检查全部调用；下降量、无递归调用图和标准原语的有限返回
//! 共同保证终止，有限返回与无 unwind 均属于下述标准库可信边界。
//! 零轮是精确定义的算法边界：`C = D = 0` 时结果恒为 `0x0f1c1c0309061df5`，
//! 与 seed、消息无关。lane 等价已蕴含 lane 独立与同步置换输入/输出的性质，无需重复立项。
//!
//! 数学规范及辅助证明集中在 `src/proof/`；已证明位 OR 拼装等于上述整数求和，
//! 尾字节不与长度字节重叠。`verify/hash.rs` 检查公开调用与规范连接，
//! 没有替换 SIMD 执行代码；同源证明使用固定 Verus 的 `--no-cheating --rlimit 10`。
//!
//! 信任 Rust/Verus 工具链、vstd 与标准库契约。固定 Verus 缺少的标准原语规范集中在
//! `verify/std.rs` 并单独编译：[`slice::as_chunks`]、[`u64::rotate_left`]、SIMD 数组转换、
//! splat、`u32` 到 `u64` 零扩展、逐 lane 位运算/合法移位及模 `2^64` 加法。
//! 运算适配只封装一次标准运算，native 与可信库共用 `src/ops.rs`，包含其 `no_unwind` 契约。
//! SIMD 运算依据 [`std::simd::Simd`] 与 [`std::simd::num::SimdUint::cast`]；
//! 可信适配不包含 round 或 hash，编译成功不计为证明通过。这些外部原语及编译器降级
//! 本身不在证明范围内，普通 Cargo 构建也不运行证明器或引入可信适配库。
//!
//! 实例不变、无分配、无隐藏调度及常量大小临时状态继续由所有权、代码审查和性能检查维护，
//! 不单独建立低收益证明。现有官方向量只覆盖 SipHash-2-4 的长度 `0 <= L < 64`，
//! 随机 SIMD 对照只覆盖 `(1, 3)`、`(2, 4)`；两者均不能代替证明。
//! 不承诺无碰撞、不同 seed/轮数必得不同结果、输出均匀性、密码学安全或恒定时间；
//! 特定样例上的不等断言也不能推广为这些性质。
//!
//! ```
//! use sip_hash::SipHash;
//! let hash = SipHash::<1, 3>::new((7, 11));
//! let message = [1_u32.to_le_bytes(), 2_u32.to_le_bytes(), 3_u32.to_le_bytes()].concat();
//! assert_eq!(hash.hash_u32x3(1, 2, 3), hash.hash(&message));
//! ```

#![cfg_attr(not(verus_only), feature(portable_simd))]
#![forbid(unsafe_code)]

#[cfg(verus_only)]
use vstd::prelude::*;

#[cfg(not(verus_only))]
mod ops;
pub(crate) mod scalar;
pub(crate) mod simd;

/// 四字初始化状态；共享借用可并发计算独立消息。
#[derive(Clone, Copy)]
#[cfg_attr(verus_only, verus_verify)]
pub struct SipHash<const C: usize, const D: usize> {
    v0: u64,
    v1: u64,
    v2: u64,
    v3: u64,
}

#[cfg_attr(verus_only, verus_verify)]
impl<const C: usize, const D: usize> SipHash<C, D> {
    /// 用两个密钥字建立可复用状态；不分配。
    ///
    /// 对任意 seed，结果状态等于模块规范的 `I(seed.0, seed.1)`；`C, D` 不影响初始化。
    #[must_use]
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result.state() == scalar::proof::initial(seed),
        no_unwind
    ))]
    pub const fn new(seed: (u64, u64)) -> Self {
        Self {
            v0: 0x736f_6d65_7073_6575 ^ seed.0,
            v1: 0x646f_7261_6e64_6f6d ^ seed.1,
            v2: 0x6c79_6765_6e65_7261 ^ seed.0,
            v3: 0x7465_6462_7974_6573 ^ seed.1,
        }
    }
}
