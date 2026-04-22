//! LWJGL natives 추출.
//!
//! 1.21.1 의 natives 라이브러리는 maven coord 에 `:natives-windows` 분류자가
//! 붙은 평범한 jar 로 들어오고, 내부에 `.dll` 파일들이 들어있다. 런치 시
//! `-Djava.library.path=<natives_dir>` 로 지정해야 하므로 jar 내 .dll 들을
//! 평탄하게 풀어넣는다. META-INF 같은 메타 파일은 제외.
//!
//! 구(舊) 스키마의 `extract.exclude` 도 존중한다.

use anyhow::{Context, Result};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use super::libraries::LibraryEntry;
use super::meta::VersionMeta;

/// natives 라이브러리만 골라낸다 — 이름에 `:natives-` 를 포함하거나
/// version.json 의 `natives` 맵에 매핑된 항목.
pub fn filter_natives<'a>(
    plan_entries: &'a [LibraryEntry],
    meta: &VersionMeta,
) -> Vec<&'a LibraryEntry> {
    // 구스키마용: natives 맵을 가진 라이브러리의 path 집합 — 현재 1.21.1 은 비어있음.
    let old_style_paths: std::collections::HashSet<String> = meta
        .libraries
        .iter()
        .filter(|l| l.natives.is_some())
        .filter_map(|l| {
            l.downloads.as_ref().and_then(|d| {
                d.classifiers.as_ref().and_then(|c| {
                    c.values().filter_map(|a| a.path.clone()).next()
                })
            })
        })
        .collect();

    plan_entries
        .iter()
        .filter(|e| {
            // 파일 경로 기반 매칭 (jar 파일명에 natives 들어감)
            let s = e.local_path.to_string_lossy();
            s.contains("natives-windows") || old_style_paths.iter().any(|p| s.ends_with(p.as_str()))
        })
        .collect()
}

/// natives jar 들을 풀어서 `natives_dir` 에 배치.
///
/// 이미 풀린 파일이 있으면 크기만 비교해 덮어씀 (해시 비교는 비용 대비 낮음).
pub fn extract_all(natives: &[&LibraryEntry], natives_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(natives_dir)
        .with_context(|| format!("natives 디렉토리 생성 실패: {}", natives_dir.display()))?;

    for entry in natives {
        extract_one(&entry.local_path, natives_dir).with_context(|| {
            format!("natives 추출 실패: {}", entry.local_path.display())
        })?;
    }
    Ok(())
}

fn extract_one(jar: &Path, natives_dir: &Path) -> Result<()> {
    let file = File::open(jar)?;
    let mut archive = zip::ZipArchive::new(file)?;

    for i in 0..archive.len() {
        let mut zf = archive.by_index(i)?;
        if zf.is_dir() {
            continue;
        }
        let name = zf.name().to_string();

        // 메타/서명/버전 정보는 skip
        if name.starts_with("META-INF/") || name.ends_with(".sha1") || name.ends_with(".git") {
            continue;
        }

        // .dll / .so / .dylib 만 추출
        let lower = name.to_ascii_lowercase();
        if !(lower.ends_with(".dll") || lower.ends_with(".so") || lower.ends_with(".dylib")) {
            continue;
        }

        // 경로 전개 — 디렉토리 구조 무시하고 파일명만 사용
        let flat_name = Path::new(&name)
            .file_name()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(&name));
        let out_path = natives_dir.join(&flat_name);

        // 크기 동일하면 skip (LWJGL dll 은 버전 동일 시 바이너리 동일)
        if let Ok(meta) = std::fs::metadata(&out_path) {
            if meta.len() == zf.size() {
                continue;
            }
        }

        let mut buf = Vec::with_capacity(zf.size() as usize);
        zf.read_to_end(&mut buf)?;
        let mut out = File::create(&out_path)
            .with_context(|| format!("natives 파일 생성 실패: {}", out_path.display()))?;
        out.write_all(&buf)?;
    }
    Ok(())
}
