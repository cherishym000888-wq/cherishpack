//! JVM Agent 배치 — 부팅 BGM 을 premain 에서 재생하기 위한 jar.
//!
//! exe 에 번들된 boot-agent.jar 를 지정 경로에 쓰고 `-javaagent:` 인자를 반환.
//! self-launcher 는 JVM 인자 리스트에 직접 추가, Prism 배포는 instance.cfg 의
//! `JvmArgs` 에 문자열로 이어 붙임.

use anyhow::{Context, Result};
use std::path::Path;

/// compile-time embedded agent jar bytes.
pub const AGENT_BYTES: &[u8] = include_bytes!("../resources/boot-agent.jar");

/// `dst` 에 agent jar 가 없거나 크기가 다르면 덮어씀.
pub fn ensure_installed(dst: &Path) -> Result<()> {
    let needs_write = !dst.exists()
        || std::fs::metadata(dst)
            .map(|m| m.len() as usize != AGENT_BYTES.len())
            .unwrap_or(true);
    if !needs_write {
        return Ok(());
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(dst, AGENT_BYTES)
        .with_context(|| format!("boot-agent.jar 쓰기 실패: {}", dst.display()))?;
    tracing::info!(
        "boot-agent.jar 배치 완료 ({} KB) @ {}",
        AGENT_BYTES.len() / 1024,
        dst.display()
    );
    Ok(())
}
