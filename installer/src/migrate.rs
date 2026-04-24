//! 구버전 레이아웃 → 새 통합 레이아웃 이사.
//!
//! 구조:
//!   %LOCALAPPDATA%\CherishPack\**   → %LOCALAPPDATA%\CherishWorld\**          (installer 루트 통합)
//!   %LOCALAPPDATA%\CherishWorld\game\ 등 직접 구조 → %LOCALAPPDATA%\CherishWorld\launcher\  (launcher 서브폴더 격리)
//!
//! 멱등(idempotent) — 이미 이사됐거나 원래 없는 경우 no-op.

use anyhow::Result;
use std::path::{Path, PathBuf};

/// 이사 대상 서브 이름들 (launcher 전용 컨텐츠).
const LAUNCHER_ITEMS: &[&str] = &[
    "game", "instance", "java", "launch-cache.json", "boot-agent.jar",
];
/// 이사 시 우선 보존 서브 (cache/logs 는 installer 와 충돌 가능 — launcher/ 로 이사)
const LAUNCHER_SHARED_SUBS: &[&str] = &["cache", "logs", "cherishworld.ico"];

pub fn migrate() -> Result<()> {
    let Ok(local) = std::env::var("LOCALAPPDATA") else { return Ok(()) };
    let local = PathBuf::from(local);
    let old_pack = local.join("CherishPack");
    let world = local.join("CherishWorld");
    let launcher = world.join("launcher");

    // 1. CherishPack → CherishWorld 병합 (파일 단위 이동)
    if old_pack.is_dir() {
        std::fs::create_dir_all(&world).ok();
        move_contents(&old_pack, &world)?;
        // 빈 폴더 삭제 (에러 무시 — 파일이 남아있으면 실패해도 OK)
        let _ = std::fs::remove_dir(&old_pack);
        tracing::info!("이사 완료: {} → {}", old_pack.display(), world.display());
    }

    // 2. CherishWorld\game, instance 등 직접 구조 → CherishWorld\launcher\
    let needs_launcher_move = LAUNCHER_ITEMS.iter().any(|n| world.join(n).exists());
    if needs_launcher_move {
        std::fs::create_dir_all(&launcher).ok();
        for name in LAUNCHER_ITEMS {
            let src = world.join(name);
            if !src.exists() { continue; }
            let dst = launcher.join(name);
            if dst.exists() { continue; } // 이미 옮겨진 것 스킵
            if let Err(e) = std::fs::rename(&src, &dst) {
                tracing::warn!("이사 실패 (계속): {} → {}: {e}", src.display(), dst.display());
            }
        }
        // CherishWorld\cache, logs 는 launcher 가 기존에 만들던 것.
        // installer 가 cache/logs 를 쓸 예정이므로, 기존 내용은 launcher\ 쪽으로 격리.
        for name in LAUNCHER_SHARED_SUBS {
            let src = world.join(name);
            if !src.exists() { continue; }
            let dst = launcher.join(name);
            if dst.exists() {
                // 이미 있으면 내용 병합
                if src.is_dir() {
                    move_contents(&src, &dst).ok();
                    let _ = std::fs::remove_dir(&src);
                }
                continue;
            }
            let _ = std::fs::rename(&src, &dst);
        }
        tracing::info!("launcher 파일 격리: {}\\launcher\\", world.display());
    }
    Ok(())
}

fn move_contents(src: &Path, dst: &Path) -> Result<()> {
    if !src.is_dir() { return Ok(()); }
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if to.exists() {
            // 이미 있으면 디렉토리는 병합, 파일은 스킵 (기존 우선)
            if from.is_dir() && to.is_dir() {
                move_contents(&from, &to).ok();
                let _ = std::fs::remove_dir(&from);
            }
            continue;
        }
        let _ = std::fs::rename(&from, &to);
    }
    Ok(())
}
