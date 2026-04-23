//! 자체 Minecraft 런처 모듈.
//!
//! Prism 의존성을 걷어내고 바닐라 → NeoForge → mrpack 순으로
//! 런치 파이프라인을 직접 구성한다. `--demo` 플래그 문제 회피가 목적.
//!
//! Day 1 범위:
//!   - `meta`      : Mojang piston-meta version manifest + version.json 파싱
//!
//! Day 2 이후에 `libraries`, `assets`, `natives`, `launch`, `neoforge` 추가.

pub mod assets;
pub mod auth;
pub mod cache;
pub mod dirs;
pub mod libraries;
pub mod meta;
pub mod natives;
pub mod neoforge;
pub mod orchestrator;
pub mod run;
