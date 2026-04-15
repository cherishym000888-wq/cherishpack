//! 설치 프로그램이 쓰는 모든 경로를 한 곳에서 해석한다.
//!
//! 레이아웃 (Windows):
//!   %LOCALAPPDATA%\CherishPack\
//!   ├── installer-state.json
//!   ├── current-manifest.json
//!   ├── install.log
//!   ├── prism\                (포터블 Prism Launcher)
//!   │   └── instances\cherishpack\minecraft\...
//!   └── cache\                (다운로드 캐시)

use anyhow::{anyhow, Result};
use std::path::PathBuf;

/// 앱 이름 — 폴더·레지스트리 키에 쓰인다.
pub const APP_NAME: &str = "CherishPack";

/// Prism 인스턴스 이름.
pub const INSTANCE_NAME: &str = "cherishpack";

#[derive(Debug, Clone)]
pub struct AppDirs {
    pub root: PathBuf,
    pub prism_root: PathBuf,
    pub instance_root: PathBuf,
    pub minecraft_root: PathBuf,
    pub cache: PathBuf,
    pub state_file: PathBuf,
    pub manifest_file: PathBuf,
    pub log_dir: PathBuf,
}

impl AppDirs {
    pub fn resolve() -> Result<Self> {
        let base = local_appdata()?.join(APP_NAME);

        let prism_root = base.join("prism");
        let instance_root = prism_root.join("instances").join(INSTANCE_NAME);
        let minecraft_root = instance_root.join("minecraft");

        Ok(Self {
            cache: base.join("cache"),
            state_file: base.join("installer-state.json"),
            manifest_file: base.join("current-manifest.json"),
            log_dir: base.join("logs"),
            prism_root,
            instance_root,
            minecraft_root,
            root: base,
        })
    }

    pub fn ensure_exists(&self) -> Result<()> {
        for p in [&self.root, &self.cache, &self.log_dir] {
            std::fs::create_dir_all(p)
                .map_err(|e| anyhow!("경로 생성 실패: {} ({})", p.display(), e))?;
        }
        Ok(())
    }
}

#[cfg(windows)]
fn local_appdata() -> Result<PathBuf> {
    // 환경변수 우선 — SHGetKnownFolderPath는 windows 크레이트로 가능하지만
    // LOCALAPPDATA 환경변수가 모든 정상 Windows 환경에서 세팅되어 있어 충분하다.
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("LOCALAPPDATA 환경변수를 찾을 수 없습니다"))
}

#[cfg(not(windows))]
fn local_appdata() -> Result<PathBuf> {
    // 개발 편의 — 비 Windows에서도 컴파일은 되게
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("HOME not set"))?;
    Ok(home.join(".local/share"))
}
