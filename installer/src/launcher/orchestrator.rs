//! 자체 런처 파이프라인 오케스트레이터.
//!
//! 레거시 `orchestrator.rs` 가 Prism 경로를 고수하는 동안, 이 모듈은
//! `launcher::*` 를 엮어 Prism 없이 설치·런치까지 끌고 간다. CLI 의
//! `--launcher` 플래그 또는 GUI 전환 시 진입한다.
//!
//! 단계:
//!   1. LauncherDirs 해석·생성
//!   2. 원격 PackManifest 조회 (기존 config·channel 재사용)
//!   3. Java 21 보장 (기존 `java::ensure_java` 재사용 — jre 는 공용)
//!   4. 바닐라 1.21.1 meta 로드 → 라이브러리/에셋/네이티브 동기화
//!   5. NeoForge installer 다운로드 → version.json 추출 → 바닐라와 병합
//!   6. 병합 meta 의 추가 라이브러리 동기화 (url 빈 항목 스킵)
//!   7. mrpack 적용 (대상: `dirs.instance`)
//!   8. `run::run(ctx)` 로 런치

use anyhow::{Context, Result};
use std::path::PathBuf;

use crate::{
    channel::{self, Channel},
    config::{ChannelEntry, PackManifest, VersionIndex},
    java, mrpack, net,
};

use super::{
    assets, auth, cache, dirs::LauncherDirs, dirs::RuntimeLayout, libraries, meta, natives,
    neoforge, run,
};

#[derive(Debug, Clone)]
pub enum Event {
    Status(String),
    Info(String),
    Warn(String),
    Progress { done: u64, total: Option<u64>, label: String },
    /// MSA device code — UI 에서 사용자에게 user_code + URL 안내.
    /// `expires_in` 초 후 코드 만료 — UI 는 카운트다운 표시.
    AuthChallenge { user_code: String, verification_uri: String, expires_in: u64 },
    Done { launched: bool },
    Error(String),
}

pub struct RunOptions {
    pub channel: Channel,
    pub auto_launch: bool,
    /// "low" | "medium" | "high" | "high_plus" — 매니페스트의 hw_profiles 키.
    pub preset: Option<String>,
    /// ⚠ `offline` feature 빌드에서만 의미 있음 — 설정되면 MSA 를 건너뛰고
    /// 지정 닉네임으로 합성 계정 생성. 내부 테스트 전용.
    #[cfg(feature = "offline")]
    pub offline_nickname: Option<String>,
}

pub async fn run_launcher(
    opts: RunOptions,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
) {
    if let Err(e) = run_inner(opts, &tx).await {
        let _ = tx.send(Event::Error(format!("{e:#}")));
    }
}

