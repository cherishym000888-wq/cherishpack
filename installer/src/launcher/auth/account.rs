//! `<dirs.root>\account.json` — MSA 계정 영속.
//!
//! 저장 내용: refresh_token + 마지막 access_token + 프로필.
//! 다음 실행에 access_token 이 만료됐으면 refresh_token 으로 갱신 시도.
//!
//! ⚠ refresh_token 은 평문 저장 — Windows 상 %LOCALAPPDATA% 는 계정별 격리
//! 되지만, 보안 요구가 높아지면 DPAPI 로 암호화 검토.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::msa::{self, Authenticated};

pub const FILE_NAME: &str = "account.json";
pub const SCHEMA_VERSION: u32 = 1;
/// 만료까지 이 시간 이상 남았으면 재사용. 이하면 선제적 refresh.
pub const EXPIRY_MARGIN_SECS: i64 = 300;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAccount {
    pub schema: u32,
    pub auth: Authenticated,
}

pub fn path(root: &Path) -> PathBuf {
    root.join(FILE_NAME)
}

pub fn save(root: &Path, auth: &Authenticated) -> Result<()> {
    let p = path(root);
    let stored = StoredAccount {
        schema: SCHEMA_VERSION,
        auth: auth.clone(),
    };
    let data = serde_json::to_vec_pretty(&stored)?;
    std::fs::write(&p, data).with_context(|| format!("account.json 쓰기 실패: {}", p.display()))?;
    // Windows ACL 은 기본값으로도 사용자 격리됨 — 추가 chmod 는 불필요.
    Ok(())
}

pub fn load(root: &Path) -> Result<Authenticated> {
    let p = path(root);
    let data = std::fs::read(&p)
        .with_context(|| format!("account.json 읽기 실패: {}", p.display()))?;
    let stored: StoredAccount = serde_json::from_slice(&data).context("account.json 파싱 실패")?;
    if stored.schema != SCHEMA_VERSION {
        anyhow::bail!(
            "account.json 스키마 불일치 (expected={}, got={}) — 재로그인 필요",
            SCHEMA_VERSION,
            stored.schema
        );
    }
    Ok(stored.auth)
}

/// 저장된 계정이 있으면 유효성 확인 후 반환 (만료 가까우면 refresh).
/// 없으면 None — 호출자가 `msa::login()` 으로 새 인증을 시작해야 함.
pub async fn load_and_refresh_if_needed(root: &Path) -> Result<Option<Authenticated>> {
    let auth = match load(root) {
        Ok(a) => a,
        Err(e) => {
            tracing::debug!(error = %e, "저장된 계정 없음 또는 로드 실패");
            return Ok(None);
        }
    };

    let now = now_epoch();
    if auth.mc_access_expires_at > now + EXPIRY_MARGIN_SECS {
        return Ok(Some(auth));
    }

    tracing::info!("access_token 만료 근접 — refresh 시도");
    match msa::refresh(&auth.ms_refresh_token).await {
        Ok(new_auth) => {
            if let Err(e) = save(root, &new_auth) {
                tracing::warn!(error = %e, "갱신된 account.json 저장 실패");
            }
            Ok(Some(new_auth))
        }
        Err(e) => {
            tracing::warn!(error = %e, "refresh 실패 — 재로그인 필요");
            Ok(None)
        }
    }
}

fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
