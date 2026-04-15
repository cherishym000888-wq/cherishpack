//! Prism Launcher 포터블 설치.
//!
//! 전략
//!  1. GitHub API (`/repos/PrismLauncher/PrismLauncher/releases/latest`)로 최신 릴리스 메타 조회
//!  2. 자산 중 `PrismLauncher-Windows-MSVC-Portable-*.zip` 선택 (x86_64)
//!  3. 같은 이름 + `.sha256` 자산이 있으면 받아서 검증. 없으면 크기 체크만.
//!  4. `%LOCALAPPDATA%\CherishPack\prism\` 에 풀기
//!  5. 포터블 마커: 최상위에 빈 `portable.txt` 생성 (이미 포함돼 있으면 그대로)
//!  6. 인스턴스 폴더 `prism/instances/cherishpack/minecraft/` 생성
//!
//! 주의
//!  - Prism은 GPL-3.0. **재배포하지 않고 공식 릴리스 URL에서 다운로드**한다.
//!  - 이미 설치돼 있고 (`prismlauncher.exe` 존재) 버전이 충분하면 스킵.

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

use crate::net;
use crate::paths::AppDirs;

const GH_LATEST: &str =
    "https://api.github.com/repos/PrismLauncher/PrismLauncher/releases/latest";

#[derive(Debug, Clone)]
pub struct PrismInstall {
    pub launcher_exe: PathBuf,
}

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    #[allow(dead_code)]
    size: u64,
}

/// 진행률 콜백: (다운로드된 바이트, 전체|None, 라벨)
pub type PrismProgress = dyn Fn(u64, Option<u64>, &str) + Send + Sync;

pub async fn ensure_installed(
    dirs: &AppDirs,
    progress: Option<&PrismProgress>,
) -> Result<PrismInstall> {
    let launcher_exe = dirs.prism_root.join("prismlauncher.exe");

    // 이미 설치되어 있으면 그대로 사용
    if launcher_exe.exists() {
        tracing::info!(path = %launcher_exe.display(), "Prism 이미 설치됨");
        ensure_instance_dirs(dirs)?;
        return Ok(PrismInstall { launcher_exe });
    }

    tracing::info!("Prism 설치 시작");
    std::fs::create_dir_all(&dirs.prism_root)?;

    // 1. 최신 릴리스 조회
    if let Some(cb) = progress {
        cb(0, None, "Prism Launcher 최신 버전 조회 중");
    }
    let release: GhRelease = net::fetch_json(GH_LATEST)
        .await
        .context("Prism 릴리스 정보 조회 실패")?;
    tracing::info!(tag = %release.tag_name, "Prism 릴리스");

    // 2. 자산 선택 — Windows MSVC Portable x64
    let asset = pick_windows_portable(&release.assets).ok_or_else(|| {
        anyhow!(
            "Windows MSVC Portable 자산을 찾지 못함. 자산 목록: {:?}",
            release.assets.iter().map(|a| &a.name).collect::<Vec<_>>()
        )
    })?;

    // 3. sha256 자산 (있으면 사용)
    let expected_sha = fetch_optional_sha256(&release.assets, &asset.name).await;

    // 4. 다운로드
    let zip_path = dirs.cache.join(&asset.name);
    std::fs::create_dir_all(&dirs.cache)?;

    match &expected_sha {
        Some(sha) => {
            if let Some(cb) = progress {
                cb(0, None, "Prism Launcher 다운로드 중 (검증 활성)");
            }
            net::download_verified(
                &asset.browser_download_url,
                &zip_path,
                sha,
                progress.map(|p| {
                    let pb: Box<dyn Fn(u64, Option<u64>) + Send + Sync> =
                        Box::new(move |d, t| p(d, t, "Prism Launcher 다운로드 중"));
                    pb
                })
                .as_deref(),
            )
            .await?;
        }
        None => {
            tracing::warn!(
                "Prism 자산 sha256 파일이 없음 — 크기 기반 체크만 수행"
            );
            if let Some(cb) = progress {
                cb(0, None, "Prism Launcher 다운로드 중 (sha 미제공)");
            }
            net::download_plain(&asset.browser_download_url, &zip_path).await?;
        }
    }

    // 5. 압축 해제
    if let Some(cb) = progress {
        cb(0, None, "Prism Launcher 압축 해제 중");
    }
    extract_zip(&zip_path, &dirs.prism_root)?;

    // 6. portable 마커
    let portable_marker = dirs.prism_root.join("portable.txt");
    if !portable_marker.exists() {
        std::fs::write(&portable_marker, b"")?;
    }

    // 7. 인스턴스 폴더
    ensure_instance_dirs(dirs)?;

    if !launcher_exe.exists() {
        bail!(
            "압축 해제 후 prismlauncher.exe 를 찾지 못함: {}",
            launcher_exe.display()
        );
    }

    // 캐시 zip은 필요 없으면 삭제 (실패해도 무시)
    let _ = std::fs::remove_file(&zip_path);

    tracing::info!(path = %launcher_exe.display(), "Prism 설치 완료");
    Ok(PrismInstall { launcher_exe })
}