async fn run_inner(
    opts: RunOptions,
    tx: &tokio::sync::mpsc::UnboundedSender<Event>,
) -> Result<()> {
    macro_rules! status {
        ($($arg:tt)*) => {{ let _ = tx.send(Event::Status(format!($($arg)*))); }};
    }
    macro_rules! info {
        ($($arg:tt)*) => {{ let _ = tx.send(Event::Info(format!($($arg)*))); }};
    }
    macro_rules! warn_ {
        ($($arg:tt)*) => {{ let _ = tx.send(Event::Warn(format!($($arg)*))); }};
    }

    // 1. 디렉토리
    let dirs = LauncherDirs::resolve()?;
    dirs.ensure_exists()?;
    info!("런처 루트: {}", dirs.root.display());

    // 1.5. 계정 확보 — MSA 또는 (feature=offline 빌드에서) 합성 오프라인 계정.
    let (msa_auth, user_type) = resolve_account(&dirs, tx, &opts).await?;
    info!("프로필: {} ({}) [{}]", msa_auth.profile.name, msa_auth.profile.uuid_dashed(), user_type);

    // 2. 매니페스트 (기존 채널 엔드포인트 재사용)
    status!("최신 버전 확인 중");
    let index: VersionIndex = net::fetch_json(channel::VERSION_INDEX_URL)
        .await
        .context("version.json 조회 실패")?;
    let entry: ChannelEntry = match opts.channel {
        Channel::Stable => index.stable,
        Channel::Beta => index.beta.unwrap_or(index.stable),
    };
    info!("서버 버전: {} (min_required={})", entry.version, entry.min_required);

    let manifest: PackManifest = net::fetch_json(&entry.manifest_url)
        .await
        .context("매니페스트 조회 실패")?;
    info!("MC {} / {} {}", manifest.minecraft, manifest.loader.kind, manifest.loader.version);

    // 3. Java 21
    status!("Java 21 확인 중");
    let java_result = java::ensure_java_at(&dirs.java, &dirs.cache, None)
        .await
        .context("Java 설치 실패")?;
    info!("Java: {}", java_result.javaw.display());

    // 4. 바닐라 meta + 동기화
    status!("Minecraft {} 메타 로드", manifest.minecraft);
    let vanilla = meta::load(&manifest.minecraft).await?;

    status!("라이브러리 동기화");
    let plan_vanilla = libraries::plan(&vanilla, &dirs.libraries);
    libraries::download_all(&plan_vanilla).await?;

    status!("에셋 동기화 ({} 개)", estimate_asset_count(&vanilla));
    let _index = assets::sync_all(&vanilla, &dirs.assets).await?;

    status!("네이티브 추출");
    let natives_dir = dirs.natives_dir(&manifest.minecraft);
    let vanilla_natives = natives::filter_natives(&plan_vanilla.entries, &vanilla);
    natives::extract_all(&vanilla_natives, &natives_dir)?;

    // 4.9. 바닐라 client.jar — installer 가 검증을 위해 참조하므로 먼저.
    let vanilla_client_jar = dirs.client_jar(&vanilla.id);
    run::fetch_client_jar(&vanilla, &vanilla_client_jar).await?;

    // 5. NeoForge 병합 + processors 실행
    let final_meta = if manifest.loader.kind.eq_ignore_ascii_case("neoforge") {
        status!("NeoForge {} installer 다운로드", manifest.loader.version);
        let installer_jar = dirs
            .cache
            .join(format!("neoforge-{}-installer.jar", manifest.loader.version));
        neoforge::fetch_installer(&manifest.loader.version, &installer_jar).await?;

        status!("NeoForge 설치 · processors 실행");
        let tx_i = tx.clone();
        neoforge::install_client(
            &installer_jar,
            &dirs.game,
            &java_result.javaw,
            |line| {
                // installer 로그를 그대로 Info 이벤트로 — UI 에서 라이브 프로그레스.
                let _ = tx_i.send(Event::Info(format!("[installer] {}", line)));
            },
        )
        .await?;

        let forge = neoforge::extract_version_json(&installer_jar)?;
        info!("NeoForge meta: id={}, main={}", forge.id, forge.main_class);
        neoforge::merge(forge, vanilla)
    } else {
        warn_!("알 수 없는 loader kind: {} — 바닐라로 런치", manifest.loader.kind);
        vanilla
    };

    // 6. 병합 후 추가 라이브러리 (url 빈 항목은 installer 가 이미 생성)
    status!("NeoForge 라이브러리 동기화");
    let plan_full = libraries::plan(&final_meta, &dirs.libraries);
    libraries::download_all(&plan_full).await?;

    // 7. mrpack 적용 (instance/ 루트)
    status!("모드팩 다운로드 · 적용");
    let mrpack_path: PathBuf = dirs
        .cache
        .join(format!("cherishpack-{}.mrpack", entry.version));
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

    let _applied = mrpack::apply(&mrpack_path, &dirs.instance, None)
        .await
        .context("mrpack 적용 실패")?;

    // 7.4. 프리셋 적용 (options.txt / iris.properties)
    let preset_key = opts.preset.as_deref().unwrap_or("medium");
    let _ = &manifest.hw_profiles; // 자체 런처에선 미사용 (미래 확장용)
    match crate::apply_preset::apply_for_self_launcher(&dirs.instance, preset_key) {
        Ok(_) => info!("프리셋 '{}' 적용", preset_key),
        Err(e) => warn_!("프리셋 적용 실패: {e:#}"),
    }

    // 7.5. 런치 캐시 저장 — 재실행 시 전체 sync 스킵용.
    let cache_entry = cache::LaunchCache {
        schema: cache::SCHEMA_VERSION,
        pack_version: entry.version.clone(),
        vanilla_id: manifest.minecraft.clone(),
        final_meta: final_meta.clone(),
        account: cache::CachedAccount {
            // MSA 경로에선 nickname 캐시 불필요(account.json 이 source of truth) — 기록용만.
            nickname: Some(msa_auth.profile.name.clone()),
        },
        channel: opts.channel.as_str().to_string(),
    };
    if let Err(e) = cache::save(&dirs.root, &cache_entry) {
        warn_!("launch-cache 저장 실패: {e:#}");
    } else {
        info!("launch-cache 저장 완료 — 다음 실행은 -l 로 빠르게");
    }

    // 7.6. 바탕화면·시작메뉴 바로가기 — CherishWorld.exe -l 로 지정.
    match install_shortcuts(&dirs) {
        Ok(_) => info!("바로가기 '체리쉬월드' 생성 (바탕화면·시작메뉴)"),
        Err(e) => warn_!("바로가기 생성 실패: {e:#}"),
    }

    // 8. 런치
    if !opts.auto_launch {
        let _ = tx.send(Event::Done { launched: false });
        return Ok(());
    }

    status!("런치");
    // NeoForge 는 `net/minecraft/client/<ver>/client-...jar` 를 자체 라이브러리로 가짐
    // → 바닐라 client.jar 를 또 얹으면 중복 export 로 JPMS 거부.
    let is_neoforge = manifest.loader.kind.eq_ignore_ascii_case("neoforge");
    let extra_classpath = if is_neoforge {
        Vec::new()
    } else {
        vec![vanilla_client_jar.clone()]
    };
    let layout = RuntimeLayout {
        dirs: &dirs,
        // natives 는 바닐라 MC 기준 — NeoForge 는 natives 를 추가하지 않는다.
        version_id: &manifest.minecraft,
        extra_classpath,
    };
    let uuid = msa_auth.profile.uuid_dashed();
    let account = run::Account {
        username: &msa_auth.profile.name,
        uuid: &uuid,
        access_token: &msa_auth.mc_access_token,
        user_type,
    };
    let launcher_info = run::LauncherInfo {
        name: "CherishWorld",
        version: env!("CARGO_PKG_VERSION"),
    };
    let ctx = run::LaunchContext {
        java: &java_result.javaw,
        layout: &layout,
        account: &account,
        launcher: &launcher_info,
    };

    let status = run::run(&final_meta, &plan_full, &ctx).await?;
    info!("Minecraft 종료 (exit={})", status);
    let _ = tx.send(Event::Done { launched: true });
    Ok(())
}

