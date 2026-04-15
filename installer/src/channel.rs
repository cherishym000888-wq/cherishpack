//! 릴리스 채널 (stable / beta).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Stable,
    Beta,
}

impl Channel {
    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Beta => "beta",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "beta" => Channel::Beta,
            _ => Channel::Stable,
        }
    }
}

/// version.json 이 호스팅되는 URL.
/// Phase 1 에서는 placeholder — Phase 2에 GitHub Release raw URL로 교체.
pub const VERSION_INDEX_URL: &str =
    "https://raw.githubusercontent.com/cherishym000888-wq/cherishpack/main/dist/version.json";
