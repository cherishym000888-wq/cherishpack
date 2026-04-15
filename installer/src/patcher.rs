//! 매니페스트 diff → 삭제·추가·갱신.
//!
//! 삭제 안전장치 (4중):
//!   1. 삭제 후보 = 이전 매니페스트 파일 − 새 매니페스트 파일
//!   2. preserve glob 에 해당하면 제외
//!   3. 디스크에 실제로 존재해야 함
//!   4. 현재 파일 sha256 이 이전 매니페스트 해시와 일치해야만 삭제
//!      (불일치 = 사용자 수정 → 스킵 + 로그)
//!   → 최종 삭제는 휴지통 이동 (trash crate)
//!
//! Phase 2에서 실제 구현.

use anyhow::Result;

use crate::config::{CurrentManifest, PackManifest};

#[derive(Debug, Default)]
pub struct PatchPlan {
    pub to_delete: Vec<String>,
    pub to_skip_modified: Vec<String>,
    pub to_add_or_update: Vec<String>,
}

pub fn plan(
    _previous: Option<&CurrentManifest>,
    _next: &PackManifest,
    _new_files: &std::collections::HashMap<String, String>,
) -> Result<PatchPlan> {
    anyhow::bail!("patcher::plan — Phase 2에서 구현 예정")
}
