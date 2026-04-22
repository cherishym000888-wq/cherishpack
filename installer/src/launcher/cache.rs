//! 런치 캐시 — 전체 설치 1회 이후 재실행을 빠르게 만들기 위한 영속 정보.
//!
//! 저장 위치: `<dirs.root>\launch-cache.json`
//!
//! 저장 내용:
//!   - `pack_version` : 마지막으로 성공적으로 설치한 팩 버전 (업데이트 감지용)
//!   - `final_meta`   : 병합된 VersionMeta 그대로. 재런치 때 다시 합성하지 않음.
//!   - `vanilla_id`   : natives 디렉토리·바닐라 client.jar 키
//!   - `account`      : 마지막에 썼던 닉네임 (CLI 에서 override 가능)
//!
//! `--launcher` 성공 후에만 기록하고, `-l`(launch-only) 모드에서 읽는다.
//! 캐시가 없거나 파싱 실패면 `-l` 은 사용자에게 "전체 설치 먼저" 라고 안내.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

use super::meta::VersionMeta;

pub const FILE_NAME: &str = "launch-cache.json";
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
pub struct LaunchCache {
    pub schema: u32,
    pub pack_version: String,
    pub vanilla_id: String,
    pub final_meta: VersionMeta,
    #[serde(default)]
    pub account: CachedAccount,
    /// 이 팩을 설치한 채널 ("stable" | "beta"). 업데이트 체크 시 기준.
    #[serde(default = "default_channel")]
    pub channel: String,
}

fn default_channel() -> String {
    "stable".into()
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CachedAccount {
    #[serde(default)]
    pub nickname: Option<String>,
}

pub fn path(root: &Path) -> std::path::PathBuf {
    root.join(FILE_NAME)
}

pub fn save(root: &Path, cache: &LaunchCache) -> Result<()> {
    let p = path(root);
    let data = serde_json::to_vec_pretty(cache).context("launch-cache 직렬화 실패")?;
    std::fs::write(&p, data).with_context(|| format!("launch-cache 쓰기 실패: {}", p.display()))?;
    Ok(())
}

pub fn load(root: &Path) -> Result<LaunchCache> {
    let p = path(root);
    let data = std::fs::read(&p).with_context(|| format!("launch-cache 읽기 실패: {}", p.display()))?;
    let cache: LaunchCache = serde_json::from_slice(&data).context("launch-cache 파싱 실패")?;
    if cache.schema != SCHEMA_VERSION {
        anyhow::bail!(
            "launch-cache 스키마 불일치 (expected={}, got={}) — 전체 재설치 필요",
            SCHEMA_VERSION,
            cache.schema
        );
    }
    Ok(cache)
}
