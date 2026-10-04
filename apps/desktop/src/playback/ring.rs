//! 有界环形缓冲(I1 播放引擎):容量固定,**满不弃旧**。
//!
//! 背压纪律(修复"钟走帧停"):队满时 `push` 拒绝并退还原值,生产者(读线程)
//! 在调用方的 condvar 上等待,直到消费者 `pop` 腾出空间并 notify——解码速度
//! 由此被拉平到消费速度,头部帧永不丢失。跨线程用法:Mutex + Condvar 组合,
//! 见 decoder.rs / audio.rs。

use std::collections::VecDeque;

pub(crate) struct Ring<T> {
    q: VecDeque<T>,
    cap: usize,
}

impl<T> Ring<T> {
    pub(crate) fn new(cap: usize) -> Self {
        assert!(cap > 0, "环形缓冲容量必须 > 0");
        Self {
            q: VecDeque::with_capacity(cap),
            cap,
        }
    }

    /// 入队;队满返回 `Err(v)`(不静默丢弃,由调用方背压重试)
    pub(crate) fn push(&mut self, v: T) -> Result<(), T> {
        if self.q.len() >= self.cap {
            return Err(v);
        }
        self.q.push_back(v);
        Ok(())
    }

    pub(crate) fn pop(&mut self) -> Option<T> {
        self.q.pop_front()
    }

    pub(crate) fn len(&self) -> usize {
        self.q.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.q.is_empty()
    }

    /// 是否已满(生产端应背压等待)
    pub(crate) fn is_full(&self) -> bool {
        self.q.len() >= self.cap
    }

    /// 剩余空位(音频按帧对入队:剩余 <2 时等待,保证成对 push)
    pub(crate) fn free(&self) -> usize {
        self.cap - self.q.len()
    }
}

#[cfg(test)]
mod tests {
    use super::Ring;

    #[test]
    fn fifo_顺序与容量上限() {
        let mut r = Ring::new(3);
        for i in 0..3 {
            assert_eq!(r.push(i), Ok(()), "未满应可入队");
        }
        assert_eq!(r.len(), 3);
        assert!(r.is_full());
        assert_eq!(r.free(), 0);
        assert_eq!(r.pop(), Some(0));
        assert_eq!(r.pop(), Some(1));
        assert_eq!(r.free(), 2);
        r.push(9).expect("有空位");
        assert_eq!(r.pop(), Some(2), "先入先出");
        assert_eq!(r.pop(), Some(9));
        assert!(r.is_empty());
    }

    #[test]
    fn 队满拒绝不弃旧() {
        let mut r = Ring::new(2);
        r.push('a').expect("空队");
        r.push('b').expect("第二位");
        // 满:拒绝并退还原值,队内容不变(头部 a 不丢)
        assert_eq!(r.push('c'), Err('c'));
        assert_eq!(r.len(), 2);
        assert!(r.is_full());
        assert_eq!(r.pop(), Some('a'), "头部帧必须保留");
        assert_eq!(r.push('c'), Ok(()));
        assert_eq!(r.pop(), Some('b'));
        assert_eq!(r.pop(), Some('c'));
    }
}
