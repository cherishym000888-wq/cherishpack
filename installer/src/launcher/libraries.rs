//! 라이브러리 규칙(rules) 평가 · 다운로드 · classpath 구성.
//!
//! 1.21.1 의 version.json 은 모든 natives 까지 평범한 `libraries[]` 항목으로
//! 들어있고 (`name` 에 `:natives-windows` 같은 분류자가 붙음), 각 항목은
//! OS 별 rule 로 필터된다. 구(舊) 스키마의 `classifiers` 맵은 참고용으로만
//! 처리한다 — 1.21.1 에서는 나타나지 않음.
//!
//! 현재 타겟 OS 는 **Windows x64** 로 고정한다. 다른 OS 는 런처 자체가
//! Windows 전용이라 고민하지 않는다.

use anyhow::{Context, Result};
use futures_util::{stream, StreamExt};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use super::meta::{DownloadArtifact, Library, OsConstraint, Rule, VersionMeta};

// ─────────────────────── 현재 OS ───────────────────────

/// Mojang 명세에서 쓰는 OS 이름.
const OS_NAME: &str = "windows";
const OS_ARCH: &str = "x64";

pub fn os_matches(os: &OsConstraint) -> bool {
    if let Some(name) = &os.name {
        if name != OS_NAME {
            return false;
        }
    }
    if let Some(arch) = &os.arch {
        if arch != OS_ARCH {
            return false;
        }
    }
    // version 정규식은 무시 — 최신 Windows 10/11 기준으로 전부 허용.
    true
}

/// rule 리스트가 "현재 환경에 허용된다" 로 평가되는지.
///
/// Mojang 규칙: 없으면 allow, 있으면 마지막으로 매칭된 rule 의 action 을 따름.
/// - allow + os 없음  → 항상 매칭
/// - allow + os 매칭  → 매칭
/// - disallow + os 매칭 → 매칭 (그러나 최종 결과는 block)
pub fn rules_allow(rules: &[Rule], features: &HashMap<String, bool>) -> bool {
    if rules.is_empty() {
        return true;
    }
    let mut allowed = false;
    for r in rules {
        if !rule_matches(r, features) {
            continue;
        }
        allowed = r.action == "allow";
    }
    allowed
}

fn rule_matches(rule: &Rule, features: &HashMap<String, bool>) -> bool {
    if let Some(os) = &rule.os {
        if !os_matches(os) {
            return false;
        }
    }
    if let Some(want) = &rule.features {
        // 런치 기능 플래그(is_demo_user, has_custom_resolution 등) — 우리는 전부 false.
        for (k, v) in want {
            if features.get(k).copied().unwrap_or(false) != *v {
                return false;
            }
        }
    }
    true
}

// ─────────────────────── 계획 ───────────────────────

#[derive(Debug, Clone)]
pub struct LibraryEntry {
    /// 로컬 저장 경로 (절대).
    pub local_path: PathBuf,
    pub url: String,
    pub sha1: String,
    pub size: u64,
    /// classpath 에 포함할지 — natives-* 분류자도 1.21.1 은 classpath 에 넣는다.
    pub on_classpath: bool,
}

#[derive(Debug, Default)]
pub struct LibraryPlan {
    pub entries: Vec<LibraryEntry>,
}

impl LibraryPlan {
    /// classpath 문자열 (OS 구분자로 join). client.jar 는 호출부에서 별도로 append.
    pub fn classpath(&self, extra: &[PathBuf]) -> String {
        let sep = if cfg!(windows) { ";" } else { ":" };
        self.entries
            .iter()
            .filter(|e| e.on_classpath)
            .map(|e| e.local_path.to_string_lossy().into_owned())
            .chain(extra.iter().map(|p| p.to_string_lossy().into_owned()))
            .collect::<Vec<_>>()
            .join(sep)
    }
}

/// version.json + 로컬 libraries 디렉토리 → 다운로드·classpath 계획.
pub fn plan(meta: &VersionMeta, libraries_dir: &Path) -> LibraryPlan {
    let features: HashMap<String, bool> = HashMap::new();
    let mut entries: Vec<LibraryEntry> = Vec::with_capacity(meta.libraries.len());

    for lib in &meta.libraries {
        if let Some(rules) = &lib.rules {
            if !rules_allow(rules, &features) {
                tracing::debug!(name = %lib.name, "rule 차단 — 스킵");
                continue;
            }
        }

        if let Some(dl) = &lib.downloads {
            if let Some(art) = &dl.artifact {
                if let Some(e) = artifact_entry(art, libraries_dir, true) {
                    entries.push(e);
                }
            }
            // 구 스키마 classifiers (1.21.1 에선 거의 안 쓰임)
            if let Some(classifiers) = &dl.classifiers {
                if let Some(classifier) = native_classifier(lib) {
                    if let Some(art) = classifiers.get(&classifier) {
                        if let Some(e) = artifact_entry(art, libraries_dir, true) {
                            entries.push(e);
                        }
                    }
                }
            }
        }
    }

    LibraryPlan { entries }
}

fn artifact_entry(art: &DownloadArtifact, libraries_dir: &Path, on_cp: bool) -> Option<LibraryEntry> {
    let path = art.path.as_deref()?;
    Some(LibraryEntry {
        local_path: libraries_dir.join(path),
        url: art.url.clone(),
        sha1: art.sha1.clone(),
        size: art.size,
        on_classpath: on_cp,
    })
}

