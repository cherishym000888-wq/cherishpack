//! HTTP 다운로드 유틸.
//!
//! - `fetch_json` : 원격 JSON → 역직렬화
//! - `download_to_file`: 스트리밍 다운로드 + 진행률 콜백 + sha256 검증

use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use std::{path::Path, time::Duration};
use tokio::{fs::File, io::AsyncWriteExt};

const USER_AGENT: &str = concat!("CherishPack-Installer/", env!("CARGO_PKG_VERSION"));

fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(60))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(Into::into)
}

/// 큰 파일 다운로드용 — 전체 timeout 없음, 읽기 idle timeout만.
fn download_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(60)) // 60초 동안 한 바이트도 안 오면 끊음
        .build()
        .map_err(Into::into)
}

pub async fn fetch_bytes(url: &str) -> Result<Vec<u8>> {
    // 큰 파일(모드 jar)도 받을 수 있게 download_client(전체 타임아웃 없음) 사용.
    // 일시적 네트워크 hiccup을 흡수하기 위해 지수 백오프 재시도 3회.
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 0..3u32 {
        if attempt > 0 {
            let wait = Duration::from_millis(500u64 << attempt); // 1s, 2s
            tokio::time::sleep(wait).await;
            tracing::warn!(url = %url, attempt, "fetch_bytes 재시도");
        }
        let r: Result<Vec<u8>> = async {
            let resp = download_client()?
                .get(url)
                .send()
                .await
                .with_context(|| format!("HTTP GET 실패: {}", url))?
                .error_for_status()?;
            Ok(resp.bytes().await?.to_vec())
        }
        .await;
        match r {
            Ok(b) => return Ok(b),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("HTTP GET 실패: {}", url)))
}

pub async fn fetch_text(url: &str) -> Result<String> {
    let resp = client()?
        .get(url)
        .send()
        .await
        .with_context(|| format!("HTTP GET 실패: {}", url))?
        .error_for_status()?;
    Ok(resp.text().await?)
}

/// 검증 없이 다운로드 (sha256 자산이 없는 경우의 fallback 용도).
pub async fn download_plain(url: &str, dst: &Path) -> Result<()> {
    if let Some(parent) = dst.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    let resp = download_client()?
        .get(url)
        .send()
        .await
        .with_context(|| format!("다운로드 실패: {}", url))?
        .error_for_status()?;
    let mut stream = resp.bytes_stream();
    let mut file = File::create(dst).await?;
    while let Some(chunk) = stream.next().await {
        file.write_all(&chunk?).await?;
    }
    file.flush().await?;
    Ok(())
}

pub async fn fetch_json<T: DeserializeOwned>(url: &str) -> Result<T> {
    let resp = client()?
        .get(url)
        .send()
        .await
        .with_context(|| format!("HTTP GET 실패: {}", url))?
        .error_for_status()?;
    let bytes = resp.bytes().await?;
    serde_json::from_slice::<T>(&bytes)
        .with_context(|| format!("JSON 파싱 실패: {}", url))
}

/// 진행률 콜백 시그니처: (다운로드된 바이트, 전체 바이트|None)
pub type ProgressFn = dyn Fn(u64, Option<u64>) + Send + Sync;

/// 파일을 다운로드하고 sha256이 일치하면 OK, 아니면 파일 삭제 + 에러.
///
/// 이어받기 지원: `<dst>.part` 파일이 있으면 Range 요청으로 재개.
/// 서버가 Range를 무시하면(응답이 200) 처음부터 다시 받는다.
pub async fn download_verified(
    url: &str,
    dst: &Path,
    expected_sha256: &str,
    progress: Option<&ProgressFn>,
) -> Result<()> {
    use tokio::io::AsyncReadExt;

    if let Some(parent) = dst.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }

    let part_path = {
        let mut p = dst.as_os_str().to_owned();
        p.push(".part");
        std::path::PathBuf::from(p)
    };

    // 이미 완성본이 있고 해시가 맞으면 스킵
    if dst.exists() {
        if let Ok(existing) = crate::hash::sha256_file(dst) {
            if existing.eq_ignore_ascii_case(expected_sha256) {
                tracing::info!(path = %dst.display(), "기존 파일 해시 일치 — 다운로드 스킵");
                return Ok(());
            }
            tracing::warn!("기존 파일 해시 불일치 — 삭제 후 재다운로드");
            let _ = tokio::fs::remove_file(dst).await;
        }
    }

    // 재개 가능한 기존 part 확인 + 기존 바이트 해시 누적
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;
    let resume_pos: u64 = match tokio::fs::metadata(&part_path).await {
        Ok(m) => {
            let n = m.len();
            if n > 0 {
                // 기존 바이트 해시에 반영
                let mut f = tokio::fs::File::open(&part_path).await?;
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    let r = f.read(&mut buf).await?;
                    if r == 0 {
                        break;
                    }
                    hasher.update(&buf[..r]);
                }
                downloaded = n;
            }
            n
        }
        Err(_) => 0,
    };

    let mut req = download_client()?.get(url);
    if resume_pos > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={}-", resume_pos));
    }
    let resp = req
        .send()
        .await
        .with_context(|| format!("다운로드 실패: {}", url))?
        .error_for_status()?;

    // 서버가 Range 를 무시해서 200 OK 로 전체를 주면 처음부터 다시
    let server_resumed = resume_pos > 0 && resp.status().as_u16() == 206;
    if resume_pos > 0 && !server_resumed {
        tracing::warn!("서버가 Range 헤더를 무시 — 처음부터 다시 받음");
        hasher = Sha256::new();
        downloaded = 0;
    }

    let total_remaining = resp.content_length();
    let total_full: Option<u64> = if server_resumed {
        total_remaining.map(|r| r + resume_pos)
    } else {
        total_remaining
    };

    // part 파일 열기 (resume 이면 append, 아니면 create)
    let mut file = if server_resumed {
        tokio::fs::OpenOptions::new()
            .append(true)
            .open(&part_path)
            .await?
    } else {
        File::create(&part_path).await?
    };

    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        if let Some(cb) = progress {
            cb(downloaded, total_full);
        }
    }
    file.flush().await?;
    drop(file);

    let got = hex::encode(hasher.finalize());
    if !got.eq_ignore_ascii_case(expected_sha256) {
        // 해시 실패 — part 파일은 남겨두지 말고 삭제 (다음 시도 시 처음부터)
        let _ = tokio::fs::remove_file(&part_path).await;
        bail!(
            "sha256 불일치: expected={}, got={}, url={}",
            expected_sha256,
            got,
            url
        );
    }

    // .part → 최종 경로로 rename
    tokio::fs::rename(&part_path, dst).await?;
    Ok(())
}
