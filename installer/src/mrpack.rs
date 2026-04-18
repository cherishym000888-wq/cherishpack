//! .mrpack (Modrinth modpack) 파싱·적용.
//!
//! 포맷
//!   zip
//!   ├── modrinth.index.json
//!   ├── overrides/          (client+server 공용)
//!   └── client-overrides/   (클라이언트 전용, overrides 보다 우선)
//!
//! 적용 순서
//!   1. zip 열기 → modrinth.index.json 파싱
//!   2. files[] 중 env.client != "unsupported" 만 대상
//!   3. 병렬 다운로드 (동시 4개) + sha512(또는 sha1) 검증 → minecraft_root/<path>
//!   4. overrides/ 그 다음 client-overrides/ 를 minecraft_root 위로 풀기
//!   5. 배치된 파일 목록 + sha256 맵을 반환 (current-manifest.json 저장용)

use anyhow::{anyhow, bail, Context, Result};
use futures_util::{stream, StreamExt};
use serde::Deserialize;
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};
use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::io::AsyncWriteExt;

use crate::net;

const PARALLEL_DOWNLOADS: usize = 4;

#[derive(Debug, Clone)]
pub struct AppliedPack {
    /// 상대경로(슬래시) → sha256
    pub files: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct Index {
    #[serde(rename = "formatVersion")]
    format_version: u32,
    #[allow(dead_code)]
    game: String,
    #[serde(rename = "versionId")]
    #[allow(dead_code)]
    version_id: String,
    #[allow(dead_code)]
    name: String,
    files: Vec<IndexFile>,
    #[allow(dead_code)]
    dependencies: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct IndexFile {
    path: String,
    hashes: Hashes,
    downloads: Vec<String>,
    #[serde(rename = "fileSize")]
    #[allow(dead_code)]
    file_size: Option<u64>,
    #[serde(default)]
    env: Option<Env>,
}

#[derive(Debug, Deserialize)]
struct Hashes {
    #[serde(default)]
    sha512: Option<String>,
    #[serde(default)]
    sha1: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Env {
    #[serde(default)]
    client: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    server: Option<String>,
}

pub type ApplyProgress = dyn Fn(usize, usize, &str) + Send + Sync;

pub async fn apply(
    mrpack_path: &Path,
    minecraft_root: &Path,
    progress: Option<&ApplyProgress>,
) -> Result<AppliedPack> {
    std::fs::create_dir_all(minecraft_root)?;

    // 1. zip 메모리 로드 (수 MB ~ 수십 MB 수준이므로 OK)
    let data = std::fs::read(mrpack_path)
        .with_context(|| format!("mrpack 읽기 실패: {}", mrpack_path.display()))?;
    let reader = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(reader)?;

    // 2. modrinth.index.json
    let index: Index = {
        let mut entry = archive
            .by_name("modrinth.index.json")
            .context("modrinth.index.json 이 mrpack 안에 없음")?;
        let mut s = String::new();
        entry.read_to_string(&mut s)?;
        serde_json::from_str(&s).context("modrinth.index.json 파싱 실패")?
    };
    if index.format_version != 1 {
        bail!("지원하지 않는 mrpack formatVersion: {}", index.format_version);
    }

    // 3. 클라이언트 대상 파일만 (참조 수명 문제 피하려고 owned 로 복사)
    struct Job {
        path: String,
        downloads: Vec<String>,
        sha512: Option<String>,
        sha1: Option<String>,
    }
    let jobs: Vec<Job> = index
        .files
        .into_iter()
        .filter(|f| {
            f.env
                .as_ref()
                .and_then(|e| e.client.as_deref())
                .map(|c| c != "unsupported")
                .unwrap_or(true)
        })
        .map(|f| Job {
            path: f.path,
            downloads: f.downloads,
            sha512: f.hashes.sha512,
            sha1: f.hashes.sha1,
        })
        .collect();

    tracing::info!(count = jobs.len(), "mrpack 파일 다운로드 대상");

    let mc_root: Arc<PathBuf> = Arc::new(minecraft_root.to_path_buf());
    let total = jobs.len();

    // 4. 병렬 다운로드 (bounded)
    let results: Vec<Result<(String, String)>> = stream::iter(jobs.into_iter().enumerate())
        .map(|(idx, job)| {
            let mc_root = Arc::clone(&mc_root);
            let Job { path, downloads, sha512, sha1 } = job;
            let progress_ref = progress;
            async move {
                if downloads.is_empty() {
                    bail!("downloads 비어있음: {}", path);
                }
                // 경로 트래버설 방지
                let safe_rel = sanitize_relpath(&path)
                    .ok_or_else(|| anyhow!("의심스러운 경로: {}", path))?;
                let dst = mc_root.join(&safe_rel);
                if let Some(p) = dst.parent() {
                    tokio::fs::create_dir_all(p).await.ok();
                }

                let (bytes, source_url) = download_first_ok(&downloads).await?;

                // 검증 — sha512 우선, 없으면 sha1
                if let Some(expected) = sha512.as_deref() {
                    let got = hex::encode(Sha512::digest(&bytes));
                    if !got.eq_ignore_ascii_case(expected) {
                        bail!("sha512 불일치: {} ({})", path, source_url);
                    }
                } else if let Some(expected) = sha1.as_deref() {
                    let got = hex::encode(Sha1::digest(&bytes));
                    if !got.eq_ignore_ascii_case(expected) {
                        bail!("sha1 불일치: {} ({})", path, source_url);
                    }
                } else {
                    bail!("해시 정보 없음: {}", path);
                }

                // 로컬 저장 + sha256 계산 (매니페스트용)
                let sha256 = hex::encode(Sha256::digest(&bytes));
                let mut f = tokio::fs::File::create(&dst).await?;
                f.write_all(&bytes).await?;
                f.flush().await?;
                drop(f);

                if let Some(cb) = progress_ref {
                    cb(idx + 1, total, &safe_rel);
                }

                Ok((safe_rel, sha256))
            }
        })
        .buffer_unordered(PARALLEL_DOWNLOADS)
        .collect()
        .await;

    let mut files: HashMap<String, String> = HashMap::new();
    let mut errors: Vec<String> = Vec::new();
    for r in results {
        match r {
            Ok((p, h)) => {
                files.insert(p, h);
            }
            Err(e) => errors.push(format!("{e}")),
        }
    }
    if !errors.is_empty() {
        bail!(
            "mrpack 적용 실패 — {}건 오류:\n  - {}",
            errors.len(),
            errors.join("\n  - ")
        );
    }

    // 5. overrides/ → client-overrides/ 순으로 풀기 (client-overrides가 나중이므로 덮어씀)
    for prefix in ["overrides/", "client-overrides/"] {
        extract_overrides(&mut archive, prefix, minecraft_root, &mut files)?;
    }

    Ok(AppliedPack { files })
}

/// 상대경로 정화 — 절대경로·`..`·드라이브문자·역슬래시 제거.
fn sanitize_relpath(raw: &str) -> Option<String> {
    let p = PathBuf::from(raw.replace('\\', "/"));
    if p.is_absolute() {
        return None;
    }
    let mut out = Vec::new();
    for comp in p.components() {
        use std::path::Component::*;
        match comp {
            CurDir => {}
            ParentDir | RootDir | Prefix(_) => return None,
            Normal(s) => out.push(s.to_string_lossy().to_string()),
        }
    }
    if out.is_empty() {
        return None;
    }
    Some(out.join("/"))
}

/// 첫 번째 성공하는 URL에서 바이트를 받아온다.
async fn download_first_ok(urls: &[String]) -> Result<(Vec<u8>, String)> {
    let mut last_err: Option<anyhow::Error> = None;
    for url in urls {
        match net::fetch_bytes(url).await {
            Ok(b) => return Ok((b, url.clone())),
            Err(e) => {
                tracing::warn!(url = %url, error = %e, "다운로드 실패, 다음 미러 시도");
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("downloads 비어있음")))
}

/// zip 의 `overrides/` 또는 `client-overrides/` 하위 항목을 풀어 minecraft_root 위로 복사.
fn extract_overrides(
    archive: &mut zip::ZipArchive<std::io::Cursor<Vec<u8>>>,
    prefix: &str,
    dst_root: &Path,
    files_out: &mut HashMap<String, String>,
) -> Result<()> {
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        // Windows ZipFile::CreateFromDirectory 는 역슬래시로 엔트리를 만들 수 있으므로
        // 정규화 후 prefix 비교.
        let name = entry.name().to_string().replace('\\', "/");
        if !name.starts_with(prefix) {
            continue;
        }
        let rel_raw = &name[prefix.len()..];
        if rel_raw.is_empty() {
            continue;
        }
        let safe_rel = match sanitize_relpath(rel_raw) {
            Some(p) => p,
            None => {
                tracing::warn!(name = %name, "overrides 경로 이탈, 스킵");
                continue;
            }
        };
        let out_path = dst_root.join(&safe_rel);

        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)?;
            continue;
        }
        if let Some(p) = out_path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut buf)?;
        let sha256 = hex::encode(Sha256::digest(&buf));
        std::fs::write(&out_path, &buf)?;
        files_out.insert(safe_rel, sha256);
    }
    Ok(())
}
