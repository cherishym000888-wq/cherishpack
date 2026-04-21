//! 바탕화면 바로가기 (.lnk) 생성.
//!
//! IShellLink COM 을 직접 쓰는 대신, PowerShell + WScript.Shell 로 한 줄.
//! 이미 같은 이름 파일이 있으면 덮어씀.

use anyhow::Result;
use std::path::Path;

pub fn create_desktop_shortcut(
    name: &str,
    target_exe: &Path,
    args: &str,
    working_dir: &Path,
    icon_path: Option<&Path>,
) -> Result<()> {
    let desktop = desktop_dir().ok_or_else(|| anyhow::anyhow!("바탕화면 경로를 찾을 수 없음"))?;
    let lnk = desktop.join(format!("{name}.lnk"));
    create_lnk_at(&lnk, target_exe, args, working_dir, icon_path)
}

/// 시작 메뉴 바로가기.
pub fn create_startmenu_shortcut(
    name: &str,
    target_exe: &Path,
    args: &str,
    working_dir: &Path,
    icon_path: Option<&Path>,
) -> Result<()> {
    let sm = startmenu_dir().ok_or_else(|| anyhow::anyhow!("시작메뉴 경로를 찾을 수 없음"))?;
    std::fs::create_dir_all(&sm).ok();
    let lnk = sm.join(format!("{name}.lnk"));
    create_lnk_at(&lnk, target_exe, args, working_dir, icon_path)
}

fn create_lnk_at(
    lnk: &Path,
    target_exe: &Path,
    args: &str,
    working_dir: &Path,
    icon_path: Option<&Path>,
) -> Result<()> {

    let icon_spec = match icon_path {
        Some(p) => format!("{},0", p.to_string_lossy()),
        None => format!("{},0", target_exe.to_string_lossy()),
    };

    // PowerShell 인용 이슈 피하려고 base64로 스크립트 전달
    let ps_script = format!(
        r#"
$s = (New-Object -ComObject WScript.Shell).CreateShortcut({lnk})
$s.TargetPath = {target}
$s.Arguments = {args}
$s.WorkingDirectory = {wd}
$s.IconLocation = {icon}
$s.Description = 'CherishPack'
$s.Save()
"#,
        lnk = ps_quote(lnk.to_string_lossy().as_ref()),
        target = ps_quote(target_exe.to_string_lossy().as_ref()),
        args = ps_quote(args),
        wd = ps_quote(working_dir.to_string_lossy().as_ref()),
        icon = ps_quote(&icon_spec),
    );

    let encoded = to_utf16_base64(&ps_script);

    let status = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-EncodedCommand",
            &encoded,
        ])
        .status()?;

    if !status.success() {
        anyhow::bail!("바로가기 생성 실패 (exit {:?})", status.code());
    }
    Ok(())
}

/// PowerShell single-quoted 문자열 — 내부 ' 는 '' 로 이스케이프
fn ps_quote(s: &str) -> String {
    let escaped = s.replace('\'', "''");
    format!("'{escaped}'")
}

/// PowerShell -EncodedCommand 규격: UTF-16LE → base64
fn to_utf16_base64(s: &str) -> String {
    use base64_std::Engine;
    let utf16: Vec<u16> = s.encode_utf16().collect();
    let mut bytes = Vec::with_capacity(utf16.len() * 2);
    for u in utf16 {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    base64_std::engine::general_purpose::STANDARD.encode(&bytes)
}

#[cfg(windows)]
fn desktop_dir() -> Option<std::path::PathBuf> {
    // USERPROFILE\Desktop — OneDrive 리다이렉트 시 맞지 않을 수 있지만 대부분 OK
    std::env::var_os("USERPROFILE").map(|p| std::path::PathBuf::from(p).join("Desktop"))
}

#[cfg(not(windows))]
fn desktop_dir() -> Option<std::path::PathBuf> {
    None
}

#[cfg(windows)]
fn startmenu_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("APPDATA").map(|p| {
        std::path::PathBuf::from(p)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
    })
}

#[cfg(not(windows))]
fn startmenu_dir() -> Option<std::path::PathBuf> {
    None
}

// base64 재노출 — reqwest/rustls 가 이미 base64 를 가져오지만 경로가 불안정하므로 직접.
mod base64_std {
    pub use ::base64::engine;
    pub use ::base64::Engine;
}
