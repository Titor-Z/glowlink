//! Model：把字节/时间/百分比变成界面上那几个字符。
//!
//! 与 CLI 的 `view/cli.rs` 是**同一套口径**（1 KB = 1024 B、小于 1 MB/s 用 KB/s），
//! 但 desktop 不许依赖 cli（那是另一个进程的表现层），所以这里是第二份实现。
//! 为了不让两份悄悄漂开，两边各有测试钉住同一组用例。

/// 人类可读的字节数。
pub fn human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * KB;
    const GB: f64 = 1024.0 * MB;
    let value = bytes as f64;
    if value < KB {
        format!("{bytes} B")
    } else if value < MB {
        format!("{:.1} KB", value / KB)
    } else if value < GB {
        format!("{:.1} MB", value / MB)
    } else {
        format!("{:.2} GB", value / GB)
    }
}

/// 速度：低于 1 MB/s 用 KB/s（不然会一直显示「0.0 MB/s」，看着像卡死）。
pub fn human_speed(bytes_per_sec: f64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    if bytes_per_sec < MB {
        format!("{:.0} KB/s", bytes_per_sec / 1024.0)
    } else {
        format!("{:.1} MB/s", bytes_per_sec / MB)
    }
}

/// 剩余秒数。总量未知时给 `None`——**不编造「剩 0 秒」**。
pub fn eta_seconds(remaining: u64, bytes_per_sec: f64) -> Option<f64> {
    if bytes_per_sec <= 0.0 {
        return None;
    }
    Some(remaining as f64 / bytes_per_sec)
}

/// 秒 → 「x 分 y 秒」/「y 秒」。不足 0.1 秒照实说。
pub fn human_eta(seconds: f64) -> String {
    if seconds < 0.1 {
        return "不足 0.1 秒".to_string();
    }
    if seconds < 60.0 {
        return format!("{seconds:.1} 秒");
    }
    let minutes = (seconds / 60.0).floor();
    let rest = seconds - minutes * 60.0;
    format!("{minutes:.0} 分 {rest:.0} 秒")
}

/// 百分比：**只进不退**（进度偶尔回跳时不要把数字也拖回去）。
pub fn percent(done: u64, total: u64) -> u8 {
    if total == 0 {
        return 0;
    }
    let value = (done as f64 / total as f64) * 100.0;
    value.clamp(0.0, 100.0).round() as u8
}
