//! 显式循环使用的整数时钟、完整值交接与帧间隔统计；不创建线程或决定应用阶段。
//!
//! [`SessionClock`] 提供同一会话的纳秒域，调用方可转成自己的输入时间类型。
//! [`TickClock`] 按绝对 epoch 与序号求期限，迟到时也不暗中跳 tick 或移动 epoch。
//! [`latest`] 交换单个 owned 值，只覆盖尚未取走的中间值；它不是有序事件队列。
//! 需要保留的按键边沿、未确认操作与业务 ack 由应用在完整包中维护。
//! [`timing`] 维护完整间隔窗口、分位数与整帧保峰；阶段名称、计时和绘制属于应用。

#![forbid(unsafe_code)]

mod clock;
mod latest;
pub mod timing;

pub use clock::{ClockError, SessionClock, TickClock};
pub use latest::{Consumer, Latest, PublishError, Publisher, Wait, latest};
