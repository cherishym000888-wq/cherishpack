//! Java 21 자동 감지 + 설치.
//!
//! 전략:
//!  1. `java -version` / `JAVA_HOME` / 레지스트리 / 알려진 경로로 Java 21+ 탐색
//!  2. 없으면 Microsoft OpenJDK 21 portable zip 다운로드 + 해제
//!  3. Prism instance.cfg 에 JavaPath 기록

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::net;
use crate::paths::AppDirs;

/// Java 21+ 가 필요한 최소 major 버전.
const REQUIRED_MAJOR: u32 = 21;

/// Microsoft OpenJDK 21 portable (Windows x64) 다운로드 URL.
/// aka.ms 리다이렉트 → 안정적으로 최신 21.x 제공.
const OPENJDK_URL: &str =
    "https://aka.ms/download-jdk/microsoft-jdk-21-windows-x64.zip";

/// 탐색 또는 설치한 javaw.exe 경로.
pub struct JavaResult {
    pub javaw: PathBuf,
    pub installed_now: bool,
}

pub type JavaProgress = dyn Fn(u64, Option<u64>, &str) + Send + Sync;

/// Java 21+ 를 찾거나, 없으면 설치한다.
pub async fn ensure_java(
    dirs: &AppDirs,
    progress: Option<&JavaProgress>,
) -> Result<JavaResult> {
    // 1) 기존 번들 Java (이전 설치로 이미 받아뒀을 수 있음)
    let bundled = find_bundled_java(dirs);
    if let Some(path) = bundled {
        tracing::info!(path = %path.display(), "번들 Java 발견");
        return Ok(JavaResult { javaw: path, installed_now: false });
    }

    // 2) 시스템 Java 탐색
    if let Some(path) = find_system_java() {
        tracing::info!(path = %path.display(), "시스템 Java 21+ 발견");
        return Ok(JavaResult { javaw: path, installed_now: false });
    }

    // 3) 다운로드 + 설치
    tracing::info!("Java 21 을 찾을 수 없음 — 다운로드 시작");
    let javaw = download_and_extract(dirs, progress).await?;
    Ok(JavaResult { javaw, installed_now: true })
}

/// 번들 java 폴더 (prism_root/java/) 안에서 javaw.exe 탐색.
fn find_bundled_java(dirs: &AppDirs) -> Option<PathBuf> {
    let java_dir = dirs.prism_root.join("java");
    if !java_dir.is_dir() {
        return None;
    }
    // jdk-21.xxx/ 또는 jdk-21/ 같은 하위 폴더 탐색
    find_javaw_in_dir(&java_dir)
}

/// 시스템 Java 21+ 탐색 (JAVA_HOME, PATH, 공통 경로).
fn find_system_java() -> Option<PathBuf> {
    // JAVA_HOME
    if let Some(home) = std::env::var_os("JAVA_HOME") {
        let candidate = PathBuf::from(&home).join("bin").join("javaw.exe");
        if candidate.exists() && check_java_version(&candidate) {
            return Some(candidate);
        }
    }

    // PATH 에서 java
    if let Ok(output) = std::process::Command::new("javaw.exe")
        .arg("-version")
        .output()
    {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if parse_major(&stderr).map_or(false, |v| v >= REQUIRED_MAJOR) {
            // which javaw
            if let Ok(out) = std::process::Command::new("where").arg("javaw.exe").output() {
                let s = String::from_utf8_lossy(&out.stdout);
                if let Some(line) = s.lines().next() {
                    let p = PathBuf::from(line.trim());
                    if p.exists() {
                        return Some(p);
                    }
                }
            }
        }
    }

    // 공통 설치 경로
    let program_files = std::env::var_os("ProgramFiles")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\Program Files"));
    for prefix in ["Microsoft", "Eclipse Adoptium", "Java", "Zulu"] {
        let base = program_files.join(prefix);
        if base.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&base) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name_s = name.to_string_lossy();
                    if name_s.contains("21") || name_s.contains("jdk-21") {
                        let javaw = entry.path().join("bin").join("javaw.exe");
                        if javaw.exists() && check_java_version(&javaw) {
                            return Some(javaw);
                        }
                    }
                }
            }
        }
    }

    None
}

/// javaw.exe -version 으로 major 버전 21+ 확인.
fn check_java_version(javaw: &Path) -> bool {
    std::process::Command::new(javaw)
        .arg("-version")
        .output()
        .ok()
        .and_then(|o| {
            let s = String::from_utf8_lossy(&o.stderr);
            parse_major(&s)
        })
        .map_or(false, |v| v >= REQUIRED_MAJOR)
}

/// "openjdk version \"21.0.6\"" → 21
fn parse_major(version_output: &str) -> Option<u32> {
    // "21.0.6" 또는 "21" 패턴 탐색
    for line in version_output.lines() {
        if let Some(start) = line.find('"') {
            let rest = &line[start + 1..];
            if let Some(end) = rest.find('"') {
                let ver = &rest[..end];
                let major = ver.split('.').next()?;
                return major.parse().ok();
            }
        }
    }
    None
}

