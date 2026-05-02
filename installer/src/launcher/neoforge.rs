//! NeoForge 지원.
//!
//! 흐름:
//!   1. NeoForge installer jar 다운로드 (maven.neoforged.net)
//!   2. installer 내부의 `version.json` 추출 → `ForgeMeta` 로 파싱
//!   3. `inheritsFrom` 기반으로 바닐라 `VersionMeta` 와 병합 → 단일 `VersionMeta`
//!   4. (필요 시) ForgeWrapper jar 다운로드 — mainClass 로 쓰임
//!
//! 우리는 Prism 과 같은 방식: ForgeWrapper 가 최초 실행 시 installer 를
//! 파싱해 processors 를 실행하고 필요한 patched jar 들을 생성하도록 맡긴다.
//! 이렇게 하면 installer 가 하는 NeoForge 설치 로직을 Rust 로 재구현할 필요가 없다.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::{io::Read, path::Path, process::Stdio};
use tokio::io::{AsyncBufReadExt, BufReader};

use super::meta::{
    ArgEntry, Arguments, DownloadArtifact, Library, LibraryDownloads, VersionMeta,
};

/// maven.neoforged.net 릴리스 저장소 기준 installer URL.
pub fn installer_url(neoforge_version: &str) -> String {
    format!(
        "https://maven.neoforged.net/releases/net/neoforged/neoforge/{v}/neoforge-{v}-installer.jar",
        v = neoforge_version
    )
}

/// NeoForge installer 다운로드 (sha1 이 maven 에 별도로 있으나 간단히 생략 —
/// 실패 시 재시도는 `net::fetch_bytes` 가 처리).
pub async fn fetch_installer(neoforge_version: &str, dst: &Path) -> Result<()> {
    if dst.exists() {
        tracing::debug!(path = %dst.display(), "기존 NeoForge installer 재사용");
        return Ok(());
    }
    if let Some(parent) = dst.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    let url = installer_url(neoforge_version);
    let bytes = crate::net::fetch_bytes(&url)
        .await
        .with_context(|| format!("NeoForge installer 다운로드 실패: {}", url))?;
    tokio::fs::write(dst, &bytes).await?;
    Ok(())
}

/// installer jar 내부의 `version.json` 파일을 읽어 파싱.
pub fn extract_version_json(installer: &Path) -> Result<ForgeMeta> {
    let file = std::fs::File::open(installer)
        .with_context(|| format!("installer 열기 실패: {}", installer.display()))?;
    let mut archive = zip::ZipArchive::new(file).context("installer jar zip 파싱 실패")?;

    let mut zf = archive
        .by_name("version.json")
        .context("installer 안에 version.json 이 없음")?;
    let mut buf = String::new();
    zf.read_to_string(&mut buf)?;
    serde_json::from_str(&buf).context("NeoForge version.json 파싱 실패")
}

// ─────────────────────── ForgeMeta ───────────────────────