/// Maven 좌표 `group:artifact:version[:classifier]` → 상대 path.
/// NeoForge 에서 `downloads.artifact.path` 가 누락된 항목 fallback 용. 현재 미사용.
#[allow(dead_code)]
pub fn maven_path(coord: &str) -> Option<String> {
    let mut parts = coord.split(':');
    let group = parts.next()?;
    let artifact = parts.next()?;
    let version = parts.next()?;
    let classifier = parts.next();
    let group_path = group.replace('.', "/");
    let filename = match classifier {
        Some(c) => format!("{}-{}-{}.jar", artifact, version, c),
        None => format!("{}-{}.jar", artifact, version),
    };
    Some(format!("{}/{}/{}/{}", group_path, artifact, version, filename))
}

/// 구 `natives` 맵에서 현재 OS 에 맞는 분류자 이름을 고른다.
fn native_classifier(lib: &Library) -> Option<String> {
    let map = lib.natives.as_ref()?;
    // "natives-windows" 같은 템플릿 — ${arch} 치환 지원.
    let raw = map.get(OS_NAME)?;
    Some(raw.replace("${arch}", if OS_ARCH == "x64" { "64" } else { "32" }))
}

// ─────────────────────── 다운로드 ───────────────────────

/// 라이브러리 동시 다운로드 — 보통 80~150개 규모라 8 병렬로 충분.
const CONCURRENCY: usize = 8;

/// 계획에 있는 전체 라이브러리를 다운로드 (이미 있고 sha1 이 맞으면 스킵).
pub async fn download_all(plan: &LibraryPlan) -> Result<()> {
    // 소유권 있는 Vec 로 복제 — spawn 경계에서 HRTB 문제를 피하기 위함.
    let entries: Vec<LibraryEntry> = plan.entries.clone();
    let total = entries.len();
    let done = Arc::new(AtomicUsize::new(0));

    let results: Vec<Result<()>> = stream::iter(entries.into_iter())
        .map(|e| {
            let done = done.clone();
            async move {
                fetch_one(&e).await?;
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                if n % 20 == 0 || n == total {
                    tracing::info!(i = n, total, "라이브러리 다운로드 진행");
                }
                Ok(())
            }
        })
        .buffer_unordered(CONCURRENCY)
        .collect()
        .await;

    for r in results {
        r?;
    }
    Ok(())
}

async fn fetch_one(e: &LibraryEntry) -> Result<()> {
    // url 이 비어있으면 processors/설치기가 생성한 로컬 파일 — 다운로드 스킵.
    // classpath 에는 여전히 올라가므로, 런치 전에 파일 존재는 다른 경로로 보장돼야 한다.
    if e.url.is_empty() {
        tracing::debug!(path = %e.local_path.display(), "url 빈 라이브러리 — 다운로드 스킵 (로컬 생성 가정)");
        return Ok(());
    }
    if let Some(parent) = e.local_path.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    if e.local_path.exists() {
        if !e.sha1.is_empty() {
            if let Ok(got) = crate::hash::sha1_file(&e.local_path) {
                if got.eq_ignore_ascii_case(&e.sha1) {
                    return Ok(());
                }
                let _ = tokio::fs::remove_file(&e.local_path).await;
            }
        } else {
            // sha1 정보가 없으면 존재만으로 스킵 (NeoForge 일부 엔트리)
            return Ok(());
        }
    }
    download_with_sha1(&e.url, &e.local_path, &e.sha1, e.size)
        .await
        .with_context(|| format!("라이브러리 실패: {}", e.url))
}

/// sha1 검증 다운로드 — net.rs 는 sha256 기반이라 별도 구현.
/// 라이브러리 jar 는 보통 수백 KB ~ 수 MB 라 스트리밍 없이 bytes 로 받고 검증.
async fn download_with_sha1(url: &str, dst: &Path, expected: &str, _size: u64) -> Result<()> {
    use sha1::{Digest, Sha1};

    let bytes = crate::net::fetch_bytes(url).await?;
    if !expected.is_empty() {
        let mut h = Sha1::new();
        h.update(&bytes);
        let got = hex::encode(h.finalize());
        if !got.eq_ignore_ascii_case(expected) {
            anyhow::bail!("sha1 불일치: expected={}, got={}, url={}", expected, got, url);
        }
    }
    tokio::fs::write(dst, &bytes).await?;
    Ok(())
}

// ─────────────────────── 테스트 ───────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn feats() -> HashMap<String, bool> {
        HashMap::new()
    }

    #[test]
    fn empty_rules_allow() {
        assert!(rules_allow(&[], &feats()));
    }

    #[test]
    fn disallow_on_current_os_blocks() {
        let rules = vec![
            Rule {
                action: "allow".into(),
                os: None,
                features: None,
            },
            Rule {
                action: "disallow".into(),
                os: Some(OsConstraint {
                    name: Some("windows".into()),
                    arch: None,
                    version: None,
                }),
                features: None,
            },
        ];
        assert!(!rules_allow(&rules, &feats()));
    }

    #[test]
    fn allow_only_matching_os() {
        let rules = vec![Rule {
            action: "allow".into(),
            os: Some(OsConstraint {
                name: Some("osx".into()),
                arch: None,
                version: None,
            }),
            features: None,
        }];
        assert!(!rules_allow(&rules, &feats()));
    }
}
