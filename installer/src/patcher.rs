//! 매니페스트 diff → 삭제·추가·갱신.
//!
//! 삭제 안전장치 (4중):
//!   1. 삭제 후보 = 이전 매니페스트 파일 − 새 매니페스트 파일
//!   2. preserve glob (매니페스트 + 하드코드) 에 해당하면 제외
//!   3. 디스크에 실제로 존재해야 함
//!   4. 현재 파일 sha256 이 이전 매니페스트 해시와 일치해야만 삭제
//!      (불일치 = 사용자 수정 → 스킵 + 로그)
//!   → 최종 삭제는 **휴지통 이동** (trash crate), 영구 삭제 금지

use anyhow::Result;
use std::{collections::HashMap, path::Path};

use crate::{
    config::{CurrentManifest, PackManifest},
    hash,
    preserve::{self, HARDCODED_PRESERVE},
};

#[derive(Debug, Default)]
pub struct PatchPlan {
    /// 휴지통으로 이동된 파일
    pub deleted: Vec<String>,
    /// 사용자가 수정해서 보존한 파일
    pub skipped_user_modified: Vec<String>,
    /// preserve 규칙으로 보존한 파일
    pub skipped_preserved: Vec<String>,
}

/// diff → 실제 삭제 수행.
/// - `previous`: 지난 번 적용한 매니페스트 (없으면 신규 설치 → 삭제 없음)
/// - `next`: 이번에 적용할 매니페스트 (preserve 규칙 참조)
/// - `newly_applied`: 이번 mrpack 적용으로 배치된 파일 목록 (상대경로 → sha256)
/// - `minecraft_root`: 인스턴스의 minecraft 디렉터리
pub fn prune_stale_files(
    previous: Option<&CurrentManifest>,
    next: &PackManifest,
    newly_applied: &HashMap<String, String>,
    minecraft_root: &Path,
) -> Result<PatchPlan> {
    let mut plan = PatchPlan::default();

    let Some(prev) = previous else {
        tracing::info!("이전 매니페스트 없음 — 삭제 단계 스킵");
        return Ok(plan);
    };

    // preserve glob 합치기 (하드코드 + 매니페스트)
    let mut preserve_patterns: Vec<String> =
        HARDCODED_PRESERVE.iter().map(|s| s.to_string()).collect();
    preserve_patterns.extend(next.preserve.iter().cloned());

    for (rel_raw, prev_hash) in &prev.files {
        let rel = rel_raw.replace('\\', "/");

        // ① 새 매니페스트에 있으면 삭제 대상 아님
        if newly_applied.contains_key(&rel) {
            continue;
        }

        // ② preserve 규칙
        if preserve::matches_any_owned(&rel, &preserve_patterns) {
            tracing::debug!(path = %rel, "preserve 규칙으로 보존");
            plan.skipped_preserved.push(rel);
            continue;
        }

        // ③ 디스크 존재 확인
        let abs = minecraft_root.join(&rel);
        if !abs.exists() {
            // 이미 없어진 파일 — 조용히 스킵
            continue;
        }

        // ④ 해시 일치 검사 — 불일치는 사용자 수정으로 간주, 절대 안 건드림
        let current_hash = match hash::sha256_file(&abs) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(path = %abs.display(), error = %e, "해시 계산 실패 — 안전상 보존");
                plan.skipped_user_modified.push(rel);
                continue;
            }
        };
        if !current_hash.eq_ignore_ascii_case(prev_hash) {
            tracing::info!(
                path = %rel,
                "사용자 수정 감지 — 삭제 스킵"
            );
            plan.skipped_user_modified.push(rel);
            continue;
        }

        // → 휴지통 이동 (영구 삭제 금지)
        match trash::delete(&abs) {
            Ok(_) => {
                tracing::info!(path = %rel, "휴지통 이동");
                plan.deleted.push(rel);
            }
            Err(e) => {
                tracing::error!(path = %abs.display(), error = %e, "휴지통 이동 실패 — 보존");
                plan.skipped_user_modified.push(rel);
            }
        }
    }

    // 빈 디렉터리 정리 (최상위는 건드리지 않음) — 실패 무시
    cleanup_empty_dirs(minecraft_root, minecraft_root);

    Ok(plan)
}

fn cleanup_empty_dirs(root: &Path, current: &Path) {
    let Ok(entries) = std::fs::read_dir(current) else {
        return;
    };
    let mut has_any = false;
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            cleanup_empty_dirs(root, &p);
            if p.read_dir().map(|mut r| r.next().is_none()).unwrap_or(false) {
                let _ = std::fs::remove_dir(&p);
            } else {
                has_any = true;
            }
        } else {
            has_any = true;
        }
    }
    // root 자체는 지우지 않음
    let _ = has_any;
}

/// 로컬 상태 파일 I/O 헬퍼
pub fn load_current_manifest(path: &Path) -> Option<CurrentManifest> {
    let data = std::fs::read(path).ok()?;
    serde_json::from_slice(&data).ok()
}

pub fn save_current_manifest(path: &Path, cm: &CurrentManifest) -> Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).ok();
    }
    let data = serde_json::to_vec_pretty(cm)?;
    std::fs::write(path, data)?;
    Ok(())
}
