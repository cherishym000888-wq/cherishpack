//! 런처 전용 디스크 레이아웃.
//!
//! 기존 `paths::AppDirs` 는 Prism 설치기 용으로 `%LOCALAPPDATA%\CherishPack\prism\...`
//! 까지 잡혀있다. 자체 런처는 Prism 없이 자체 경로를 쓰므로 독립적으로 해석한다.
//!
//! 레이아웃:
//!   %LOCALAPPDATA%\CherishWorld\
//!   ├── game\                     ← assets / libraries / versions / natives
//!   │   ├── assets\{indexes,objects}\
//!   │   ├── libraries\
//!   │   ├── versions\<id>\<id>.jar
//!   │   └── natives\<id>\
//!   ├── instance\                 ← 실제 .minecraft (mods/config/saves/...)
//!   ├── java\                     ← 다운받은 JRE
//!   ├── cache\
//!   ├── account.json
//!   └── logs\
//!
//! 버전/인스턴스 별로 natives 디렉토리를 분리해 충돌을 막는다.

use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};

pub const APP_NAME: &str = "CherishWorld";

#[derive(Debug, Clone)]
pub struct LauncherDirs {
    pub root: PathBuf,
    pub game: PathBuf,
    pub instance: PathBuf,
    pub java: PathBuf,
    pub cache: PathBuf,
    pub logs: PathBuf,

    pub assets: PathBuf,
    pub libraries: PathBuf,
    pub versions: PathBuf,
    pub natives_root: PathBuf,
}

impl LauncherDirs {
    pub fn resolve() -> Result<Self> {
        let base = local_appdata()?.join(APP_NAME);
        Ok(Self::at(base))
    }

    /// 테스트·대안 경로를 위한 명시적 생성.
    pub fn at(base: PathBuf) -> Self {
        let game = base.join("game");
        Self {
            assets: game.join("assets"),
            libraries: game.join("libraries"),
            versions: game.join("versions"),
            natives_root: game.join("natives"),
            instance: base.join("instance"),
            java: base.join("java"),
            cache: base.join("cache"),
            logs: base.join("logs"),
            game,
            root: base,
        }
    }

    pub fn ensure_exists(&self) -> Result<()> {
        for p in [
            &self.root,
            &self.game,
            &self.instance,
            &self.java,
            &self.cache,
            &self.logs,
            &self.assets,
            &self.libraries,
            &self.versions,
            &self.natives_root,
        ] {
            std::fs::create_dir_all(p)
                .map_err(|e| anyhow!("경로 생성 실패: {} ({})", p.display(), e))?;
        }
        Ok(())
    }

    pub fn client_jar(&self, version_id: &str) -> PathBuf {
        self.versions.join(version_id).join(format!("{}.jar", version_id))
    }

    pub fn natives_dir(&self, version_id: &str) -> PathBuf {
        self.natives_root.join(version_id)
    }
}

#[cfg(windows)]
fn local_appdata() -> Result<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("LOCALAPPDATA 환경변수를 찾을 수 없습니다"))
}

#[cfg(not(windows))]
fn local_appdata() -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("HOME not set"))?;
    Ok(home.join(".local/share"))
}

/// 런처 디렉토리에서 파생된 "실행 시 필요한 경로 번들".
///
/// `run::LaunchContext` 가 다수의 `&Path` 를 받는 대신 이걸 받도록 한다.
pub struct RuntimeLayout<'a> {
    pub dirs: &'a LauncherDirs,
    /// 최종 meta id — natives 디렉토리 격리 키로 쓰임 (예: "neoforge-21.1.220").
    pub version_id: &'a str,
    /// classpath 에 **추가로** 올릴 jar 목록.
    ///
    /// - 바닐라 런치: `[vanilla_client.jar]`
    /// - NeoForge: **빈 Vec** — NeoForge 라이브러리 목록에 이미 패치된 client jar 가
    ///   포함되므로 바닐라 jar 를 또 얹으면 `java.lang.module.ResolutionException`
    ///   (같은 패키지를 두 모듈이 export) 이 발생.
    pub extra_classpath: Vec<PathBuf>,
}

impl<'a> RuntimeLayout<'a> {
    pub fn game_dir(&self) -> &Path {
        &self.dirs.instance
    }
    pub fn assets_root(&self) -> &Path {
        &self.dirs.assets
    }
    pub fn libraries_dir(&self) -> &Path {
        &self.dirs.libraries
    }
    pub fn natives_dir(&self) -> PathBuf {
        self.dirs.natives_dir(self.version_id)
    }
    pub fn extra_classpath(&self) -> &[PathBuf] {
        &self.extra_classpath
    }
}
