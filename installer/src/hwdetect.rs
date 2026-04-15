//! 하드웨어 감지. Phase 3 구현.
//!
//! 수집 대상:
//!   - 총 RAM (GB)
//!   - 여유 RAM
//!   - GPU 벤더 / 이름 / VRAM (WMI Win32_VideoController)
//!   - 디스플레이 해상도
//!
//! 반환값은 `preset` 모듈이 소비한다.

#[derive(Debug, Clone, Default)]
pub struct HwSnapshot {
    pub total_ram_mb: u32,
    pub available_ram_mb: u32,
    pub gpu_name: Option<String>,
    pub gpu_vram_mb: Option<u32>,
    pub is_integrated_gpu_guess: bool,
}

pub fn detect() -> HwSnapshot {
    // Phase 3에서 windows/wmi 크레이트로 실제 구현.
    HwSnapshot::default()
}