/// 계정 해석 — feature=offline 빌드에서 offline_nickname 이 주어졌으면 합성 계정,
/// 아니면 일반 MSA 경로.
async fn resolve_account(
    dirs: &LauncherDirs,
    tx: &tokio::sync::mpsc::UnboundedSender<Event>,
    opts: &RunOptions,
) -> Result<(auth::msa::Authenticated, &'static str)> {
    #[cfg(feature = "offline")]
    if let Some(nick) = opts.offline_nickname.as_deref() {
        let _ = tx.send(Event::Warn(format!(
            "⚠ OFFLINE 테스트 모드 — MSA 건너뛰고 '{}' 로 런치 (내부 빌드 전용)",
            nick
        )));
        return Ok((auth::offline::synthesize(nick), "legacy"));
    }
    let _ = opts; // feature 비활성 빌드에서 미사용 경고 억제
    let _ = tx.send(Event::Status("Microsoft 계정 확인".into()));
    let a = ensure_msa_account(dirs, tx).await?;
    Ok((a, "msa"))
}

// ─────────────────────── launch-only (fast relaunch) ───────────────────────

pub struct LaunchOnlyOptions {}

/// 전체 sync 없이 캐시된 meta 로 바로 런치. 실패 시 전체 설치 안내.
pub async fn run_launch_only(
    _opts: LaunchOnlyOptions,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
) {
    if let Err(e) = run_launch_only_inner(&tx).await {
        let _ = tx.send(Event::Error(format!("{e:#}")));
    }
}

