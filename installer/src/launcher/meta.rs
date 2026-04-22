//! Mojang piston-meta 파싱.
//!
//! 1) version_manifest_v2.json 에서 원하는 버전(id)의 URL·sha1 을 찾고
//! 2) 해당 version.json 을 받아 런치에 필요한 필드(mainClass / libraries /
//!    assetIndex / downloads.client / arguments / javaVersion)만 역직렬화한다.
//!
//! piston-meta 는 스키마가 버전에 따라 미묘하게 다르므로 전부 Option 으로
//! 선언하고 실패 없이 스킵·로깅한다. (예: 1.12.2 이하는 arguments 대신
//! `minecraftArguments` 문자열을 쓰지만 우리는 1.21.1 만 타겟이므로 arguments 만 본다.)

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Mojang 공식 manifest (v2 — sha1 포함).
pub const VERSION_MANIFEST_URL: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

// ─────────────────────── manifest ───────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionManifest {
    pub latest: LatestVersions,
    pub versions: Vec<ManifestEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatestVersions {
    pub release: String,
    pub snapshot: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
    /// version.json 파일의 sha1 — 무결성 검증에 사용.
    pub sha1: String,
    #[serde(default)]
    pub time: Option<String>,
    #[serde(rename = "releaseTime", default)]
    pub release_time: Option<String>,
}

// ─────────────────────── version.json ───────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionMeta {
    pub id: String,
    #[serde(rename = "mainClass")]
    pub main_class: String,
    #[serde(rename = "type", default)]
    pub kind: Option<String>,

    #[serde(rename = "assetIndex")]
    pub asset_index: AssetIndexRef,

    /// 17, 21, … (1.21.1 기준 21)
    #[serde(rename = "javaVersion", default)]
    pub java_version: Option<JavaVersion>,

    pub downloads: Downloads,

    #[serde(default)]
    pub libraries: Vec<Library>,

    /// 1.13+ : 분리된 game/jvm 인자.
    #[serde(default)]
    pub arguments: Option<Arguments>,

    /// 1.12 이하 호환용 (참고만).
    #[serde(rename = "minecraftArguments", default)]
    pub legacy_minecraft_arguments: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetIndexRef {
    pub id: String,
    pub sha1: String,
    pub size: u64,
    #[serde(rename = "totalSize", default)]
    pub total_size: Option<u64>,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JavaVersion {
    #[serde(default)]
    pub component: Option<String>,
    #[serde(rename = "majorVersion")]
    pub major_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Downloads {
    pub client: DownloadArtifact,
    #[serde(default)]
    pub client_mappings: Option<DownloadArtifact>,
    #[serde(default)]
    pub server: Option<DownloadArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadArtifact {
    pub sha1: String,
    pub size: u64,
    pub url: String,
    /// libraries 쪽 artifact 에서만 나타남.
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Library {
    pub name: String,
    #[serde(default)]
    pub downloads: Option<LibraryDownloads>,
    #[serde(default)]
    pub rules: Option<Vec<Rule>>,
    /// 구 natives 표기 — "natives-windows" 등의 분류자.
    #[serde(default)]
    pub natives: Option<std::collections::HashMap<String, String>>,
    #[serde(default)]
    pub extract: Option<ExtractSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryDownloads {
    #[serde(default)]
    pub artifact: Option<DownloadArtifact>,
    /// 구 스키마(natives 분리 jar) 대응.
    #[serde(default)]
    pub classifiers: Option<std::collections::HashMap<String, DownloadArtifact>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub action: String, // "allow" | "disallow"
    #[serde(default)]
    pub os: Option<OsConstraint>,
    #[serde(default)]
    pub features: Option<std::collections::HashMap<String, bool>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OsConstraint {
    #[serde(default)]
    pub name: Option<String>, // "windows" | "osx" | "linux"
    #[serde(default)]
    pub arch: Option<String>, // "x86" | "x64" | "arm64"
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractSpec {
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Arguments {
    #[serde(default)]
    pub game: Vec<ArgEntry>,
    #[serde(default)]
    pub jvm: Vec<ArgEntry>,
}

/// `"--foo"` 같은 단순 문자열과 `{rules, value}` 오브젝트 둘 다 허용.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ArgEntry {
    Simple(String),
    Conditional {
        #[serde(default)]
        rules: Vec<Rule>,
        value: ArgValue,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ArgValue {
    One(String),
    Many(Vec<String>),
}

// ─────────────────────── API ───────────────────────

/// 전체 매니페스트를 가져온다.
pub async fn fetch_manifest() -> Result<VersionManifest> {
    crate::net::fetch_json::<VersionManifest>(VERSION_MANIFEST_URL)
        .await
        .context("Mojang version_manifest_v2.json 다운로드 실패")
}

/// 매니페스트에서 지정 버전 항목을 찾는다.
pub fn find_version<'a>(manifest: &'a VersionManifest, id: &str) -> Result<&'a ManifestEntry> {
    manifest
        .versions
        .iter()
        .find(|v| v.id == id)
        .ok_or_else(|| anyhow!("매니페스트에서 버전을 찾을 수 없음: {}", id))
}

/// 지정 버전의 version.json 을 받아 sha1 검증 후 역직렬화한다.
pub async fn fetch_version_meta(entry: &ManifestEntry) -> Result<VersionMeta> {
    let bytes = crate::net::fetch_bytes(&entry.url)
        .await
        .with_context(|| format!("version.json 다운로드 실패: {}", entry.url))?;

    // sha1 검증 — manifest v2 에만 sha1 이 있다.
    use sha1::{Digest, Sha1};
    let mut h = Sha1::new();
    h.update(&bytes);
    let got = hex::encode(h.finalize());
    if !got.eq_ignore_ascii_case(&entry.sha1) {
        bail!(
            "version.json sha1 불일치: expected={}, got={}, id={}",
            entry.sha1,
            got,
            entry.id
        );
    }

    let meta: VersionMeta = serde_json::from_slice(&bytes)
        .with_context(|| format!("version.json 파싱 실패 ({})", entry.id))?;

    if meta.id != entry.id {
        tracing::warn!(
            manifest_id = %entry.id,
            version_json_id = %meta.id,
            "manifest id 와 version.json id 불일치 — 계속 진행",
        );
    }
    Ok(meta)
}

/// 편의 함수: manifest → entry → meta 한번에.
pub async fn load(id: &str) -> Result<VersionMeta> {
    let manifest = fetch_manifest().await?;
    let entry = find_version(&manifest, id)?;
    fetch_version_meta(entry).await
}

// ─────────────────────── 테스트 ───────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arg_entry_parses_simple_and_conditional() {
        // 단순 문자열
        let s: ArgEntry = serde_json::from_str(r#""--username""#).unwrap();
        matches!(s, ArgEntry::Simple(_));

        // 조건부 (value 가 문자열)
        let c1: ArgEntry = serde_json::from_str(
            r#"{"rules":[{"action":"allow","os":{"name":"windows"}}],"value":"-XX:+UseG1GC"}"#,
        )
        .unwrap();
        matches!(c1, ArgEntry::Conditional { .. });

        // 조건부 (value 가 배열)
        let c2: ArgEntry = serde_json::from_str(
            r#"{"rules":[{"action":"allow"}],"value":["--demo","--width"]}"#,
        )
        .unwrap();
        matches!(c2, ArgEntry::Conditional { .. });
    }
}