/// NeoForge/Forge version.json — 대부분 필드가 Optional. `inheritsFrom` 이 핵심.
#[derive(Debug, Deserialize)]
pub struct ForgeMeta {
    pub id: String,
    #[serde(rename = "inheritsFrom")]
    pub inherits_from: String,
    #[serde(rename = "mainClass")]
    pub main_class: String,
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub libraries: Vec<Library>,
    #[serde(default)]
    pub arguments: Option<ForgeArguments>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ForgeArguments {
    #[serde(default)]
    pub game: Vec<ArgEntry>,
    #[serde(default)]
    pub jvm: Vec<ArgEntry>,
}

// ─────────────────────── 병합 ───────────────────────

/// NeoForge 메타를 바닐라 위에 얹어 최종 `VersionMeta` 를 만든다.
///
/// 규칙 (Forge 전통에 따름):
///   - `mainClass` : forge 값으로 override
///   - `id` / `kind` : forge 값 우선
///   - `libraries` : **forge 것이 앞**, 바닐라 뒤 (같은 maven coord 가 있으면 forge 우선)
///   - `arguments.game` / `arguments.jvm` : 바닐라 뒤에 forge append
///   - `assetIndex` / `downloads` / `javaVersion` : 바닐라 유지 (forge 에 없음)
pub fn merge(forge: ForgeMeta, vanilla: VersionMeta) -> VersionMeta {
    if forge.inherits_from != vanilla.id {
        tracing::warn!(
            expected = %forge.inherits_from,
            got = %vanilla.id,
            "inheritsFrom 이 부모 바닐라 id 와 다름 — 그래도 병합 진행",
        );
    }

    let mut libraries = forge.libraries;
    let forge_coords: std::collections::HashSet<String> =
        libraries.iter().map(|l| maven_coord_key(&l.name)).collect();
    for v in vanilla.libraries {
        if !forge_coords.contains(&maven_coord_key(&v.name)) {
            libraries.push(v);
        }
    }

    // 인자 병합: 바닐라 → forge
    let arguments = match (vanilla.arguments, forge.arguments) {
        (Some(mut va), Some(fa)) => {
            va.game.extend(fa.game);
            va.jvm.extend(fa.jvm);
            Some(va)
        }
        (Some(va), None) => Some(va),
        (None, Some(fa)) => Some(Arguments {
            game: fa.game,
            jvm: fa.jvm,
        }),
        (None, None) => None,
    };

    VersionMeta {
        id: forge.id,
        main_class: forge.main_class,
        kind: forge.kind.or(vanilla.kind),
        asset_index: vanilla.asset_index,
        java_version: vanilla.java_version,
        downloads: vanilla.downloads,
        libraries,
        arguments,
        legacy_minecraft_arguments: vanilla.legacy_minecraft_arguments,
    }
}

/// Maven 좌표에서 "group:artifact" 부분만 뽑는다 (`:version:classifier` 제거).
fn maven_coord_key(name: &str) -> String {
    let mut it = name.splitn(3, ':');
    match (it.next(), it.next()) {
        (Some(g), Some(a)) => format!("{}:{}", g, a),
        _ => name.to_string(),
    }
}

// ─────────────────────── forge 라이브러리 URL 보강 ───────────────────────

/// NeoForge version.json 의 일부 라이브러리는 `downloads.artifact.url` 이 빈
/// 문자열로 들어있다 (installer 가 processors 를 통해 로컬에서 생성함).
/// 이런 항목은 다운로드 건너뛰고 classpath 에만 올리며, installer 를 통해
/// 실제 파일이 배치되었다고 가정한다 — ForgeWrapper 가 그 역할.
///
/// 실제 런처에서는 url 이 비어있는 엔트리를 필터링하는 로직이 필요하다.
/// 여기서 바로 그 판단을 해서 분리해주는 편의 함수.
#[allow(dead_code)] // 현재는 libraries::fetch_one 이 url 빈 항목을 직접 스킵 — 이 헬퍼는 미사용.
pub fn split_downloadable(libs: Vec<Library>) -> (Vec<Library>, Vec<Library>) {
    let mut downloadable = Vec::new();
    let mut local_only = Vec::new();
    for l in libs {
        if is_downloadable(&l) {
            downloadable.push(l);
        } else {
            local_only.push(l);
        }
    }
    (downloadable, local_only)
}

#[allow(dead_code)]
fn is_downloadable(l: &Library) -> bool {
    match &l.downloads {
        Some(LibraryDownloads {
            artifact: Some(DownloadArtifact { url, .. }),
            ..
        }) => !url.is_empty(),
        _ => false,
    }
}

// ─────────────────────── ForgeWrapper ───────────────────────

// ─────────────────────── 헤드리스 설치 ───────────────────────

/// NeoForge installer 를 `--installClient` 모드로 실행해 processors 를 돌린다.
///
/// installer 동작:
///   - `<game_dir>/launcher_profiles.json` 이 존재해야 한다 (형식만 맞으면 내용 무관).
///   - `<game_dir>/libraries/...` 에 부족한 라이브러리를 다운로드 · 패치 생성.
///   - `<game_dir>/versions/neoforge-<ver>/...` 에 version.json 과 dummy jar 생성.
///
/// 결과적으로 `LibraryPlan` 에서 url 이 비어있던 항목들의 실제 파일이
/// 디스크에 배치된다.
pub async fn install_client(
    installer: &Path,
    game_dir: &Path,
    java: &Path,
    mut on_line: impl FnMut(&str) + Send,
) -> Result<()> {
    ensure_launcher_profiles(game_dir).await?;

    // NeoForge installer 자체는 우리가 넘긴 java 로 실행되지만, installer 가
    // 내부적으로 자식 java 프로세스를 spawn 할 때 시스템 PATH 의 java 를
    // 사용하는 경우가 있다. 시스템 PATH 의 첫 java 가 Oracle Java 8 (구식)
    // 이라면 NeoForge 21.x (Java 21 필요) 와 호환 안 되어 hang 발생.
    // → JAVA_HOME 과 PATH 를 번들 JDK 21 으로 강제해 자식이 같은 java 를 쓰게 함.
    let java_bin = java
        .parent()
        .ok_or_else(|| anyhow!("java 실행파일의 bin 디렉토리를 결정할 수 없음: {}", java.display()))?;
    let java_home = java_bin
        .parent()
        .ok_or_else(|| anyhow!("java 의 home 디렉토리를 결정할 수 없음: {}", java_bin.display()))?;
    let new_path = match std::env::var_os("PATH") {
        Some(orig) => {
            let mut s = std::ffi::OsString::from(java_bin);
            s.push(";");
            s.push(orig);
            s
        }
        None => std::ffi::OsString::from(java_bin),
    };

    tracing::info!(
        installer = %installer.display(),
        game_dir = %game_dir.display(),
        java = %java.display(),
        java_home = %java_home.display(),
        "NeoForge installer --installClient 실행",
    );

    let mut child = tokio::process::Command::new(java)
        .arg("-jar")
        .arg(installer)
        .arg("--installClient")
        .arg(game_dir)
        .current_dir(game_dir)
        .env("JAVA_TOOL_OPTIONS", "")
        .env("JAVA_HOME", java_home)
        .env("PATH", &new_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("NeoForge installer 프로세스 실행 실패")?;

    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");

    // stderr 는 별도 태스크로 흘려보내 deadlock 방지.
    let stderr_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        let mut tail = Vec::<String>::new();
        while let Ok(Some(line)) = reader.next_line().await {
            // installer 는 정보 로그도 stderr 에 찍는 경우가 있음 — 마지막 N줄만 보관.
            tail.push(line);
            if tail.len() > 50 {
                tail.remove(0);
            }
        }
        tail.join("\n")
    });

