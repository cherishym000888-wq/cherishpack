//! 언인스톨러. Phase 2 구현.
//!
//! - Prism 인스턴스 폴더를 휴지통으로 이동 (saves 보호는 사용자에게 질의)
//! - %LOCALAPPDATA%\CherishPack\ 정리
//! - HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\CherishPack 레지스트리 제거

use anyhow::Result;

use crate::paths::AppDirs;

pub fn run(_dirs: &AppDirs) -> Result<()> {
    tracing::warn!("uninstall — Phase 2에서 구현 예정");
    Ok(())
}