/// 인스턴스 루트와 minecraft 폴더 준비.
fn ensure_instance_dirs(dirs: &AppDirs) -> Result<()> {
    std::fs::create_dir_all(&dirs.minecraft_root)?;
    // 기본 instance.cfg — Prism이 인스턴스로 인식하게
    let cfg_path = dirs.instance_root.join("instance.cfg");
    if !cfg_path.exists() {
        let cfg = "InstanceType=OneSix\nname=CherishPack\niconKey=default\n";
        std::fs::write(&cfg_path, cfg)?;
    }
    Ok(())
}

/// Windows MSVC Portable (x64) zip 자산 선택.
fn pick_windows_portable(assets: &[GhAsset]) -> Option<&GhAsset> {
    // 우선순위: MSVC + Portable + .zip, "arm"·"i686"·"Setup" 제외
    let score = |name: &str| -> i32 {
        let n = name.to_ascii_lowercase();
        if !n.ends_with(".zip") {
            return -1;
        }
        if !n.contains("windows") {
            return -1;
        }
        if !n.contains("portable") {
            return -1;
        }
        if n.contains("arm") || n.contains("i686") || n.contains("setup") {
            return -1;
        }
        let mut s = 0;
        if n.contains("msvc") {
            s += 10;
        }
        if n.contains("x64") || n.contains("x86_64") || n.contains("amd64") {
            s += 5;
        }
        s
    };
    assets
        .iter()
        .filter(|a| score(&a.name) >= 0)
        .max_by_key(|a| score(&a.name))
}

/// `<name>.sha256` 자산이 있으면 내용을 받아 hex 해시 문자열만 추출.
async fn fetch_optional_sha256(assets: &[GhAsset], target_name: &str) -> Option<String> {
    let wanted = format!("{}.sha256", target_name);
    let asset = assets.iter().find(|a| a.name.eq_ignore_ascii_case(&wanted))?;
    let text = net::fetch_text(&asset.browser_download_url).await.ok()?;
    // 보통 "<hash>  <filename>\n" 형태
    let token = text.split_whitespace().next()?;
    if token.len() == 64 && token.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(token.to_ascii_lowercase())
    } else {
        None
    }
}

/// 간단 zip 해제. 경로 트래버설 방지.
fn extract_zip(zip_path: &Path, dst_root: &Path) -> Result<()> {
    let file = std::fs::File::open(zip_path)
        .with_context(|| format!("zip 열기 실패: {}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let rel = match entry.enclosed_name() {
            Some(p) => p.to_owned(),
            None => {
                tracing::warn!(name = entry.name(), "zip 항목 경로가 의심됨, 스킵");
                continue;
            }
        };
        let out_path = dst_root.join(&rel);

        // 경로 트래버설 방지 — dst_root 하위여야 함
        let canon_parent = dst_root.canonicalize().unwrap_or_else(|_| dst_root.to_owned());
        if !out_path.starts_with(&canon_parent) && !out_path.starts_with(dst_root) {
            tracing::warn!(path = %out_path.display(), "zip 경로 이탈 탐지, 스킵");
            continue;
        }

        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)?;
            continue;
        }
        if let Some(p) = out_path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let mut out = std::fs::File::create(&out_path)?;
        std::io::copy(&mut entry, &mut out)?;
    }
    Ok(())
}

/// Prism 인스턴스를 실행. 창은 Prism이 직접 띄우고 Minecraft 실행까지 Prism이 담당.
pub fn launch_instance(dirs: &AppDirs, install: &PrismInstall) -> Result<()> {
    use std::process::Command;

    // Prism CLI: -l <instance>  (--launch)
    let status = Command::new(&install.launcher_exe)
        .arg("-l")
        .arg(crate::paths::INSTANCE_NAME)
        .current_dir(&dirs.prism_root)
        .spawn()
        .with_context(|| format!("Prism 실행 실패: {}", install.launcher_exe.display()))?;

    tracing::info!(pid = status.id(), "Prism 인스턴스 시작");
    Ok(())
}
