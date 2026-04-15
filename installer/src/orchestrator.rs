//! 설치/패치 전체 흐름.
//!
//! GUI 에서 `run()` 을 tokio task로 띄우고, 진행 상황은 `mpsc::Sender<Event>` 로 흘린다.

use anyhow::{bail, Context, Result};
use std::cmp::Ordering;

use crate::{
    channel::{self, Channel},
    config::{ChannelEntry, CurrentManifest, PackManifest, VersionIndex},
    mrpack, net,
    paths::AppDirs,
    patcher, prism, state,
};

#[derive(Debug, Clone)]
pub enum Event {
    Status(String),
    Progress { done: u64, total: Option<u64>, label: String },
    SubStep { idx: usize, total: usize, label: String },
    Info(String),
    Warn(String),
    Done { launched: bool },
    Error(String),
}

pub struct RunOptions {
    pub channel: Channel,
    pub preset: Option<String>,
    pub auto_launch: bool,
}

pub async fn run(
    dirs: AppDirs,
    opts: RunOptions,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
) {
    if let Err(e) = run_inner(&dirs, &opts, &tx).await {
        let _ = tx.send(Event::Error(format!("{e:#}")));
    }
}

async fn run_inner(
    dirs: &AppDirs,
    opts: &RunOptions,
    tx: &tokio::sync::mpsc::UnboundedSender<Event>,
) -> Result<()> {
    macro_rules! status {
        ($($arg:tt)*) => { let _ = tx.send(Event::Status(format!($($arg)*))); };
    }
    macro_rules! info {
        ($($arg:tt)*) => { let _ = tx.send(Event::Info(format!($($arg)*))); };
    }
    macro_rules! warn_ {
        ($($arg:tt)*) => { let _ = tx.send(Event::Warn(format!($($arg)*))); };
    }

    // 1. 로컬 상태 로드
    let mut st = state::load(&dirs.state_file);
    st.channel = opts.channel.as_str().to_string();
    if let Some(p) = &opts.preset {
        st.user_preset = Some(p.clone());
    }

    // 2. version.json 조회
    status!("최신 버전 확인 중");
    let index: VersionIndex = net::fetch_json(channel::VERSION_INDEX_URL)
        .await
        .context("version.json 조회 실패")?;
    let entry: ChannelEntry = match opts.channel {
        Channel::Stable => index.stable,
        Channel::Beta => index.beta.unwrap_or(index.stable),
    };
    info!("서버 버전: {} (min_required={})", entry.version, entry.min_required);

    // 3. 강제 업데이트 체크 — 설치본이 min_required 미만이면 업데이트 강제
    if let Some(cur) = &st.installed_version {
        if state::compare(cur, &entry.min_required) == Ordering::Less {
            warn_!("설치된 버전 {cur} 이 최소 요구 {min_req} 미만 — 강제 업데이트",
                min_req = entry.min_required);
        }
    }

    // 4. 매니페스트 다운로드
    status!("매니페스트 다운로드");
    let manifest: PackManifest = net::fetch_json(&entry.manifest_url)
        .await
        .context("매니페스트 조회 실패")?;

    // min_installer_version 체크 — 설치 프로그램이 낡았으면 알림
    if let Some(min_inst) = &manifest.min_installer_version {
        if state::compare(env!("CARGO_PKG_VERSION"), min_inst) == Ordering::Less {
            bail!(
                "설치 프로그램 업데이트 필요 — 최소 {min_inst}, 현재 {cur}",
                cur = env!("CARGO_PKG_VERSION")
            );
        }
    }

    // 5. Prism 설치 보장
    status!("Prism Launcher 준비 중");
    let tx_p = tx.clone();
    let prism_install = prism::ensure_installed(
        dirs,
        Some(&move |d, t, label| {
            let _ = tx_p.send(Event::Progress {
                done: d,
                total: t,
                label: label.to_string(),
            });
        }),
    )
    .await
    .context("Prism 설치 실패")?;

    // 6. .mrpack 다운로드 + 검증
    status!("모드팩 다운로드 중 ({} )", entry.version);
    let mrpack_path = dirs.cache.join(format!("cherishpack-{}.mrpack", entry.version));
    let tx_m = tx.clone();
    net::download_verified(
        &manifest.mrpack_url,
        &mrpack_path,
        &manifest.mrpack_sha256,
        Some(&move |d, t| {
            let _ = tx_m.send(Event::Progress {
                done: d,
                total: t,
                label: "모드팩 아카이브 다운로드".into(),
            });
        }),
    )
    .await
    .context("mrpack 다운로드 실패")?;

    // 7. 이전 매니페스트 로드
    let previous = patcher::load_current_manifest(&dirs.manifest_file);

    // 8. mrpack 적용
    status!("모드팩 적용 중");
    let tx_a = tx.clone();
    let applied = mrpack::apply(
        &mrpack_path,
        &dirs.minecraft_root,
        Some(&move |idx, total, label| {
            let _ = tx_a.send(Event::SubStep {
                idx,
                total,
                label: label.to_string(),
            });
        }),
    )
    .await
    .context("mrpack 적용 실패")?;

    // 9. 구 파일 정리 (4중 안전장치 + 휴지통)
    status!("구버전 파일 정리 중");
    let plan = patcher::prune_stale_files(
        previous.as_ref(),
        &manifest,
        &applied.files,
        &dirs.minecraft_root,
    )?;
    info!(
        "정리 결과: 삭제 {}, 사용자수정 보존 {}, preserve 보존 {}",
        plan.deleted.len(),
        plan.skipped_user_modified.len(),
        plan.skipped_preserved.len()
    );

    // 10. current-manifest 저장
    patcher::save_current_manifest(
        &dirs.manifest_file,
        &CurrentManifest {
            pack_version: entry.version.clone(),
            files: applied.files,
        },
    )?;

    // 11. InstallerState 업데이트
    st.installed_version = Some(entry.version.clone());
    state::save(&dirs.state_file, &st)?;

    // 12. Prism 실행
    if opts.auto_launch {
        status!("Prism 실행");
        prism::launch_instance(dirs, &prism_install)?;
        let _ = tx.send(Event::Done { launched: true });
    } else {
        let _ = tx.send(Event::Done { launched: false });
    }

    Ok(())
}
