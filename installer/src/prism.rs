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

// Prism 11+ 는 첫 실행 시 prismlauncher.cfg 의 Language=ko 를 무시하고 영어 UI 로 뜨는
// init-order 이슈가 있음. 10.0.5 는 해당 이슈가 없어서 시드한 언어가 바로 적용됨.
// 새 Prism 이 필요해지면 버전 올리기.
const GH_LATEST: &str =
    "https://api.github.com/repos/PrismLauncher/PrismLauncher/releases/tags/10.0.5";

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

    // 진행률 세부 중계는 생략하고 단계 라벨만 전달 (MVP)
    if let Some(cb) = progress {
        cb(0, None, "Prism Launcher 다운로드 중");
    }
    match &expected_sha {
        Some(sha) => {
            net::download_verified(&asset.browser_download_url, &zip_path, sha, None).await?;
        }
        None => {
            tracing::warn!("Prism 자산 sha256 파일이 없음 — 크기 기반 체크만 수행");
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

/// 기존 시스템의 PrismLauncher / PolyMC / MultiMC 설치에서 `accounts.json` 을
/// 포터블 Prism 루트로 복사. 이미 대상 파일이 있으면 건드리지 않는다.
/// 데모 모드로 실행되어 Pixelmon 메인메뉴에서 NPE로 크래시 나는 문제 회피.
pub fn import_accounts_if_missing(dirs: &AppDirs) -> Result<bool> {
    let dst = dirs.prism_root.join("accounts.json");
    if dst.exists() {
        return Ok(false);
    }
    let appdata = match std::env::var_os("APPDATA") {
        Some(v) => std::path::PathBuf::from(v),
        None => return Ok(false),
    };
    let candidates = [
        appdata.join("PrismLauncher").join("accounts.json"),
        appdata.join("PolyMC").join("accounts.json"),
        appdata.join("MultiMC").join("accounts.json"),
    ];
    for src in &candidates {
        if src.exists() {
            if let Err(e) = std::fs::copy(src, &dst) {
                tracing::warn!(from = %src.display(), err = %e, "accounts.json 복사 실패");
                continue;
            }
            tracing::info!(from = %src.display(), "기존 Prism 계정 가져오기 성공");
            return Ok(true);
        }
    }
    Ok(false)
}

/// accounts.json 이 없으면 오프라인 계정 하나를 심는다.
/// 이미 파일이 있으면 건드리지 않아 기존/MS 계정 보존.
pub fn seed_offline_account_if_missing(dirs: &AppDirs, nickname: &str) -> Result<bool> {
    let path = dirs.prism_root.join("accounts.json");
    if path.exists() {
        return Ok(false);
    }
    std::fs::create_dir_all(&dirs.prism_root)?;
    let name = if nickname.trim().is_empty() { "Player" } else { nickname };
    let uuid = offline_uuid(name);
    let json = serde_json::json!({
        "formatVersion": 3,
        "accounts": [{
            "active": true,
            "type": "Offline",
            "profile": {
                "id": uuid,
                "name": name,
                "capes": [],
                "skin": { "id": "", "url": "", "variant": "CLASSIC" }
            }
        }]
    });
    std::fs::write(&path, serde_json::to_vec_pretty(&json)?)?;
    tracing::info!(path = %path.display(), name, "오프라인 계정 기본값 생성");
    Ok(true)
}

fn offline_uuid(name: &str) -> String {
    use md5::{Digest, Md5};
    let mut h = Md5::new();
    h.update(format!("OfflinePlayer:{name}").as_bytes());
    let mut b: [u8; 16] = h.finalize().into();
    // UUID v3 (name-based, MD5) 규약: version/variant 비트 세팅
    b[6] = (b[6] & 0x0f) | 0x30;
    b[8] = (b[8] & 0x3f) | 0x80;
    hex::encode(b)
}

/// prismlauncher.cfg 가 없으면 기본 설정을 심어 첫 실행 마법사(언어/테마/업데이트)를 건너뛴다.
/// - `ConfigVersion` 존재 = 마법사 스킵 트리거
/// - `Language=ko` = 언어 선택 창 스킵
/// - `ApplicationTheme`/`IconTheme` = Appearance 창 스킵
/// - `CloseAfterLaunch=true` = MC 기동 후 Prism 창 자동 종료 (몰입감)
/// - `UpdateDialogCheckDate` 장기 미래 = 업데이트 팝업 억제
pub fn write_default_prism_cfg_if_missing(dirs: &AppDirs) -> Result<bool> {
    let path = dirs.prism_root.join("prismlauncher.cfg");
    if path.exists() {
        return Ok(false);
    }
    std::fs::create_dir_all(&dirs.prism_root)?;
    // Prism 11 첫 실행 시 명시적 Language=ko 로 시드해도 영어 UI 로 뜨는 현상이 있음
    // (Qt 번역기 로드 타이밍 이슈 추정). 윈도우 로케일 사용을 명시하고 Language 도 병행 설정.
    // 타깃 사용자가 모두 한국인이므로 시스템 로케일 = ko_KR 로 가정.
    let content = "[General]\n\
         Language=ko\n\
         UseSystemLocale=true\n\
         ConfigVersion=1.3\n\
         ApplicationTheme=bright\n\
         IconTheme=pe_colored\n\
         CloseAfterLaunch=true\n\
         ShowConsole=false\n\
         AutoCloseConsole=true\n\
         ShowConsoleOnError=true\n\
         ConsoleMaxLines=100000\n\
         ConsoleOverflowStop=true\n\
         UpdateDialogCheckDate=2099-12-31\n";
    std::fs::write(&path, content)?;
    tracing::info!(path = %path.display(), "prismlauncher.cfg 기본값 생성 (마법사 스킵)");
    Ok(true)
}

/// Prism 의 한국어 번역 파일(mmc_ko.qm) 을 미리 받아 `translations/` 에 저장한다.
/// Prism 은 이 폴더의 `.qm` 파일을 직접 로드하므로, 첫 실행부터 한국어 UI 가 뜬다.
/// 번역 인덱스는 Prism 포터블이 기본 포함하는 `translations/index_v2.json` 을 사용.
/// 네트워크 실패 시 조용히 경고만 (기본 영어 UI 로 폴백).
pub async fn ensure_korean_translation(dirs: &AppDirs) -> Result<()> {
    // Prism 번역 파일은 sha1 기반 이름으로 배포됨. 번역이 갱신되면 sha1 이 바뀌어
    // 이 상수도 업데이트해야 하지만, 한 번에 단일 다운로드만 하면 되므로 index
    // 조회 단계에서 실패할 위험을 없앨 수 있다. (2026-04-23 기준 최신 ko 번역)
    const KO_QM_SHA1: &str = "afda617b83cf0012ddfb2b0444ea649ddecc5f2d";

    let trans_dir = dirs.prism_root.join("translations");
    let qm_path = trans_dir.join("mmc_ko.qm");
    if qm_path.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(&trans_dir)?;
    let url = format!("https://i18n.prismlauncher.org/{}.class", KO_QM_SHA1);
    tracing::info!(url, "한국어 번역 파일 다운로드");
    net::download_plain(&url, &qm_path)
        .await
        .with_context(|| format!("download failed: {}", url))?;
    tracing::info!(path = %qm_path.display(), "한국어 번역 파일 설치 완료");
    Ok(())
}

/// 첫 실행 시 한국어 + 접근성 온보딩 스킵을 위한 options.txt 기본값.
/// 이미 파일이 있으면 건드리지 않는다 (사용자 설정 보존).
pub fn write_default_options_if_missing(dirs: &AppDirs) -> Result<()> {
    let path = dirs.minecraft_root.join("options.txt");
    if path.exists() {
        return Ok(());
    }
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let content = "lang:ko_kr\nonboardAccessibility:false\n";
    std::fs::write(&path, content)?;
    tracing::info!(path = %path.display(), "기본 options.txt 생성 (ko_kr, 접근성 스킵)");
    Ok(())
}

/// instance.cfg 의 특정 key=value 를 set/update. 없으면 append, 있으면 교체.
/// Prism 이 인스턴스를 사용 중이면 다음 실행 때 반영됨.
pub fn set_instance_cfg_kv(dirs: &AppDirs, updates: &[(&str, &str)]) -> Result<()> {
    let cfg_path = dirs.instance_root.join("instance.cfg");
    let mut content = if cfg_path.exists() {
        std::fs::read_to_string(&cfg_path).unwrap_or_default()
    } else {
        String::new()
    };
    for (k, v) in updates {
        let prefix = format!("{}=", k);
        let new_line = format!("{}={}", k, v);
        let mut lines: Vec<String> = content.lines().map(String::from).collect();
        let mut found = false;
        for line in &mut lines {
            if line.starts_with(&prefix) {
                *line = new_line.clone();
                found = true;
                break;
            }
        }
        if !found {
            lines.push(new_line);
        }
        content = lines.join("\n");
        if !content.ends_with('\n') {
            content.push('\n');
        }
    }
    std::fs::write(&cfg_path, content)?;
    Ok(())
}

/// options.txt 에 `soundCategory_music:0.0` 설정 (덮어쓰기). 다른 카테고리는 건드리지 않음.
pub fn mute_mc_music(dirs: &AppDirs) -> Result<()> {
    let path = dirs.minecraft_root.join("options.txt");
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = content.lines().map(String::from).collect();
    let mut found = false;
    for line in &mut lines {
        if line.starts_with("soundCategory_music:") {
            *line = "soundCategory_music:0.0".into();
            found = true;
        }
    }
    if !found {
        lines.push("soundCategory_music:0.0".into());
    }
    std::fs::write(&path, lines.join("\n") + "\n")?;
    Ok(())
}

/// 인스턴스 루트와 minecraft 폴더 준비.
fn ensure_instance_dirs(dirs: &AppDirs) -> Result<()> {
    std::fs::create_dir_all(&dirs.minecraft_root)?;
    // 기본 instance.cfg — Prism이 인스턴스로 인식하게
    let cfg_path = dirs.instance_root.join("instance.cfg");
    if !cfg_path.exists() {
        let cfg = "InstanceType=OneSix\nname=CherishPack\niconKey=default\nJavaVersion=\nOverrideJavaArgs=false\nOverrideMemory=false\n";
        std::fs::write(&cfg_path, cfg)?;
    }
    Ok(())
}

/// Prism 이 인스턴스를 읽기 위해 필요한 `mmc-pack.json` 작성.
/// 로더 종류에 따라 uid 가 다르다.
pub fn write_mmc_pack(
    dirs: &AppDirs,
    minecraft_version: &str,
    loader_type: &str,
    loader_version: &str,
) -> Result<()> {
    let loader_uid = match loader_type.to_ascii_lowercase().as_str() {
        "neoforge" => "net.neoforged",
        "forge" => "net.minecraftforge",
        "fabric" => "net.fabricmc.fabric-loader",
        "quilt" => "org.quiltmc.quilt-loader",
        other => anyhow::bail!("알 수 없는 로더 타입: {}", other),
    };

    let json = serde_json::json!({
        "formatVersion": 1,
        "components": [
            {
                "important": true,
                "uid": "net.minecraft",
                "version": minecraft_version
            },
            {
                "uid": loader_uid,
                "version": loader_version
            }
        ]
    });

    let path = dirs.instance_root.join("mmc-pack.json");
    std::fs::create_dir_all(&dirs.instance_root)
        .with_context(|| format!("인스턴스 디렉토리 생성 실패: {}", dirs.instance_root.display()))?;
    std::fs::write(&path, serde_json::to_vec_pretty(&json)?)
        .with_context(|| format!("mmc-pack.json 쓰기 실패: {}", path.display()))?;
    tracing::info!(path = %path.display(), "mmc-pack.json 기록");
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
/// 부모 프로세스(installer)가 끝나도 Prism 이 계속 살아있도록 디태치.
pub fn launch_instance(dirs: &AppDirs, install: &PrismInstall) -> Result<()> {
    spawn_detached(&install.launcher_exe, &dirs.prism_root)
        .with_context(|| format!("Prism 실행 실패: {}", install.launcher_exe.display()))?;
    tracing::info!("Prism 인스턴스 실행 요청");
    Ok(())
}

/// Windows 전용: DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP 로 디태치 실행.
#[cfg(windows)]
pub fn spawn_detached(exe: &std::path::Path, workdir: &std::path::Path) -> Result<()> {
    spawn_detached_ex(exe, workdir, true)
}

#[cfg(windows)]
pub fn spawn_detached_ex(
    exe: &std::path::Path,
    workdir: &std::path::Path,
    auto_launch: bool,
) -> Result<()> {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

    let mut cmd = std::process::Command::new(exe);
    if auto_launch {
        cmd.arg("-l").arg(crate::paths::INSTANCE_NAME);
    }
    cmd.current_dir(workdir)
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let child = cmd.spawn()?;
    tracing::info!(pid = child.id(), auto_launch, "Prism 디태치 실행");
    Ok(())
}

#[cfg(not(windows))]
pub fn spawn_detached(exe: &std::path::Path, workdir: &std::path::Path) -> Result<()> {
    spawn_detached_ex(exe, workdir, true)
}

#[cfg(not(windows))]
pub fn spawn_detached_ex(
    exe: &std::path::Path,
    workdir: &std::path::Path,
    auto_launch: bool,
) -> Result<()> {
    let mut cmd = std::process::Command::new(exe);
    if auto_launch {
        cmd.arg("-l").arg(crate::paths::INSTANCE_NAME);
    }
    let _ = cmd.current_dir(workdir).spawn()?;
    Ok(())
}
