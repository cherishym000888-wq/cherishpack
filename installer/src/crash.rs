//! 크래시 리포트 수집·업로드.
//!
//! Phase 3 구현. 정책:
//!   - Prism 인스턴스의 `crash-reports/` 폴더 스캔
//!   - 사용자 동의 후에만 업로드
//!   - 업로드 전 **사용자명 마스킹** (C:\Users\XXX\ → C:\Users\<user>\)
//!   - 엔드포인트: POST /api/crash-report (multipart, ≤ 1MB)
//!   - 서버(138.2.127.45) + Cloudflare 앞단, 7일 보관

use anyhow::Result;

pub async fn collect_and_upload(_consent: bool) -> Result<usize> {
    Ok(0)
}

/// Windows 사용자명 마스킹 — 간단한 문자열 치환.
pub fn mask_username(text: &str, username: &str) -> String {
    if username.is_empty() {
        return text.to_string();
    }
    text.replace(username, "<user>")
}
