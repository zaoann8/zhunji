//! 日志初始化——simplelog 写 stderr + 文件（~/Library/Logs/Zhunji/zhunji.log）。
//!
//! 原版 zhunlu 用 ~/Library/Logs/OpenLess/openless.log + Swift `Log.swift`；
//! native 重写统一到 Zhunji 目录，P1 起 Swift 侧排查也读同一文件。

use simplelog::{
    ColorChoice, CombinedLogger, ConfigBuilder, LevelFilter, TermLogger, TerminalMode,
    WriteLogger,
};

const LOG_ROTATE_LIMIT_BYTES: u64 = 10 * 1024 * 1024;

/// 把日志同时写到 stderr + ~/Library/Logs/Zhunji/zhunji.log。
/// 幂等：simplelog CombinedLogger 重复 init 会失败，直接忽略。
pub(crate) fn init_file_logger() {
    let log_dir = log_dir_path();
    if let Err(e) = std::fs::create_dir_all(&log_dir) {
        eprintln!(
            "[logger] WARN create log dir failed path={}: {e}",
            log_dir.display()
        );
    }
    let log_file = log_dir.join("zhunji.log");
    if let Err(e) = rotate_log_if_too_large(&log_file) {
        eprintln!("[logger] WARN 日志轮转失败: {e}");
    }
    let config = ConfigBuilder::new().set_time_format_rfc3339().build();
    let mut loggers: Vec<Box<dyn simplelog::SharedLogger>> = vec![TermLogger::new(
        LevelFilter::Info,
        config.clone(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    )];
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)
    {
        Ok(file) => {
            loggers.push(WriteLogger::new(LevelFilter::Info, config, file));
            eprintln!("[logger] file logger ready path={}", log_file.display());
        }
        Err(e) => {
            eprintln!(
                "[logger] ERROR open log file failed path={}: {e}",
                log_file.display()
            );
        }
    }
    let _ = CombinedLogger::init(loggers);
}

fn rotate_log_if_too_large(path: &std::path::Path) -> std::io::Result<()> {
    let Ok(metadata) = std::fs::metadata(path) else {
        return Ok(());
    };
    if metadata.len() <= LOG_ROTATE_LIMIT_BYTES {
        return Ok(());
    }

    let archive = path.with_file_name("zhunji.log.1");
    match std::fs::remove_file(&archive) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    std::fs::rename(path, archive)
}

pub fn log_dir_path() -> std::path::PathBuf {
    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return std::path::PathBuf::from(home)
                .join("Library")
                .join("Logs")
                .join("Zhunji");
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            return std::path::PathBuf::from(local).join("Zhunji").join("Logs");
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Ok(home) = std::env::var("HOME") {
            return std::path::PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("Zhunji")
                .join("logs");
        }
    }
    std::env::temp_dir().join("Zhunji")
}
