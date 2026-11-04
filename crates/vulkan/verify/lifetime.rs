//! Vulkan 生命周期协议的独立 Verus 模型，不是生产实现的证明。
//!
//! 对应 `owner::Arena` 的单槽换代和 `native::Context` 的录制、提交、完成、退役边界。
//! 多个消费者用 pending/external 计数及同队列最大提交序号表示；不建模依赖图、
//! 分配失败、FFI、Drop、设备丢失或跨队列同步。匿名计数也不验证命令票据身份、
//! 重复取消/完成、批次顺序、retirement 等待快照或重新发布。
//! `completed` 与 `external_completed` 是宿主完成事件；模型不能证明 GPU 确已完成。
//! 验证调用者必须另行证明这些事件与真实 wait/callback 对应，且登记所有消费者。
//!
//! 模型不使用 `assume` 或 `external_body`。证明只保证遵守前置条件的显式状态转换：
//! 发布或仍有消费者时不能释放；释放换代后旧标识失效；generation 用尽后不复用。
//! Rust 的所有权不保证析构一定运行，本模型也不证明最终释放或无泄漏。
//!
//! 运行入口：`scripts/verify_vulkan.ps1`；协议来源：
//! <https://docs.vulkan.org/refpages/latest/refpages/source/vkDestroyBuffer.html>、
//! <https://docs.vulkan.org/refpages/latest/refpages/source/vkQueueWaitIdle.html>。

use vstd::prelude::*;

