//! MSA device code flow + Xbox/Minecraft 토큰 교환 체인.
//!
//! 외부 의존 없이 reqwest + serde 로만 구현 — Microsoft 공식 엔드포인트만 사용.
//!
//! Azure app 등록 필요:
//!   1. portal.azure.com → Microsoft Entra ID → App registrations → New
//!   2. "Accounts in any Microsoft account" 선택
//!   3. Redirect URI 는 비워둠 (device code flow 는 불필요)
//!   4. Manifest 에서 `allowPublicClient: true` 로 설정
//!   5. 발급된 Application (client) ID 를 `MSA_CLIENT_ID` 에 기입
//!
//! 환경변수 `CHERISHWORLD_MSA_CLIENT_ID` 가 있으면 상수를 override.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Azure 앱 Client ID. 빌드 시 `CHERISHWORLD_MSA_CLIENT_ID` env 로도 override.
///
/// ⚠ 실제 배포 전 등록한 GUID 로 교체 필수.
pub const MSA_CLIENT_ID_DEFAULT: &str = "8b7d5ae0-9d41-4f74-a826-ee411f546f42";

pub fn client_id() -> String {
    std::env::var("CHERISHWORLD_MSA_CLIENT_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| MSA_CLIENT_ID_DEFAULT.to_string())
}

// 개인 Microsoft 계정용 엔드포인트.
// ⚠ 이 코드 경로(MSA device flow) 는 현재 배포에 사용되지 않음. Mojang 이 2024년부터
//    신규 Azure client_id 에 대한 검증을 강화해 "Invalid app registration" 으로 거부하기
//    때문에, 실제 사용자 배포는 Prism Launcher 를 사용하는 것으로 결정 (2026-04-22).
//    이 모듈은 향후 Mojang 정책 완화 또는 allowlist 승인 후 재활성화 용으로 유지.
const DEVICE_CODE_URL: &str =
    "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const XBL_AUTH_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_AUTH_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MC_XBOX_LOGIN_URL: &str =
    "https://api.minecraftservices.com/authentication/login_with_xbox";
const MC_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";
const SCOPE: &str = "XboxLive.signin offline_access";

// ─────────────────────── 공개 타입 ───────────────────────

/// device code 첫 응답 — 사용자가 브라우저에서 코드를 입력하도록 안내할 때 필요.
#[derive(Debug, Clone)]
pub struct DeviceChallenge {
    pub user_code: String,
    pub verification_uri: String,
    /// Microsoft 가 제공하는 로컬라이즈드 안내 문구 — UI 가 자체 문구를 쓰면 미사용.
    #[allow(dead_code)]
    pub message: String,
    pub expires_in: u64,
    /// 내부적으로 polling 시 사용.
    pub(crate) device_code: String,
    pub(crate) interval: u64,
}

/// 인증 성공 시 Minecraft 서비스로부터 받은 최종 토큰 + 프로필.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Authenticated {
    /// Minecraft 서비스 bearer — 런치 시 `--accessToken` 으로 전달.
    pub mc_access_token: String,
    /// 만료 epoch 초 — 지나면 refresh 필요.
    pub mc_access_expires_at: i64,
    /// 영구 갱신용 — 브라우저 재인증 없이 access_token 재발급.
    pub ms_refresh_token: String,
    pub profile: Profile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    /// 대시 없는 hex (예: "069a79f4..."). Mojang 포맷.
    pub id: String,
    pub name: String,
}

impl Profile {
    /// 런처 인자에 넣을 대시 포함 UUID.
    pub fn uuid_dashed(&self) -> String {
        if self.id.len() != 32 {
            return self.id.clone();
        }
        format!(
            "{}-{}-{}-{}-{}",
            &self.id[0..8],
            &self.id[8..12],
            &self.id[12..16],
            &self.id[16..20],
            &self.id[20..32]
        )
    }
}

// ─────────────────────── 흐름 1: device code 시작 ───────────────────────

pub async fn start_device_flow() -> Result<DeviceChallenge> {
    #[derive(Deserialize)]
    struct Resp {
        user_code: String,
        device_code: String,
        verification_uri: String,
        expires_in: u64,
        interval: u64,
        message: String,
    }

    let cid = client_id();
    ensure_client_id_configured(&cid)?;

    let client = http_client()?;
    let resp: Resp = client
        .post(DEVICE_CODE_URL)
        .form(&[("client_id", cid.as_str()), ("scope", SCOPE)])
        .send()
        .await
        .context("device code 요청 실패")?
        .error_for_status()
        .context("device code 응답 오류")?
        .json()
        .await?;

    Ok(DeviceChallenge {
        user_code: resp.user_code,
        device_code: resp.device_code,
        verification_uri: resp.verification_uri,
        expires_in: resp.expires_in,
        interval: resp.interval.max(1),
        message: resp.message,
    })
}

