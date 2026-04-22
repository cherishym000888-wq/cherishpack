//! Microsoft 계정(MSA) 인증 — Minecraft 정품 검증.
//!
//! 기본 전제: 이 런처는 Mojang EULA 준수를 목표로 하며, 실제 MSA
//! OAuth 흐름을 거쳐 Minecraft 서비스가 발급한 access_token 을 사용한다.
//! Offline 계정은 개발자 테스트 이외에는 사용하지 않는다.
//!
//! 흐름 (device code flow — 데스크톱 앱에 적합):
//!   MS device code → MS access_token → Xbox Live (XBL) → XSTS → Minecraft
//!
//! 토큰 저장: `<dirs.root>\account.json` — refresh_token 으로 2회차부터
//! 사용자 상호작용 없이 access_token 갱신.

pub mod account;
pub mod msa;

#[cfg(feature = "offline")]
pub mod offline;