verus! {

#[derive(Clone, Copy)]
struct Key {
    owner: u64,
    generation: u32,
}

struct Lifetime {
    owner: u64,
    generation: u32,
    occupied: bool,
    exhausted: bool,
    published: bool,
    pending: u32,
    external: u32,
    issued: u64,
    completed: u64,
    last_use: u64,
}

impl Lifetime {
    spec fn valid(&self) -> bool {
        self.completed <= self.issued
            && self.last_use <= self.issued
            && (self.exhausted ==> !self.occupied)
            && (!self.occupied ==> !self.published && self.pending == 0
                && self.external == 0 && self.last_use == 0)
    }

    spec fn accepts(&self, key: Key) -> bool {
        self.occupied && key.owner == self.owner && key.generation == self.generation
    }

    fn new(owner: u64) -> (state: Self)
        ensures
            state.valid(),
            state.owner == owner,
            state.generation == 0,
            state.occupied && state.published && !state.exhausted,
            state.pending == 0 && state.external == 0,
            state.issued == 0 && state.completed == 0 && state.last_use == 0,
    {
        Self { owner, generation: 0, occupied: true, exhausted: false,
            published: true, pending: 0, external: 0, issued: 0, completed: 0, last_use: 0 }
    }

    fn key(&self) -> (key: Key)
        requires self.occupied,
        ensures self.accepts(key),
    {
        Key { owner: self.owner, generation: self.generation }
    }

    fn record(&mut self, key: Key)
        requires old(self).valid(), old(self).accepts(key),
            old(self).published, old(self).pending < u32::MAX,
        ensures final(self).valid(), *final(self) == (Self { pending: (old(self).pending + 1) as u32, ..*old(self) }),
    {
        self.pending = self.pending + 1;
    }

    fn retire(&mut self)
        requires old(self).valid(), old(self).occupied,
        ensures final(self).valid(), *final(self) == (Self { published: false, ..*old(self) }),
    {
        self.published = false;
    }

    fn cancel(&mut self)
        requires old(self).valid(), old(self).pending > 0,
        ensures final(self).valid(), *final(self) == (Self { pending: (old(self).pending - 1) as u32, ..*old(self) }),
    {
        self.pending = self.pending - 1;
    }

    fn submit(&mut self)
        requires old(self).valid(), old(self).pending > 0, old(self).issued < u64::MAX,
        ensures final(self).valid(), *final(self) == (Self {
            pending: (old(self).pending - 1) as u32,
            issued: (old(self).issued + 1) as u64,
            last_use: (old(self).issued + 1) as u64,
            ..*old(self)
        }),
    {
        self.pending = self.pending - 1;
        self.issued = self.issued + 1;
        self.last_use = self.issued;
    }

    fn encode_external(&mut self)
        requires old(self).valid(), old(self).pending > 0, old(self).external < u32::MAX,
        ensures final(self).valid(), *final(self) == (Self {
            pending: (old(self).pending - 1) as u32,
            external: (old(self).external + 1) as u32, ..*old(self)
        }),
    {
        self.pending = self.pending - 1;
        self.external = self.external + 1;
    }

    // 输入事件：宿主已确认同一队列此序号及以前的提交真实完成。
    fn completed(&mut self, serial: u64)
        requires old(self).valid(), old(self).completed <= serial <= old(self).issued,
        ensures final(self).valid(), *final(self) == (Self { completed: serial, ..*old(self) }),
    {
        self.completed = serial;
    }

    // 输入事件：一个外部批次已完成，或已保证以后绝不提交。
    fn external_completed(&mut self)
        requires old(self).valid(), old(self).external > 0,
        ensures final(self).valid(), *final(self) == (Self { external: (old(self).external - 1) as u32, ..*old(self) }),
    {
        self.external = self.external - 1;
    }

    fn release(&mut self)
        requires old(self).valid(), old(self).occupied, !old(self).published,
            old(self).pending == 0, old(self).external == 0,
            old(self).last_use <= old(self).completed,
        ensures
            final(self).valid(), !final(self).occupied, final(self).owner == old(self).owner,
            final(self).issued == old(self).issued, final(self).completed == old(self).completed,
            old(self).generation < u32::MAX ==> final(self).generation == old(self).generation + 1
                && !final(self).exhausted,
            old(self).generation == u32::MAX ==> final(self).exhausted,
    {
        self.occupied = false;
        self.last_use = 0;
        if self.generation < u32::MAX {
            self.generation = self.generation + 1;
        } else {
            self.exhausted = true;
        }
    }

    fn reuse(&mut self)
        requires old(self).valid(), !old(self).occupied, !old(self).exhausted,
        ensures final(self).valid(), *final(self) == (Self { occupied: true, published: true, ..*old(self) }),
    {
        self.occupied = true;
        self.published = true;
    }

    fn access(&self, key: Key)
        requires self.accepts(key),
    {}
}

// 对任意可释放代次证明复用后旧标识仍失效，而非只检查一次具体换代。
fn reuse_rejects_old_key(mut state: Lifetime, key: Key)
    requires state.valid(), state.accepts(key), !state.published,
        state.pending == 0, state.external == 0, state.last_use <= state.completed,
        state.generation < u32::MAX,
{
    state.release();
    state.reuse();
    assert(!state.accepts(key));
}

fn generation_exhaustion(mut state: Lifetime)
    requires state.valid(), state.occupied, !state.published, state.generation == u32::MAX,
        state.pending == 0, state.external == 0, state.last_use <= state.completed,
{
    state.release();
    assert(state.exhausted && !state.occupied);
}

fn foreign_owner(state: Lifetime, key: Key)
    requires key.owner != state.owner,
{
    assert(!state.accepts(key));
}

fn multiple_consumers() {
    let mut state = Lifetime::new(7);
    let key = state.key();
    state.record(key);
    state.record(key);
    state.record(key);
    state.retire();
    state.cancel();
    state.submit();
    state.encode_external();
    state.completed(1);
    assert(state.external > 0);
    state.external_completed();
    state.release();
    state.reuse();
    assert(!state.accepts(key));
}

#[cfg(reject_early_release)]
fn reject_early_release() {
    let mut state = Lifetime::new(7);
    let key = state.key();
    state.record(key);
    state.submit();
    state.retire();
    state.release();
}

#[cfg(reject_pending_release)]
fn reject_pending_release() {
    let mut state = Lifetime::new(7);
    let key = state.key();
    state.record(key);
    state.retire();
    state.release();
}

#[cfg(reject_stale_key)]
fn reject_stale_key() {
    let mut state = Lifetime::new(7);
    let key = state.key();
    state.retire();
    state.release();
    state.reuse();
    state.access(key);
}

#[cfg(reject_external_release)]
fn reject_external_release() {
    let mut state = Lifetime::new(7);
    let key = state.key();
    state.record(key);
    state.encode_external();
    state.retire();
    state.release();
}

fn main() {
    multiple_consumers();
}

}
