//! 하드웨어 감지.
//!
//! Phase 2 MVP: RAM 만 정확히 감지 (`GlobalMemoryStatusEx`).
//! Phase 3: GPU·VRAM 은 WMI (`Win32_VideoController`) 로 확장 예정.

#[derive(Debug, Clone, Default)]
pub struct HwSnapshot {
    pub total_ram_mb: u32,
    pub available_ram_mb: u32,
    pub gpu_name: Option<String>,
    pub gpu_vram_mb: Option<u32>,
    pub is_integrated_gpu_guess: bool,
}

#[cfg(windows)]
pub fn detect() -> HwSnapshot {
    let mut snap = HwSnapshot::default();

    // RAM — GlobalMemoryStatusEx
    unsafe {
        use windows::Win32::System::SystemInformation::{
            GlobalMemoryStatusEx, MEMORYSTATUSEX,
        };
        let mut mem = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        if GlobalMemoryStatusEx(&mut mem).is_ok() {
            snap.total_ram_mb = (mem.ullTotalPhys / (1024 * 1024)) as u32;
            snap.available_ram_mb = (mem.ullAvailPhys / (1024 * 1024)) as u32;
        }
    }

    // GPU — WMI (실패는 조용히)
    if let Ok((name, vram_mb)) = detect_gpu_wmi() {
        let n_lower = name.to_ascii_lowercase();
        snap.is_integrated_gpu_guess = n_lower.contains("intel")
            || n_lower.contains("uhd")
            || n_lower.contains("hd graphics")
            || n_lower.contains("vega")
            || n_lower.contains("radeon graphics");
        snap.gpu_name = Some(name);
        if vram_mb > 0 {
            snap.gpu_vram_mb = Some(vram_mb);
        }
    }

    snap
}

#[cfg(not(windows))]
pub fn detect() -> HwSnapshot {
    HwSnapshot::default()
}

#[cfg(windows)]
fn detect_gpu_wmi() -> anyhow::Result<(String, u32)> {
    use wmi::{COMLibrary, WMIConnection};
    let com = COMLibrary::new()?;
    let wmi = WMIConnection::new(com)?;

    #[derive(serde::Deserialize)]
    #[serde(rename = "Win32_VideoController")]
    #[serde(rename_all = "PascalCase")]
    struct Video {
        name: Option<String>,
        adapter_ram: Option<u32>, // bytes (u32 → 4GB 상한, WDDM 2.0+에선 부정확할 수 있음)
    }

    let rows: Vec<Video> = wmi.query()?;
    let pick = rows
        .into_iter()
        .filter(|v| v.name.is_some())
        .max_by_key(|v| v.adapter_ram.unwrap_or(0))
        .ok_or_else(|| anyhow::anyhow!("GPU 정보 없음"))?;
    let name = pick.name.unwrap_or_default();
    let vram_mb = pick.adapter_ram.unwrap_or(0) / (1024 * 1024);
    Ok((name, vram_mb))
}
