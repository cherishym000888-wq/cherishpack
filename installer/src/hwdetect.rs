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

    // GPU — 별도 스레드에서 실행 (COM 초기화가 메인 스레드의 iced/winit STA 와 충돌 방지).
    // 1순위: DXGI (DedicatedVideoMemory 는 SIZE_T 64-bit → >4GB 정확)
    // 2순위: WMI (AdapterRAM 은 DWORD 32-bit → 4GB 에서 잘림)
    let gpu_result = std::thread::spawn(|| {
        if let Ok(v) = detect_gpu_dxgi() {
            return Some(v);
        }
        detect_gpu_wmi().ok()
    })
    .join()
    .ok()
    .flatten();
    if let Some((name, vram_mb)) = gpu_result {
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

/// DXGI 로 GPU/VRAM 탐지. `DedicatedVideoMemory` 는 SIZE_T(64-bit on x64) 라 >4GB도 정확.
/// 통합 GPU(Intel HD, AMD Vega 등)는 DedicatedVideoMemory 가 매우 작고 SharedSystemMemory 가 크다.
#[cfg(windows)]
fn detect_gpu_dxgi() -> anyhow::Result<(String, u32)> {
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1, DXGI_ADAPTER_DESC1,
        DXGI_ADAPTER_FLAG, DXGI_ADAPTER_FLAG_SOFTWARE,
    };
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
        let mut best: Option<(String, u64)> = None;
        let mut i = 0u32;
        loop {
            let adapter: IDXGIAdapter1 = match factory.EnumAdapters1(i) {
                Ok(a) => a,
                Err(_) => break,
            };
            i += 1;
            let desc: DXGI_ADAPTER_DESC1 = match adapter.GetDesc1() {
                Ok(d) => d,
                Err(_) => continue,
            };
            // 소프트웨어 어댑터(WARP) 제외
            if DXGI_ADAPTER_FLAG(desc.Flags as i32).0 & DXGI_ADAPTER_FLAG_SOFTWARE.0 != 0 {
                continue;
            }
            // UTF-16 → String (NUL 종료 전까지)
            let name_len = desc
                .Description
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(desc.Description.len());
            let name = String::from_utf16_lossy(&desc.Description[..name_len]);
            let vram = desc.DedicatedVideoMemory as u64;
            match &best {
                None => best = Some((name, vram)),
                Some((_, bv)) if vram > *bv => best = Some((name, vram)),
                _ => {}
            }
        }
        let (name, vram_bytes) = best.ok_or_else(|| anyhow::anyhow!("DXGI 어댑터 없음"))?;
        let vram_mb = (vram_bytes / (1024 * 1024)) as u32;
        Ok((name, vram_mb))
    }
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