async fn run_launch_only_inner(
    tx: &tokio::sync::mpsc::UnboundedSender<Event>,
) -> Result<()> {
    macro_rules! status {
        ($($arg:tt)*) => {{ let _ = tx.send(Event::Status(format!($($arg)*))); }};
    }
    macro_rules! info {
        ($($arg:tt)*) => {{ let _ = tx.send(Event::Info(format!($($arg)*))); }};
    }

    let dirs = LauncherDirs::resolve()?;
    let cache_entry = cache::load(&dirs.root).context(
        "launch-cache 없음 — 먼저 --launcher 로 전체 설치를 한번 실행하세요",
    )?;
    info!("캐시된 pack_version={} vanilla={}", cache_entry.pack_version, cache_entry.vanilla_id);

    // MSA 계정 확보 — refresh 실패/없음이면 device flow 로 재인증.
    status!("Microsoft 계정 확인");
    let msa_auth = ensure_msa_account(&dirs, tx).await?;
    info!("Minecraft 프로필: {}", msa_auth.profile.name);

    // 업데이트 체크 — 네트워크 실패해도 런치는 진행 (best-effort).
    check_update_nonblocking(&cache_entry, tx).await;

    let java_result = java::ensure_java_at(&dirs.java, &dirs.cache, None)
        .await
        .context("Java 확인 실패")?;

    status!("런치 준비");
    let plan = libraries::plan(&cache_entry.final_meta, &dirs.libraries);
    // 병합 meta id 가 vanilla id 와 다르면 NeoForge(또는 다른 loader) 가 끼어있는 것.
    let is_vanilla_only = cache_entry.final_meta.id == cache_entry.vanilla_id;
    let extra_classpath = if is_vanilla_only {
        vec![dirs.client_jar(&cache_entry.vanilla_id)]
    } else {
        Vec::new()
    };
    let layout = RuntimeLayout {
        dirs: &dirs,
        version_id: &cache_entry.vanilla_id,
        extra_classpath,
    };
    let uuid = msa_auth.profile.uuid_dashed();
    let account = run::Account {
        username: &msa_auth.profile.name,
        uuid: &uuid,
        access_token: &msa_auth.mc_access_token,
        user_type: "msa",
    };
    let launcher_info = run::LauncherInfo {
        name: "CherishWorld",
        version: env!("CARGO_PKG_VERSION"),
    };
    let ctx = run::LaunchContext {
        java: &java_result.javaw,
        layout: &layout,
        account: &account,
        launcher: &launcher_info,
    };

    status!("Minecraft 실행");
    let status = run::run(&cache_entry.final_meta, &plan, &ctx).await?;
    info!("Minecraft 종료 (exit={})", status);
    let _ = tx.send(Event::Done { launched: true });
    Ok(())
}

