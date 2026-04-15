//! InstallerState 파일 I/O + 버전 비교.

use anyhow::Result;
use std::path::Path;

use crate::config::InstallerState;

pub fn load(path: &Path) -> InstallerState {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save(path: &Path, s: &InstallerState) -> Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).ok();
    }
    std::fs::write(path, serde_json::to_vec_pretty(s)?)?;
    Ok(())
}

/// 간단한 semver 유사 비교 (점 단위 숫자). 비교 불가 토큰은 문자열 비교로 폴백.
/// 반환: a < b → Less, a == b → Equal, a > b → Greater
pub fn compare(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let parse = |s: &str| -> Vec<u64> {
        s.split(|c: char| !c.is_ascii_digit())
            .filter(|t| !t.is_empty())
            .filter_map(|t| t.parse::<u64>().ok())
            .collect()
    };
    let av = parse(a);
    let bv = parse(b);
    for (x, y) in av.iter().zip(bv.iter()) {
        match x.cmp(y) {
            Ordering::Equal => continue,
            other => return other,
        }
    }
    av.len().cmp(&bv.len())
}
