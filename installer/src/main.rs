//! CherishPack Installer — 진입점
//!
//! Phase 1 구현 범위:
//!  - 로깅 초기화
//!  - 경로·상태 디렉터리 준비
//!  - 원격 version.json 로드 (skeleton)
//!  - GUI 띄우기 (skeleton)
//!
//! 이후 Phase에서 prism / mrpack / patcher / gui 본체를 채운다.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod channel;
mod config;
mod crash;
mod gui;
mod hash;
mod hwdetect;
mod logger;
mod mrpack;
mod net;
mod paths;
mod patcher;
mod preserve;
mod preset;
mod prism;
mod uninstall;

use anyhow::Result;
use tracing::{error, info};

fn main() -> Result<()> {
    // 1. 경로 준비 (%LOCALAPPDATA%\CherishPack\)
    let dirs = paths::AppDirs::resolve()?;
    dirs.ensure_exists()?;

    // 2. 로깅 초기화 (콘솔 + install.log 롤링)
    let _guard = logger::init(&dirs)?;

    info!(version = env!("CARGO_PKG_VERSION"), "CherishPack installer 시작");
    info!(?dirs, "경로 확인");

    // 3. CLI 인자 분기 (--uninstall 등)
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--uninstall") {
        return uninstall::run(&dirs);
    }

    // 4. GUI 실행 — 이후 Phase에서 실제 화면 구현
    if let Err(e) = gui::run(dirs) {
        error!(error = ?e, "GUI 종료 오류");
        return Err(e);
    }

    Ok(())
}
