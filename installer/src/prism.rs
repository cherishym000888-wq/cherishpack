//! Prism Launcher 포터블 설치.
//!
//! Phase 2에서 구현. 지금은 시그니처만.
//!
//! 전략:
//!  1. GitHub API로 PrismLauncher/PrismLauncher latest 릴리스 조회
//!  2. Windows MSVC portable zip 자산 선택 (아키텍처: x86_64)
//!  3. sha256 알려지지 않으면 자산 `.sha256` 파일 병행 받아 비교
//!  4. `%LOCALAPPDATA%\CherishPack\prism\` 에 풀고 포터블 모드로 설정
//!     (prismlauncher.cfg 옆에 빈 `portable.txt` 생성 등)
//!  5. 인스턴스 디렉터리 생성 (`prism/instances/cherishpack/`)

use anyhow::Result;

use crate::paths::AppDirs;

pub struct PrismInstall {
    pub launcher_exe: std::path::PathBuf,
}

pub async fn ensure_installed(_dirs: &AppDirs) -> Result<PrismInstall> {
    anyhow::bail!("prism::ensure_installed — Phase 2에서 구현 예정")
}

pub async fn launch_instance(_dirs: &AppDirs) -> Result<()> {
    anyhow::bail!("prism::launch_instance — Phase 2에서 구현 예정")
}
