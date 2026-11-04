//! SipHash 的整数规范与标量证明；不包含执行期状态。

use vstd::prelude::*;

verus! {

pub type State = (u64, u64, u64, u64);

impl<const C: usize, const D: usize> crate::SipHash<C, D> {
    pub open(in super::super) spec fn state(&self) -> State { (self.v0, self.v1, self.v2, self.v3) }
}

pub open spec fn initial(seed: (u64, u64)) -> State {
    (0x736f6d6570736575u64 ^ seed.0, 0x646f72616e646f6du64 ^ seed.1,
     0x6c7967656e657261u64 ^ seed.0, 0x7465646279746573u64 ^ seed.1)
}

pub open spec fn rotate(x: u64, n: u32) -> u64 {
    (x << (n % 64)) | (x >> ((64 - n % 64) % 64))
}

pub open spec fn round(s: State) -> State {
    let a = s.0.wrapping_add(s.1);
    let b = rotate(s.1, 13) ^ a;
    let a = rotate(a, 32);
    let c = s.2.wrapping_add(s.3);
    let d = rotate(s.3, 16) ^ c;
    let a = a.wrapping_add(d);
    let d = rotate(d, 21) ^ a;
    let c = c.wrapping_add(b);
    let b = rotate(b, 17) ^ c;
    (a, b, rotate(c, 32), d)
}

pub open spec fn rounds(s: State, n: nat) -> State
    decreases n,
{
    if n == 0 { s } else { round(rounds(s, (n - 1) as nat)) }
}

pub open spec fn compress(s: State, m: u64, c: nat) -> State {
    let t = rounds((s.0, s.1, s.2, s.3 ^ m), c);
    (t.0 ^ m, t.1, t.2, t.3)
}

pub open spec fn finish(s: State, d: nat) -> u64 {
    let t = rounds((s.0, s.1, s.2 ^ 0xffu64, s.3), d);
    t.0 ^ t.1 ^ t.2 ^ t.3
}

/// 至多八字节的 little-endian 数值；递归仅存在于已擦除的规范。
pub open spec fn packed(msg: Seq<u8>, start: nat, count: nat) -> u64
    recommends count <= 8, start + count <= msg.len(),
    decreases count,
{
    if count == 0 { 0 } else {
        packed(msg, start, (count - 1) as nat)
            | ((msg[(start + count - 1) as int] as u64) << ((count - 1) * 8))
    }
}

pub open spec fn absorbed(s: State, msg: Seq<u8>, count: nat, c: nat) -> State
    recommends count * 8 <= msg.len(),
    decreases count,
{
    if count == 0 { s } else {
        compress(absorbed(s, msg, (count - 1) as nat, c),
            packed(msg, ((count - 1) * 8) as nat, 8), c)
    }
}

pub open spec fn partial_tail(msg: Seq<u8>, count: nat) -> u64
    recommends count <= msg.len() % 8,
{
    (((msg.len() % 256) as u64) << 56) | packed(msg, (msg.len() / 8 * 8) as nat, count)
}

pub open spec fn tail(msg: Seq<u8>) -> u64 {
    partial_tail(msg, msg.len() % 8)
}

pub open spec fn hash(s: State, msg: Seq<u8>, c: nat, d: nat) -> u64 {
    finish(compress(absorbed(s, msg, msg.len() / 8, c), tail(msg), c), d)
}

pub open spec fn le32(x: u32) -> Seq<u8> {
    Seq::new(4, |i: int| ((x >> (i * 8)) % 256u32) as u8)
}

pub open spec fn message(a: u32, b: u32, c: u32) -> Seq<u8> {
    le32(a) + le32(b) + le32(c)
}

pub open spec fn coords(s: State, a: u32, b: u32, c: u32, compress_rounds: nat, final_rounds: nat) -> u64 {
    hash(s, message(a, b, c), compress_rounds, final_rounds)
}

pub(crate) proof fn packed_step(msg: Seq<u8>, start: nat, count: nat)
    requires count < 8, start + count < msg.len(),
    ensures packed(msg, start, count + 1) == packed(msg, start, count)
        | ((msg[(start + count) as int] as u64) << (count * 8)),
{
    reveal(packed);
}

pub(crate) proof fn packed_first(msg: Seq<u8>, start: nat)
    requires start < msg.len(),
    ensures packed(msg, start, 1) == msg[start as int] as u64,
{
    let x = msg[start as int] as u64;
    assert(0u64 | (x << 0) == x) by(bit_vector);
    reveal_with_fuel(packed, 2);
}

pub open spec fn partitioned(msg: Seq<u8>, chunks: Seq<[u8; 8]>, remainder: Seq<u8>) -> bool {
    chunks.len() == msg.len() / 8
        && remainder == msg.subrange((msg.len() / 8 * 8) as int, msg.len() as int)
        && remainder.len() == msg.len() % 8
        && forall|i: int, j: int| 0 <= i < chunks.len() && 0 <= j < 8 ==>
            #[trigger] chunks[i]@[j] == msg[i * 8 + j]
}

pub(crate) proof fn chunk_word(msg: Seq<u8>, chunks: Seq<[u8; 8]>, remainder: Seq<u8>, index: nat)
    requires partitioned(msg, chunks, remainder), index < chunks.len(),
    ensures packed(chunks[index as int]@, 0, 8) == packed(msg, index * 8, 8),
{
    reveal_with_fuel(packed, 9);
}

pub open spec fn unprocessed(chunks: Seq<[u8; 8]>, pending: Seq<[u8; 8]>) -> bool {
    pending.len() <= chunks.len()
        && pending == chunks.subrange((chunks.len() - pending.len()) as int, chunks.len() as int)
}

pub(crate) proof fn pending_start(chunks: Seq<[u8; 8]>)
    ensures unprocessed(chunks, chunks),
{
    assert(chunks =~= chunks.subrange(0, chunks.len() as int));
}

pub(crate) proof fn pending_word(msg: Seq<u8>, chunks: Seq<[u8; 8]>, remainder: Seq<u8>,
    pending: Seq<[u8; 8]>, word: [u8; 8], rest: Seq<[u8; 8]>)
    requires partitioned(msg, chunks, remainder), unprocessed(chunks, pending),
        pending.len() > 0, word == pending[0], rest == pending.subrange(1, pending.len() as int),
    ensures unprocessed(chunks, rest), rest.len() + 1 == pending.len(),
        packed(word@, 0, 8) == packed(msg, (chunks.len() - pending.len()) as nat * 8, 8),
{
    let index = (chunks.len() - pending.len()) as nat;
    chunk_word(msg, chunks, remainder, index);
    assert(rest =~= chunks.subrange((chunks.len() - rest.len()) as int, chunks.len() as int));
}

pub(crate) proof fn tail_step(msg: Seq<u8>, chunks: Seq<[u8; 8]>, remainder: Seq<u8>, count: nat)
    requires partitioned(msg, chunks, remainder), count < remainder.len(),
    ensures partial_tail(msg, count + 1) == partial_tail(msg, count)
            | ((remainder[count as int] as u64) << (count * 8)),
{
    packed_step(msg, (msg.len() / 8 * 8) as nat, count);
    let tag = ((msg.len() % 256) as u64) << 56;
    let prefix = packed(msg, (msg.len() / 8 * 8) as nat, count);
    let byte = (remainder[count as int] as u64) << (count * 8);
    assert(tag | (prefix | byte) == (tag | prefix) | byte) by(bit_vector);
}

pub(crate) proof fn tail_start(msg: Seq<u8>)
    requires msg.len() <= usize::MAX,
    ensures ((msg.len() as u64) << 56) == partial_tail(msg, 0),
{
    length_byte(msg.len() as usize);
    let tag = ((msg.len() % 256) as u64) << 56;
    assert(tag | 0u64 == tag) by(bit_vector);
}

pub(crate) proof fn absorbed_step(s: State, msg: Seq<u8>, count: nat, c: nat)
    requires (count + 1) * 8 <= msg.len(),
    ensures absorbed(s, msg, count + 1, c)
        == compress(absorbed(s, msg, count, c), packed(msg, count * 8, 8), c),
{
    reveal(absorbed);
}

pub(crate) proof fn rounds_step(s: State, n: nat)
    ensures rounds(s, n + 1) == round(rounds(s, n)),
{
    reveal(rounds);
}

pub(crate) proof fn length_byte(len: usize)
    ensures ((len as u64) << 56) == (((len % 256) as u64) << 56),
{
    assert(((len as u64) << 56) == (((len % 256) as u64) << 56)) by(bit_vector);
}

/// rustdoc 的数学整数求和；与执行码的位 OR 编码分别定义。
pub open spec fn byte_sum(msg: Seq<u8>, start: nat, count: nat) -> nat
    recommends count <= 8, start + count <= msg.len(),
    decreases count,
{
    if count == 0 { 0 } else {
        byte_sum(msg, start, (count - 1) as nat)
            + msg[(start + count - 1) as int] as nat
                * vstd::arithmetic::power2::pow2(((count - 1) * 8) as nat)
    }
}

/// 字节字段互不重叠，因而 OR 精确等于整数求和；未占用的高位保持零。
pub(crate) proof fn packed_integer(msg: Seq<u8>, start: nat, count: nat)
    requires count <= 8, start + count <= msg.len(),
    ensures packed(msg, start, count) as nat == byte_sum(msg, start, count),
        count < 8 ==> packed(msg, start, count) >> (count * 8) == 0,
    decreases count,
{
    if count > 0 {
        let n = (count - 1) as nat;
        packed_integer(msg, start, n);
        let lower = packed(msg, start, n);
        let byte = msg[(start + n) as int];
        let shift = (n * 8) as u64;
        assert(shift <= 56 && lower >> shift == 0);
        assert((byte as u64) <= u64::MAX >> shift) by(bit_vector)
            requires shift <= 56;
        vstd::bits::lemma_u64_mul_pow2_le_max_iff_max_shr(byte as u64, shift, u64::MAX);
        vstd::bits::lemma_u64_shl_is_mul(byte as u64, shift);
        assert((lower | ((byte as u64) << shift)) as u128
            == (lower as u128) + (((byte as u64) << shift) as u128)) by(bit_vector)
            requires shift <= 56, lower >> shift == 0;
        if count < 8 {
            assert((lower | ((byte as u64) << shift)) >> (shift + 8) == 0) by(bit_vector)
                requires shift < 56, lower >> shift == 0;
        }
    } else {
        assert(0u64 >> 0 == 0) by(bit_vector);
    }
}

pub(crate) proof fn tail_integer(msg: Seq<u8>)
    ensures tail(msg) as nat == (msg.len() % 256) * 0x0100000000000000nat
        + byte_sum(msg, (msg.len() / 8 * 8) as nat, msg.len() % 8),
{
    let start = (msg.len() / 8 * 8) as nat;
    let count = msg.len() % 8;
    packed_integer(msg, start, count);
    let low = packed(msg, start, count);
    let shift = (count * 8) as u64;
    let length = (msg.len() % 256) as u64;
    assert(low >> 56 == 0) by(bit_vector)
        requires shift <= 56, low >> shift == 0;
    assert(((length << 56) | low) as u128
        == ((length as u128) << 56) + (low as u128)) by(bit_vector)
        requires length <= 255, low >> 56 == 0;
    assert(length * 0x0100000000000000nat <= u128::MAX) by(nonlinear_arith)
        requires length <= 255;
    vstd::arithmetic::power2::lemma2_to64_rest();
    vstd::bits::lemma_u128_shl_is_mul(length as u128, 56);
}

pub(crate) proof fn coords_words(s: State, a: u32, b: u32, c: u32, cr: nat, dr: nat)
    ensures coords(s, a, b, c, cr, dr) == finish(compress(
        compress(s, (a as u64) | ((b as u64) << 32), cr),
        (12u64 << 56) | (c as u64), cr), dr),
{
    broadcast use vstd::seq_lib::group_seq_properties;
    let msg = message(a, b, c);
    reveal_with_fuel(packed, 9);
    assert((0u64 | ((((a >> 0) % 256) as u8 as u64) << 0)
        | ((((a >> 8) % 256) as u8 as u64) << 8)
        | ((((a >> 16) % 256) as u8 as u64) << 16)
        | ((((a >> 24) % 256) as u8 as u64) << 24)
        | ((((b >> 0) % 256) as u8 as u64) << 32)
        | ((((b >> 8) % 256) as u8 as u64) << 40)
        | ((((b >> 16) % 256) as u8 as u64) << 48)
        | ((((b >> 24) % 256) as u8 as u64) << 56))
        == (a as u64) | ((b as u64) << 32)) by(bit_vector);
    assert((0u64 | ((((c >> 0) % 256) as u8 as u64) << 0)
        | ((((c >> 8) % 256) as u8 as u64) << 8)
        | ((((c >> 16) % 256) as u8 as u64) << 16)
        | ((((c >> 24) % 256) as u8 as u64) << 24)) == c as u64) by(bit_vector);
    reveal_with_fuel(absorbed, 2);
}

spec fn state_xor(s: State) -> u64 { s.0 ^ s.1 ^ s.2 ^ s.3 }

proof fn zero_compress(s: State, m: u64)
    ensures state_xor(compress(s, m, 0)) == state_xor(s),
{
    let (a, b, c, d) = s;
    assert((a ^ m) ^ b ^ c ^ (d ^ m) == a ^ b ^ c ^ d) by(bit_vector);
}

proof fn zero_absorbed(s: State, msg: Seq<u8>, n: nat)
    requires n * 8 <= msg.len(),
    ensures state_xor(absorbed(s, msg, n, 0)) == state_xor(s),
    decreases n,
{
    if n > 0 {
        zero_absorbed(s, msg, (n - 1) as nat);
        zero_compress(absorbed(s, msg, (n - 1) as nat, 0), packed(msg, ((n - 1) * 8) as nat, 8));
    }
}

/// 零压缩轮与零结束轮使所有消息字及密钥字在最终 XOR 中抵消。
pub(crate) proof fn zero_rounds(seed: (u64, u64), msg: Seq<u8>)
    ensures hash(initial(seed), msg, 0, 0) == 0x0f1c1c0309061df5u64,
{
    let s = initial(seed);
    zero_absorbed(s, msg, msg.len() / 8);
    zero_compress(absorbed(s, msg, msg.len() / 8, 0), tail(msg));
    let t = compress(absorbed(s, msg, msg.len() / 8, 0), tail(msg), 0);
    let (a, b, c, d) = t;
    assert(a ^ b ^ (c ^ 0xffu64) ^ d == (a ^ b ^ c ^ d) ^ 0xffu64) by(bit_vector);
    let (k0, k1) = seed;
    assert((0x736f6d6570736575u64 ^ k0) ^ (0x646f72616e646f6du64 ^ k1)
        ^ (0x6c7967656e657261u64 ^ k0) ^ (0x7465646279746573u64 ^ k1)
        ^ 0xffu64 == 0x0f1c1c0309061df5u64) by(bit_vector);
}

}
