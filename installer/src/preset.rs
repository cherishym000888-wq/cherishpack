//! 하드웨어 스냅샷 → 권장 프리셋.
//!
//! 정책:
//!   - RAM < 8GB              → low
//!   - 통합 그래픽 추정        → low (RAM 무관)
//!   - VRAM < 4GB              → medium
//!   - RAM >= 16GB && VRAM>=4 → high
//!   - 그 외                   → medium
//!
//! 자동 선택이 아니라 **추천**이다. 사용자가 UI에서 최종 변경 가능.

use crate::hwdetect::HwSnapshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Low,
    Medium,
    High,
}

impl Preset {
    pub fn key(self) -> &'static str {
        match self {
            Preset::Low => "low",
            Preset::Medium => "medium",
            Preset::High => "high",
        }
    }
}

pub fn recommend(hw: &HwSnapshot) -> Preset {
    if hw.is_integrated_gpu_guess {
        return Preset::Low;
    }
    if hw.total_ram_mb > 0 && hw.total_ram_mb < 8 * 1024 {
        return Preset::Low;
    }
    if hw.gpu_vram_mb.unwrap_or(0) < 4 * 1024 {
        return Preset::Medium;
    }
    if hw.total_ram_mb >= 16 * 1024 {
        return Preset::High;
    }
    Preset::Medium
}

/// JVM 힙 자동 계산: 총 RAM의 40%, 최소 4GB, 최대 8GB.
pub fn suggest_heap_mb(total_ram_mb: u32) -> u32 {
    let v = (total_ram_mb as u64 * 40 / 100) as u32;
    v.clamp(4 * 1024, 8 * 1024)
}