/// 원격 version.json 을 조회해 캐시보다 새 pack 이 있으면 경고 이벤트.
/// 타임아웃 ~5초, 실패 시 조용히 넘어감. 런치 자체를 막지 않는다.
async fn check_update_nonblocking(
    cache_entry: &cache::LaunchCache,
    tx: &tokio::sync::mpsc::UnboundedSender<Event>,
) {
    let fut = async {
        let index: VersionIndex = net::fetch_json(channel::VERSION_INDEX_URL).await.ok()?;
        let remote = if cache_entry.channel == "beta" {
            index.beta.unwrap_or(index.stable)
        } else {
            index.stable
        };
        Some(remote)
    };
    let remote = match tokio::time::timeout(std::time::Duration::from_secs(5), fut).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            tracing::debug!("업데이트 체크 실패 (네트워크) — 스킵");
            return;
        }
        Err(_) => {
            tracing::debug!("업데이트 체크 타임아웃 — 스킵");
            return;
        }
    };

    match crate::state::compare(&cache_entry.pack_version, &remote.version) {
        std::cmp::Ordering::Less => {
            let _ = tx.send(Event::Warn(format!(
                "새 팩 버전 {}  →  {} 사용 가능. --launcher 로 재설치하세요.",
                cache_entry.pack_version, remote.version
            )));
        }
        _ => {
            let _ = tx.send(Event::Info(format!(
                "최신 팩 확인 — {} (채널={})",
                remote.version, cache_entry.channel
            )));
        }
    }
}

/// 저장된 account.json 을 사용하거나, 없으면 device flow 로 새 인증.
async fn ensure_msa_account(
    dirs: &LauncherDirs,
    tx: &tokio::sync::mpsc::UnboundedSender<Event>,
) -> Result<auth::msa::Authenticated> {
    if let Some(a) = auth::account::load_and_refresh_if_needed(&dirs.root).await? {
        return Ok(a);
    }
    let _ = tx.send(Event::Info(
        "저장된 계정이 없습니다 — Microsoft 로그인 진행".into(),
    ));
    let tx_c = tx.clone();
    let a = auth::msa::login(move |ch| {
        let _ = tx_c.send(Event::AuthChallenge {
            user_code: ch.user_code.clone(),
            verification_uri: ch.verification_uri.clone(),
            expires_in: ch.expires_in,
        });
        // UX 개선 — 검증 URL 을 기본 브라우저로 자동 오픈. 실패해도 조용히 무시.
        open_in_browser(&ch.verification_uri);
    })
    .await?;
    if let Err(e) = auth::account::save(&dirs.root, &a) {
        let _ = tx.send(Event::Warn(format!("account.json 저장 실패: {e:#}")));
    }
    Ok(a)
}

/// 바탕화면·시작메뉴에 `CherishWorld.exe -l` 바로가기 설치.
///
/// 아이콘은 installer 바이너리에 내장된 ico 를 `dirs.root\cherishworld.ico` 에 풀어
/// LNK 에서 참조한다.
fn install_shortcuts(dirs: &LauncherDirs) -> Result<()> {
    let exe = std::env::current_exe().context("current_exe 조회 실패")?;

    // 아이콘 배치 — 설치기에 내장된 ico 그대로 사용.
    let icon_bytes: &[u8] = include_bytes!("../../resources/icon.ico");
    let icon_path = dirs.root.join("cherishworld.ico");
    let icon_opt = if std::fs::write(&icon_path, icon_bytes).is_ok() {
        Some(icon_path.as_path())
    } else {
        None
    };

    crate::shortcut::create_desktop_shortcut("체리쉬월드", &exe, "-l", &dirs.root, icon_opt)?;
    let _ = crate::shortcut::create_startmenu_shortcut(
        "체리쉬월드",
        &exe,
        "-l",
        &dirs.root,
        icon_opt,
    );
    Ok(())
}

/// Windows 의 기본 브라우저로 URL 을 연다. 실패하면 조용히 무시.
fn open_in_browser(url: &str) {
    #[cfg(windows)]
    {
        // `cmd /C start "" "<url>"` — start 의 첫 quoted 인자는 창 제목이므로 빈 문자열 필요.
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

fn estimate_asset_count(meta: &meta::VersionMeta) -> String {
    // 정확한 개수는 인덱스 다운로드 후에나 알 수 있음 — 대략적 안내.
    meta.asset_index.total_size.map(|s| format!("~{}MB", s / 1_000_000)).unwrap_or_default()
}