// ─────────────────────── 흐름 2: polling → MS access_token ───────────────────────

pub async fn poll_for_ms_token(challenge: &DeviceChallenge) -> Result<MsTokens> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(challenge.expires_in);
    let client = http_client()?;
    let cid = client_id();

    loop {
        tokio::time::sleep(Duration::from_secs(challenge.interval)).await;
        if tokio::time::Instant::now() >= deadline {
            bail!("device code 만료 — 다시 시도하세요");
        }

        let resp = client
            .post(TOKEN_URL)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", cid.as_str()),
                ("device_code", challenge.device_code.as_str()),
            ])
            .send()
            .await?;

        let status = resp.status();
        let body: serde_json::Value = resp.json().await?;

        if status.is_success() {
            return Ok(MsTokens {
                access_token: body
                    .get("access_token")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("MS access_token 없음"))?
                    .to_string(),
                refresh_token: body
                    .get("refresh_token")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("MS refresh_token 없음"))?
                    .to_string(),
            });
        }

        let err_code = body.get("error").and_then(|v| v.as_str()).unwrap_or("");
        match err_code {
            // 아직 유저가 입력 안 함 — 계속 polling
            "authorization_pending" => continue,
            "slow_down" => {
                // interval + 5초 권장
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
            "authorization_declined" => bail!("사용자가 인증을 거부했습니다"),
            "expired_token" | "code_expired" => bail!("device code 만료"),
            _ => bail!("MS 토큰 교환 실패: {}", body),
        }
    }
}

pub struct MsTokens {
    pub access_token: String,
    pub refresh_token: String,
}

// ─────────────────────── 흐름 3: refresh_token → 새 access_token ───────────────────────

pub async fn refresh_ms_token(refresh_token: &str) -> Result<MsTokens> {
    let cid = client_id();
    ensure_client_id_configured(&cid)?;

    let resp: serde_json::Value = http_client()?
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", cid.as_str()),
            ("refresh_token", refresh_token),
            ("scope", SCOPE),
        ])
        .send()
        .await
        .context("refresh_token 요청 실패")?
        .error_for_status()
        .context("refresh_token 거부됨 — 재로그인 필요")?
        .json()
        .await?;

    Ok(MsTokens {
        access_token: resp
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("refreshed access_token 없음"))?
            .to_string(),
        refresh_token: resp
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .unwrap_or(refresh_token)
            .to_string(),
    })
}

// ─────────────────────── 흐름 4: MS → XBL → XSTS → Minecraft ───────────────────────

pub async fn exchange_to_minecraft(ms_access_token: &str) -> Result<(String, i64)> {
    let xbl = xbl_auth(ms_access_token).await?;
    let (xsts_token, userhash) = xsts_auth(&xbl).await?;
    mc_login(&xsts_token, &userhash).await
}

async fn xbl_auth(ms_token: &str) -> Result<String> {
    let body = serde_json::json!({
        "Properties": {
            "AuthMethod": "RPS",
            "SiteName": "user.auth.xboxlive.com",
            "RpsTicket": format!("d={}", ms_token),
        },
        "RelyingParty": "http://auth.xboxlive.com",
        "TokenType": "JWT",
    });
    let resp: serde_json::Value = http_client()?
        .post(XBL_AUTH_URL)
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await
        .context("XBL 인증 요청 실패")?
        .error_for_status()
        .context("XBL 인증 거부")?
        .json()
        .await?;

    resp.get("Token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("XBL 응답에 Token 없음"))
}

async fn xsts_auth(xbl_token: &str) -> Result<(String, String)> {
    let body = serde_json::json!({
        "Properties": {
            "SandboxId": "RETAIL",
            "UserTokens": [xbl_token],
        },
        "RelyingParty": "rp://api.minecraftservices.com/",
        "TokenType": "JWT",
    });
    let resp = http_client()?
        .post(XSTS_AUTH_URL)
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await
        .context("XSTS 인증 요청 실패")?;

    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        let err: serde_json::Value = resp.json().await?;
        let xerr = err.get("XErr").and_then(|v| v.as_i64()).unwrap_or(0);
        let msg = match xerr {
            2148916233 => "이 계정은 Xbox Live 프로필이 없습니다 — xbox.com 에서 먼저 프로필 생성",
            2148916235 => "Xbox Live 이용 불가 지역",
            2148916236 | 2148916237 => "성인 인증 필요",
            2148916238 => "미성년자 계정 — 부모 동의 후 Family 등록 필요",
            _ => "Xbox Live 인증 거부",
        };
        bail!("XSTS 실패 (XErr={}): {}", xerr, msg);
    }

    let json: serde_json::Value = resp.error_for_status()?.json().await?;
    let token = json
        .get("Token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("XSTS 응답에 Token 없음"))?
        .to_string();
    let userhash = json
        .get("DisplayClaims")
        .and_then(|d| d.get("xui"))
        .and_then(|xui| xui.get(0))
        .and_then(|u| u.get("uhs"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("XSTS 응답에 userhash(uhs) 없음"))?
        .to_string();
    Ok((token, userhash))
}

async fn mc_login(xsts_token: &str, userhash: &str) -> Result<(String, i64)> {
    let body = serde_json::json!({
        "identityToken": format!("XBL3.0 x={};{}", userhash, xsts_token),
    });
    let resp = http_client()?
        .post(MC_XBOX_LOGIN_URL)
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await
        .context("Minecraft login_with_xbox 실패")?;

    let status = resp.status();
    // 에러 시 본문까지 보존해 실제 원인을 찾을 수 있도록.
    if !status.is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        tracing::error!(
            status = status.as_u16(),
            body = %body_text,
            "login_with_xbox HTTP 에러 — 전체 응답",
        );
        if status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::UNAUTHORIZED {
            bail!(
                "Minecraft 서비스 접근 거부 (HTTP {}). 응답 본문:\n{}",
                status.as_u16(),
                if body_text.is_empty() { "(비어있음)" } else { &body_text }
            );
        }
        bail!("login_with_xbox HTTP {} — 본문: {}", status.as_u16(), body_text);
    }

    let json: serde_json::Value = resp.json().await?;
    let token = json
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("MC access_token 없음"))?
        .to_string();
    let expires_in = json.get("expires_in").and_then(|v| v.as_i64()).unwrap_or(86400);
    Ok((token, expires_in))
}