    let mut stdout_reader = BufReader::new(stdout).lines();
    while let Some(line) = stdout_reader
        .next_line()
        .await
        .context("installer stdout 읽기 실패")?
    {
        let trimmed = line.trim_end();
        if !trimmed.is_empty() {
            tracing::debug!(target: "neoforge_installer", "{}", trimmed);
            on_line(trimmed);
        }
    }

    let status = child.wait().await.context("installer 종료 대기 실패")?;
    let stderr_tail = stderr_task.await.unwrap_or_default();

    if !status.success() {
        anyhow::bail!(
            "NeoForge installer 비정상 종료 (exit={:?})\n--- stderr (마지막 50줄) ---\n{}",
            status.code(),
            stderr_tail,
        );
    }
    tracing::info!("NeoForge installer 완료");
    Ok(())
}

/// installer 가 요구하는 `launcher_profiles.json` 을 최소 형태로 생성.
async fn ensure_launcher_profiles(game_dir: &Path) -> Result<()> {
    tokio::fs::create_dir_all(game_dir).await.ok();
    let path = game_dir.join("launcher_profiles.json");
    if path.exists() {
        return Ok(());
    }
    // Vanilla launcher 와 동일 형식의 최소 JSON — installer 는 `profiles` 키만 확인.
    let stub = r#"{"profiles":{},"settings":{},"version":3}"#;
    tokio::fs::write(&path, stub)
        .await
        .with_context(|| format!("launcher_profiles.json 생성 실패: {}", path.display()))?;
    tracing::debug!(path = %path.display(), "launcher_profiles.json stub 생성");
    Ok(())
}

/// Prism 이 사용하는 ForgeWrapper 최신 릴리스.
/// ForgeWrapper.Main 이 installer jar 를 파싱해 processors 를 실행하므로,
/// 런처에서는 이 jar 를 classpath 에 올리고 mainClass 만 교체하면 된다.
///
/// 실제 NeoForge 21.1.220 의 version.json 은 이미 적절한 mainClass 를
/// 지정하므로 대부분의 경우 ForgeWrapper 가 없어도 동작한다 — 이 상수는
/// fallback 용.
#[allow(dead_code)]
pub const FORGEWRAPPER_MAVEN_URL: &str =
    "https://github.com/ZekerZhayard/ForgeWrapper/releases/download/1.6.0/ForgeWrapper-1.6.0.jar";
#[allow(dead_code)]
pub const FORGEWRAPPER_MAIN: &str = "io.github.zekerzhayard.forgewrapper.installer.Main";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coord_key_strips_version() {
        assert_eq!(maven_coord_key("org.ow2.asm:asm:9.6"), "org.ow2.asm:asm");
        assert_eq!(
            maven_coord_key("org.lwjgl:lwjgl:3.3.3:natives-windows"),
            "org.lwjgl:lwjgl"
        );
        assert_eq!(maven_coord_key("singlepart"), "singlepart");
    }

    #[test]
    fn installer_url_format() {
        assert_eq!(
            installer_url("21.1.220"),
            "https://maven.neoforged.net/releases/net/neoforged/neoforge/21.1.220/neoforge-21.1.220-installer.jar"
        );
    }
}
