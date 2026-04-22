//! ⚠ 내부 테스트 전용 — `offline` feature 로만 컴파일됨.
//!
//! MSA 없이 지정 닉네임으로 런치 가능하게 합성 계정을 만든다. 외부 배포 금지 —
//! Mojang EULA 는 정품 보유자의 플레이를 전제로 하므로 이 경로는 전용 빌드에서만 사용.

use super::msa::{Authenticated, Profile};

/// "OfflinePlayer:<nickname>" MD5 → UUIDv3 — 바닐라 런처 관행과 동일.
pub fn offline_uuid(nickname: &str) -> String {
    use md5::{Digest, Md5};
    let mut h = Md5::new();
    h.update(format!("OfflinePlayer:{}", nickname).as_bytes());
    let mut bytes: [u8; 16] = h.finalize().into();
    bytes[6] = (bytes[6] & 0x0F) | 0x30;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11],
        bytes[12], bytes[13], bytes[14], bytes[15],
    )
}

/// 합성 `Authenticated` — `user_type="legacy"`, access_token="0" 로 런치 인자를 채운다.
/// 서버가 offline-mode 일 때만 실제로 접속 가능.
pub fn synthesize(nickname: &str) -> Authenticated {
    Authenticated {
        mc_access_token: "0".to_string(),
        // 사실상 만료 체크 안 되도록 먼 미래.
        mc_access_expires_at: i64::MAX,
        ms_refresh_token: String::new(),
        profile: Profile {
            id: offline_uuid(nickname),
            name: nickname.to_string(),
        },
    }
}
