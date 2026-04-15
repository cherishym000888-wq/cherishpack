//! 로깅: 콘솔 + `%LOCALAPPDATA%\CherishPack\logs\install.log` 일별 롤링.
//!
//! `_guard` 를 main 끝까지 살려두어야 로그가 플러시된다.

use anyhow::Result;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use crate::paths::AppDirs;

pub fn init(dirs: &AppDirs) -> Result<WorkerGuard> {
    let file_appender = tracing_appender::rolling::daily(&dirs.log_dir, "install.log");
    let (nb_writer, guard) = tracing_appender::non_blocking(file_appender);

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,cherishpack_installer=debug"));

    let file_layer = fmt::layer()
        .with_writer(nb_writer)
        .with_ansi(false)
        .with_target(true)
        .with_line_number(true);

    let stderr_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(false);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(file_layer)
        .with(stderr_layer)
        .init();

    Ok(guard)
}
