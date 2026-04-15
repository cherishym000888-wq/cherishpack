//! 원격/로컬 설정 스키마.
//!
//! - `VersionIndex` : GitHub Release에 올려두는 최신 버전 포인터 (`version.json`)
//! - `PackManifest` : 각 버전별 매니페스트 (`manifests/<ver>.json`)
//! - `InstallerState`: 로컬 상태 (`installer-state.json`)
//! - `CurrentManifest`: 마지막 적용 매니페스트 스냅샷 (diff 기준)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// version.json — 채널별 최신 버전 포인터.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionIndex {
    pub stable: ChannelEntry,
    #[serde(default)]
    pub beta: Option<ChannelEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelEntry {
    pub version: String,
    pub manifest_url: String,
    /// 이 버전 미만이면 실행 차단 (강제 업데이트)
    pub min_required: String,
}

/// manifests/<version>.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackManifest {
    pub pack_version: String,
    pub released_at: String,
    pub minecraft: String,
    pub loader: Loader,
    pub mrpack_url: String,
    pub mrpack_sha256: String,

    /// 패치 시 절대 삭제/덮어쓰지 않을 glob 목록
    #[serde(default)]
    pub preserve: Vec<String>,

    /// 파일별 덮어쓰기 정책
    #[serde(default)]
    pub overwrite_policy: HashMap<String, OverwritePolicy>,

    /// 하드웨어 프로파일 (low / medium / high)
    pub hw_profiles: HashMap<String, HwProfile>,

    /// servers.dat 프리셋
    #[serde(default)]
    pub server: Option<ServerPin>,

    /// 이 매니페스트를 처리할 수 있는 최소 설치 프로그램 버전
    #[serde(default)]
    pub min_installer_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Loader {
    #[serde(rename = "type")]
    pub kind: String, // "neoforge"
    pub version: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverwritePolicy {
    /// 항상 덮어씀 (서버 강제 설정)
    Always,
    /// 사용자가 수정하지 않은 경우에만 (이전 매니페스트 해시와 비교)
    IfUnchanged,
    /// 최초 1회만. 이후 사용자 소유.
    Never,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HwProfile {
    #[serde(default)]
    pub shaders: Option<String>,
    #[serde(default)]
    pub resourcepack: Option<String>,
    pub ram_mb: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerPin {
    pub name: String,
    pub ip: String,
    #[serde(default = "default_true")]
    pub pinned: bool,
}

fn default_true() -> bool {
    true
}

// ─────────────────────────────────────────────
// 로컬 상태
// ─────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstallerState {
    #[serde(default)]
    pub installed_version: Option<String>,
    #[serde(default = "default_channel")]
    pub channel: String,
    #[serde(default)]
    pub user_preset: Option<String>, // "low" / "medium" / "high"
    #[serde(default)]
    pub anon_install_id: Option<String>, // UUID (크래시 리포트용)
    #[serde(default)]
    pub crash_report_consent: Option<bool>,
}

fn default_channel() -> String {
    "stable".into()
}

/// 마지막으로 성공적으로 적용한 매니페스트의 파일 목록 스냅샷.
/// patcher가 diff를 계산할 때 이 파일을 기준으로 삼는다.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CurrentManifest {
    pub pack_version: String,
    /// 상대경로 → sha256
    pub files: HashMap<String, String>,
}
