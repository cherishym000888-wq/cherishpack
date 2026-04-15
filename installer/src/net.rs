//! HTTP 다운로드 유틸.
//!
//! - `fetch_json` : 원격 JSON → 역직렬화
//! - `download_to_file`: 스트리밍 다운로드 + 진행률 콜백 + sha256 검증

use anyhow::{anyhow, bail, Context, Result};
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

pub async fn fetch_bytes(url: &str) -> Result<Vec<u8>> {
    let resp = client()?
        .get(url)
        .send()
        .await
        .with_context(|| format!("HTTP GET 실패: {}", url))?
        .error_for_status()?;
    Ok(resp.bytes().await?.to_vec())
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
    let resp = client()?
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
pub async fn download_verified(
    url: &str,
    dst: &Path,
    expected_sha256: &str,
    progress: Option<&ProgressFn>,
) -> Result<()> {
    if let Some(parent) = dst.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }

    let resp = client()?
        .get(url)
        .send()
        .await
        .with_context(|| format!("다운로드 실패: {}", url))?
        .error_for_status()?;

    let total = resp.content_length();
    let mut stream = resp.bytes_stream();
    let mut file = File::create(dst).await?;
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        if let Some(cb) = progress {
            cb(downloaded, total);
        }
    }
    file.flush().await?;
    drop(file);

    let got = hex::encode(hasher.finalize());
    if !got.eq_ignore_ascii_case(expected_sha256) {
        let _ = tokio::fs::remove_file(dst).await;
        bail!(
            "sha256 불일치: expected={}, got={}, url={}",
            expected_sha256,
            got,
            url
        );
    }

    if total.map_or(false, |t| t != downloaded) {
        return Err(anyhow!(
            "다운로드 크기 불일치: expected={:?}, got={}",
            total,
            downloaded
        ));
    }

    Ok(())
}