/// Microsoft OpenJDK 21 다운로드 → prism_root/java/ 에 해제.
async fn download_and_extract(
    dirs: &AppDirs,
    progress: Option<&JavaProgress>,
) -> Result<PathBuf> {
    let java_dir = dirs.prism_root.join("java");
    std::fs::create_dir_all(&java_dir)?;

    let zip_path = dirs.cache.join("openjdk-21.zip");
    std::fs::create_dir_all(&dirs.cache)?;

    if let Some(cb) = progress {
        cb(0, None, "Java 21 다운로드 중 (Microsoft OpenJDK)");
    }
    net::download_plain(&OPENJDK_URL, &zip_path).await
        .context("Java 다운로드 실패")?;

    if let Some(cb) = progress {
        cb(0, None, "Java 압축 해제 중");
    }
    extract_zip(&zip_path, &java_dir)?;

    // 캐시 정리
    let _ = std::fs::remove_file(&zip_path);

    // 해제된 폴더에서 javaw.exe 찾기
    let javaw = find_javaw_in_dir(&java_dir)
        .ok_or_else(|| anyhow::anyhow!("Java 해제 후 javaw.exe 를 찾지 못함"))?;

    tracing::info!(path = %javaw.display(), "Java 21 설치 완료");
    Ok(javaw)
}

/// 자체 런처 전용 — Prism 경로(AppDirs) 없이 임의 폴더에 Java 21 보장.
/// `java_dir`: 번들 Java 가 설치될 루트.  `cache_dir`: 다운로드 zip 임시 보관.
#[cfg(feature = "offline")]
pub async fn ensure_java_at(
    java_dir: &Path,
    cache_dir: &Path,
    progress: Option<&JavaProgress>,
) -> Result<JavaResult> {
    if java_dir.is_dir() {
        if let Some(path) = find_javaw_in_dir(java_dir) {
            return Ok(JavaResult { javaw: path, installed_now: false });
        }
    }
    if let Some(path) = find_system_java() {
        return Ok(JavaResult { javaw: path, installed_now: false });
    }
    std::fs::create_dir_all(java_dir)?;
    std::fs::create_dir_all(cache_dir)?;
    let zip_path = cache_dir.join("openjdk-21.zip");
    if let Some(cb) = progress { cb(0, None, "Java 21 다운로드 중 (Microsoft OpenJDK)"); }
    net::download_plain(&OPENJDK_URL, &zip_path).await.context("Java 다운로드 실패")?;
    if let Some(cb) = progress { cb(0, None, "Java 압축 해제 중"); }
    extract_zip(&zip_path, java_dir)?;
    let _ = std::fs::remove_file(&zip_path);
    let javaw = find_javaw_in_dir(java_dir)
        .ok_or_else(|| anyhow::anyhow!("Java 해제 후 javaw.exe 를 찾지 못함"))?;
    Ok(JavaResult { javaw, installed_now: true })
}

/// 디렉터리 트리에서 bin/javaw.exe 재귀 탐색.
fn find_javaw_in_dir(dir: &Path) -> Option<PathBuf> {
    // 1단계: dir/bin/javaw.exe
    let direct = dir.join("bin").join("javaw.exe");
    if direct.exists() {
        return Some(direct);
    }
    // 2단계: dir/<subdir>/bin/javaw.exe
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                let candidate = entry.path().join("bin").join("javaw.exe");
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// zip 해제 (경로 트래버설 방지).
fn extract_zip(zip_path: &Path, dst_root: &Path) -> Result<()> {
    let file = std::fs::File::open(zip_path)
        .with_context(|| format!("zip 열기 실패: {}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let rel = match entry.enclosed_name() {
            Some(p) => p.to_owned(),
            None => continue,
        };
        let out_path = dst_root.join(&rel);
        if !out_path.starts_with(dst_root) {
            continue;
        }
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)?;
            continue;
        }
        if let Some(p) = out_path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let mut out = std::fs::File::create(&out_path)?;
        std::io::copy(&mut entry, &mut out)?;
    }
    Ok(())
}

/// Prism instance.cfg 에 JavaPath 를 설정한다.
/// OverrideJava=true 가 있어야 Prism이 커스텀 경로를 씀.
pub fn set_instance_java(dirs: &AppDirs, javaw: &Path) -> Result<()> {
    let cfg_path = dirs.instance_root.join("instance.cfg");
    let content = if cfg_path.exists() {
        std::fs::read_to_string(&cfg_path)?
    } else {
        String::new()
    };

    let javaw_str = javaw.to_string_lossy().replace('\\', "/");

    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();

    // Prism schema: OverrideJavaLocation=true 여야 JavaPath 가 글로벌 대신 인스턴스 값을 쓴다.
    // (OverrideJava 키는 존재하지 않음 — 0.1.11 까지 잘못된 키를 쓰고 있었음)
    set_or_insert(&mut lines, "OverrideJavaLocation", "true");
    set_or_insert(&mut lines, "JavaPath", &javaw_str);
    // 인스턴스 자체 설정을 쓰게 하려면 AutomaticJava 비활성화 필요
    set_or_insert(&mut lines, "AutomaticJava", "false");

    let out = lines.join("\n") + "\n";
    std::fs::write(&cfg_path, out)?;
    tracing::info!(java = %javaw_str, "instance.cfg JavaPath 설정");
    Ok(())
}

fn set_or_insert(lines: &mut Vec<String>, key: &str, value: &str) {
    let prefix = format!("{key}=");
    if let Some(pos) = lines.iter().position(|l| l.starts_with(&prefix)) {
        lines[pos] = format!("{key}={value}");
    } else {
        lines.push(format!("{key}={value}"));
    }
}
