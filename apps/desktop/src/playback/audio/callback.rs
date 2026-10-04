//! 声卡回调与环满背压(I1):样本入队、背压等待、cpal 回调填充三分支。
//!
//! 纪律:任何分支都必须写满输出缓冲(cpal 复用缓冲,不清零会放出陈旧样本);
//! 消费后 notify_one,唤醒环满背压等待的读线程;背压等待不计为停流。

use super::subprocess::lock_or_recover;
use super::{AudioShared, CHANNELS, PRIME_FRAMES};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Condvar, Mutex};

/// 入队一帧立体声样本(成对 push,保 ring 长度为偶数;前置条件:free() ≥ 2)
pub(super) fn push_pair(chunk: &[u8], st: &mut AudioShared) {
    let l = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
    let r = f32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
    st.ring.push(l).expect("背压等待后必有 ≥2 空位");
    st.ring.push(r).expect("背压等待后必有 ≥2 空位");
}

/// 背压等待:环内剩余不足一帧对(2 样本)时阻塞在 cond 上,直到声卡回调
/// pop 腾位或停机。返回 false = 停机(shutdown/seek/load 换流),调用方应
/// 立即退出读线程。等待期间置 `backpressured`,看门狗不得误判为停流。
pub(super) fn ensure_pair_space(
    shared: &Mutex<AudioShared>,
    cond: &Condvar,
    stopped: &AtomicBool,
) -> bool {
    let mut st = lock_or_recover(shared);
    while st.ring.free() < 2 {
        if stopped.load(Ordering::Relaxed) {
            return false;
        }
        st.backpressured = true;
        st = cond
            .wait(st)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
    }
    st.backpressured = false;
    true
}

/// cpal 回调主体(抽出便于脱离声卡单测):预缓冲/静音/欠载三分支。
pub(super) fn fill_output(
    data: &mut [f32],
    shared: &Mutex<AudioShared>,
    cond: &Condvar,
    consumed: &AtomicU64,
    muted: &AtomicBool,
    primed: &AtomicBool,
) {
    data.fill(0.0); // 静音基线:未填充分支天然输出静音
    let mut st = lock_or_recover(shared);
    if !primed.load(Ordering::Relaxed) && st.ring.len() >= (PRIME_FRAMES as usize) * CHANNELS {
        primed.store(true, Ordering::Relaxed);
    }
    let want_frames = data.len() / CHANNELS;
    if !primed.load(Ordering::Relaxed) {
        // 预缓冲未满:静音等待,不消费(不计欠载)
    } else if muted.load(Ordering::Relaxed) {
        // 静音:丢弃式消费 —— 主时钟照走,解除静音不回跳
        let n = want_frames.min(st.ring.len() / CHANNELS);
        for _ in 0..n * CHANNELS {
            st.ring.pop();
        }
        consumed.fetch_add(n as u64, Ordering::Relaxed);
        drop(st);
        cond.notify_one();
    } else if st.ring.len() >= want_frames * CHANNELS {
        for slot in data.as_chunks_mut::<CHANNELS>().0 {
            slot[0] = st.ring.pop().unwrap_or(0.0); // 成对 pop,必有值;防御不 panic
            slot[1] = st.ring.pop().unwrap_or(0.0);
        }
        consumed.fetch_add(want_frames as u64, Ordering::Relaxed);
        drop(st);
        cond.notify_one();
    } else {
        // 欠载:输出已清零的静音继续(残量保留凑整帧,不算消费)
        st.underruns += 1;
    }
}
