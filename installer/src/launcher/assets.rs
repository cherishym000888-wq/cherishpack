//! Minecraft 에셋(사운드·언어·텍스처 인덱스) 다운로드.
//!
//! 흐름:
//!   1. version.json 의 `assetIndex.url` 을 받아 sha1 검증 후
//!      `assets/indexes/<id>.json` 으로 저장.
//!   2. 인덱스의 `objects` 맵을 순회하여 각 오브젝트를
//!      `assets/objects/<prefix>/<hash>` 로 받는다 (`prefix` = hash 의 앞 2글자).
//!   3. 동일 파일이 이미 있고 sha1 이 맞으면 스킵.
//!
//! resources.mojang.com 은 Range 를 지원하지만 에셋은 대부분 KB 단위라
//! 스트리밍 이어받기까지는 필요 없다. 단순히 bytes 받고 검증.

use anyhow::{Context, Result};
use futures_util::{stream, StreamExt};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use super::meta::{AssetIndexRef, VersionMeta};

const RESOURCES_BASE: &str = "https://resources.download.minecraft.net";

#[derive(Debug, Deserialize)]
pub struct AssetIndex {
    pub objects: BTreeMap<String, AssetObject>,
    /// 레거시(1.6 이전) 가상 에셋 플래그 — 1.21.1 에서는 미사용, 스키마 보존.
    #[allow(dead_code)]
    #[serde(default)]
    pub map_to_resources: bool,
    #[allow(dead_code)]
    #[serde(rename = "virtual", default)]
    pub is_virtual: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AssetObject {
    pub hash: String,
    /// 무결성 검증은 해시로 하므로 size 는 참고용 — 프로그레스 표시 확장 시 사용.
    #[allow(dead_code)]
    pub size: u64,
}

/// assets 루트 디렉토리(`assets/`) 안의 하위 경로 계산.
pub fn indexes_dir(assets_root: &Path) -> PathBuf {
    assets_root.join("indexes")
}
pub fn objects_dir(assets_root: &Path) -> PathBuf {
    assets_root.join("objects")
}

/// assetIndex json 을 받아 로컬에 저장하고 파싱된 구조를 돌려준다.
pub async fn fetch_index(idx: &AssetIndexRef, assets_root: &Path) -> Result<AssetIndex> {
    let dst = indexes_dir(assets_root).join(format!("{}.json", idx.id));

    if dst.exists() {
        if let Ok(got) = crate::hash::sha1_file(&dst) {
            if got.eq_ignore_ascii_case(&idx.sha1) {
                tracing::debug!(path = %dst.display(), "기존 assetIndex sha1 일치");
                let bytes = tokio::fs::read(&dst).await?;
                return Ok(serde_json::from_slice(&bytes)?);
            }
        }
    }

    if let Some(parent) = dst.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }

    let bytes = crate::net::fetch_bytes(&idx.url)
        .await
        .with_context(|| format!("assetIndex 다운로드 실패: {}", idx.url))?;

    use sha1::{Digest, Sha1};
    let mut h = Sha1::new();
    h.update(&bytes);
    let got = hex::encode(h.finalize());
    if !got.eq_ignore_ascii_case(&idx.sha1) {
        anyhow::bail!(
            "assetIndex sha1 불일치: expected={}, got={}",
            idx.sha1,
            got
        );
    }

    tokio::fs::write(&dst, &bytes).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// 동시 다운로드 수. 1.21.1 기준 오브젝트 ~3,600개 — 16 병렬이면 수분 → 30초대.
const CONCURRENCY: usize = 16;

/// 모든 오브젝트를 `objects/<xx>/<hash>` 로 다운로드 (16개 병렬).
pub async fn download_objects(index: &AssetIndex, assets_root: &Path) -> Result<()> {
    let objects_root = objects_dir(assets_root);
    let total = index.objects.len();
    let done = Arc::new(AtomicUsize::new(0));

    // 소유권 있는 Vec 로 변환 — spawn 경계에서 HRTB 문제 회피.
    let owned: Vec<(String, AssetObject)> = index
        .objects
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    let results: Vec<Result<()>> = stream::iter(owned.into_iter())
        .map(|(name, obj)| {
            let objects_root = objects_root.clone();
            let done = done.clone();
            async move {
                fetch_one(&name, &obj, &objects_root).await?;
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                if n % 200 == 0 || n == total {
                    tracing::info!(i = n, total, "에셋 다운로드 진행");
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

async fn fetch_one(name: &str, obj: &AssetObject, objects_root: &Path) -> Result<()> {
    let prefix = &obj.hash[..2];
    let dst = objects_root.join(prefix).join(&obj.hash);

    if dst.exists() {
        if let Ok(got) = crate::hash::sha1_file(&dst) {
            if got.eq_ignore_ascii_case(&obj.hash) {
                return Ok(());
            }
        }
        let _ = tokio::fs::remove_file(&dst).await;
    }

    if let Some(parent) = dst.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }

    let url = format!("{}/{}/{}", RESOURCES_BASE, prefix, obj.hash);
    let bytes = crate::net::fetch_bytes(&url)
        .await
        .with_context(|| format!("에셋 다운로드 실패: {} ({})", name, url))?;

    use sha1::{Digest, Sha1};
    let mut h = Sha1::new();
    h.update(&bytes);
    let got = hex::encode(h.finalize());
    if !got.eq_ignore_ascii_case(&obj.hash) {
        anyhow::bail!(
            "에셋 sha1 불일치: expected={}, got={}, name={}",
            obj.hash,
            got,
            name
        );
    }
    tokio::fs::write(&dst, &bytes).await?;
    Ok(())
}

/// 편의 함수: version.json → 인덱스 + 모든 오브젝트.
pub async fn sync_all(meta: &VersionMeta, assets_root: &Path) -> Result<AssetIndex> {
    let index = fetch_index(&meta.asset_index, assets_root).await?;
    download_objects(&index, assets_root).await?;
    Ok(index)
}
