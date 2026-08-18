//! 内存回收 — 移植自 openless `asr/local/cache.rs` 的 `pressure_relief_macos`：
//! macOS libmalloc 的 freelist 不会主动把物理页归还内核，free 过的内存只进
//! freelist 待复用，RSS / 活动监视器只升不降（原版注释：看起来"释放没生效"）。
//!
//! 每次听写会话分配/释放大块音频缓冲（cpal PCM、WAV 编码、caulk 池），
//! session 结束回收资源后调一次，把 freelist 上的页归还内核。
//! 原版只在 drop 1.2GB 本地模型后调；这里没有本地模型，但同一
//! 「会话后归还」机制适用。

/// 让 libmalloc 把 freelist 上的物理页归还内核，返回本次释放的字节数（日志用）。
/// 调用时机：session 资源回收完成后（recorder stop / ASR 断开之后），归还
/// 本次会话分配产生的滞留页。
///
/// SAFETY: 系统 API；NULL zone + goal=0 = 对所有 zone 尽量多地归还，无内存安全风险。
pub fn pressure_relief() -> usize {
    unsafe { malloc_zone_pressure_relief(std::ptr::null_mut(), 0) }
}

extern "C" {
    fn malloc_zone_pressure_relief(zone: *mut libc::c_void, goal: libc::size_t) -> libc::size_t;
}
