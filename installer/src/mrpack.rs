//! .mrpack (Modrinth modpack) 파싱·적용.
//!
//! 포맷: zip 안에 `modrinth.index.json` + `overrides/` (+ `client-overrides/`)
//! index.json 의 `files[]` 각 항목은 `downloads[]` URL 중 하나에서 받아
//! `sha1` / `sha512` 검증, `path` 에 배치.
//!
//! Phase 2에서 실제 구현.

use anyhow::Result;
use std::path::Path;

pub struct AppliedPack {
    /// 상대경로 → sha256 (current-manifest.json 저장용)
    pub files: std::collections::HashMap<String, String>,
}

pub async fn apply(
    _mrpack_path: &Path,
    _instance_minecraft_root: &Path,
) -> Result<AppliedPack> {
    anyhow::bail!("mrpack::apply — Phase 2에서 구현 예정")
}