pub async fn fetch_profile(mc_access_token: &str) -> Result<Profile> {
    let resp = http_client()?
        .get(MC_PROFILE_URL)
        .bearer_auth(mc_access_token)
        .send()
        .await
        .context("MC profile 조회 실패")?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        bail!("이 Microsoft 계정으로 Minecraft: Java Edition 을 구매하지 않았습니다");
    }
    let json: Profile = resp.error_for_status()?.json().await?;
    Ok(json)
}

// ─────────────────────── 상위 API ───────────────────────

/// 전체 체인: device challenge 표시 → polling → Minecraft 토큰 + 프로필.
///
/// `on_challenge` 콜백으로 user_code / URL 을 호출자(CLI/GUI)에 전달.
pub async fn login(
    mut on_challenge: impl FnMut(&DeviceChallenge) + Send,
) -> Result<Authenticated> {
    let challenge = start_device_flow().await?;
    on_challenge(&challenge);

    let ms = poll_for_ms_token(&challenge).await?;
    let (mc_token, mc_expires_in) = exchange_to_minecraft(&ms.access_token).await?;
    let profile = fetch_profile(&mc_token).await?;

    Ok(Authenticated {
        mc_access_token: mc_token,
        mc_access_expires_at: now_epoch() + mc_expires_in,
        ms_refresh_token: ms.refresh_token,
        profile,
    })
}

/// refresh_token 으로 access_token 갱신 (브라우저 상호작용 없음).
pub async fn refresh(refresh_token: &str) -> Result<Authenticated> {
    let ms = refresh_ms_token(refresh_token).await?;
    let (mc_token, mc_expires_in) = exchange_to_minecraft(&ms.access_token).await?;
    let profile = fetch_profile(&mc_token).await?;

    Ok(Authenticated {
        mc_access_token: mc_token,
        mc_access_expires_at: now_epoch() + mc_expires_in,
        ms_refresh_token: ms.refresh_token,
        profile,
    })
}

// ─────────────────────── 보조 ───────────────────────

fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(concat!("CherishWorld-Launcher/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(Into::into)
}

fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 미설정 placeholder — 과거에 상수 기본값이었던 GUID 패턴.
const MSA_CLIENT_ID_PLACEHOLDER: &str = "00000000-0000-0000-0000-000000000000";

fn ensure_client_id_configured(cid: &str) -> Result<()> {
    if cid == MSA_CLIENT_ID_PLACEHOLDER || cid.is_empty() {
        bail!(
            "MSA client_id 가 미설정 상태입니다. Azure portal 에서 app registration 후 \
             `CHERISHWORLD_MSA_CLIENT_ID` 환경변수로 지정하거나 \
             `launcher::auth::msa::MSA_CLIENT_ID_DEFAULT` 상수를 수정하세요."
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_uuid_dashed() {
        let p = Profile {
            id: "069a79f444e94726a5befca90e38aaf5".into(),
            name: "Test".into(),
        };
        assert_eq!(p.uuid_dashed(), "069a79f4-44e9-4726-a5be-fca90e38aaf5");
    }

    #[test]
    fn profile_uuid_dashed_already_dashed() {
        let p = Profile {
            id: "invalid".into(),
            name: "Test".into(),
        };
        assert_eq!(p.uuid_dashed(), "invalid");
    }
}
