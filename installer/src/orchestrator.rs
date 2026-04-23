//! 설치/패치 전체 흐름.
//!
//! GUI 에서 `run()` 을 tokio task로 띄우고, 진행 상황은 `mpsc::Sender<Event>` 로 흘린다.

use anyhow::{bail, Context, Result};
use std::cmp::Ordering;

use crate::{
    channel::{self, Channel},
    config::{ChannelEntry, CurrentManifest, PackManifest, VersionIndex},
    apply_preset, java, mrpack, net,
    paths::AppDirs,
    patcher, prism, shortcut, state,
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
        ($($arg:tt)*) => {{ let _ = tx.send(Event::Status(format!($($arg)*))); }};
    }
    macro_rules! info {
        ($($arg:tt)*) => {{ let _ = tx.send(Event::Info(format!($($arg)*))); }};
    }
    macro_rules! warn_ {
        ($($arg:tt)*) => {{ let _ = tx.send(Event::Warn(format!($($arg)*))); }};
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

    // 4.5. mmc-pack.json (Prism 인스턴스 메타) — 매니페스트에서 마크/로더 버전 가져옴
    prism::write_mmc_pack(
        dirs,
        &manifest.minecraft,
        &manifest.loader.kind,
        &manifest.loader.version,
    )?;

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

    // 5.5. Java 21 보장
    status!("Java 21 확인 중");
    let tx_j = tx.clone();
    let java_result = java::ensure_java(
        dirs,
        Some(&move |d, t, label| {
            let _ = tx_j.send(Event::Progress {
                done: d,
                total: t,
                label: label.to_string(),
            });
        }),
    )
    .await
    .context("Java 설치 실패")?;
    if java_result.installed_now {
        info!("Java 21 새로 설치됨: {}", java_result.javaw.display());
    } else {
        info!("Java 21 확인: {}", java_result.javaw.display());
    }
    // Prism 인스턴스에 Java 경로 기록
    if let Err(e) = java::set_instance_java(dirs, &java_result.javaw) {
        warn_!("instance.cfg Java 경로 설정 실패: {e:#}");
    }

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

    // 11.5. 바탕화면 / 시작메뉴 바로가기 (실패해도 전체 성공을 막지 않음)
    //   아이콘은 설치기 바이너리에 내장된 icon.ico 를 prism 루트에 풀어 경로로 지정
    let icon_bytes: &[u8] = include_bytes!("../resources/icon.ico");
    let icon_path = dirs.prism_root.join("cherishpack.ico");
    if let Err(e) = std::fs::create_dir_all(&dirs.prism_root) {
        warn_!("prism_root 생성 실패: {e:#}");
    }
    if let Err(e) = std::fs::write(&icon_path, icon_bytes) {
        warn_!("아이콘 파일 기록 실패: {e:#}");
    }
    let icon_opt = if icon_path.exists() { Some(icon_path.as_path()) } else { None };

    let exe = dirs.prism_root.join("prismlauncher.exe");
    let args = format!("-l {}", crate::paths::INSTANCE_NAME);
    match shortcut::create_desktop_shortcut("체리쉬월드", &exe, &args, &dirs.prism_root, icon_opt) {
        Ok(_) => info!("바탕화면 바로가기 '체리쉬월드' 생성"),
        Err(e) => warn_!("바탕화면 바로가기 생성 실패: {e:#}"),
    }
    match shortcut::create_startmenu_shortcut("체리쉬월드", &exe, &args, &dirs.prism_root, icon_opt) {
        Ok(_) => info!("시작메뉴 바로가기 '체리쉬월드' 생성"),
        Err(e) => warn_!("시작메뉴 바로가기 생성 실패: {e:#}"),
    }

    // 11.6. 계정은 가져오지 않는다 — Prism client_id 가 다르면 토큰 refresh 가 실패해서
    //       'Account refresh failed' 팝업이 먼저 뜨는 나쁜 UX 가 됨. 신규 설치는 처음부터
    //       MSA 로그인하도록 둠.
    info!("Microsoft 로그인 모드 — Prism 창에서 로그인 필요");
    match prism::write_default_prism_cfg_if_missing(dirs) {
        Ok(true) => info!("prismlauncher.cfg 기본값 생성 (첫실행 마법사 스킵)"),
        Ok(false) => info!("기존 prismlauncher.cfg 유지"),
        Err(e) => warn_!("prismlauncher.cfg 작성 실패: {e:#}"),
    }
    if let Err(e) = prism::ensure_korean_translation(dirs).await {
        warn_!("한국어 번역 다운로드 실패(영어 UI 로 폴백): {e:#}");
    }
    if let Err(e) = prism::write_default_options_if_missing(dirs) {
        warn_!("options.txt 기본값 작성 실패: {e:#}");
    }

    // 11.7. 프리셋에 따라 리소스팩 + 쉐이더 선택 적용
    let preset_key = opts.preset.as_deref().unwrap_or("medium");
    match apply_preset::apply(dirs, preset_key) {
        Ok(_) => info!("프리셋 '{}' 적용 완료 (리소스팩·쉐이더 설정)", preset_key),
        Err(e) => warn_!("프리셋 적용 실패: {e:#}"),
    }

    // 11.8. 체리쉬월드 부팅 경험 — 핑크 로딩화면 + 부팅 BGM + MC 자체음악 뮤트
    //   (a) installer exe 복사 → Prism 의 PreLaunchCommand 로 재참조 (매 실행시 earlydisplay 재패치)
    //   (b) boot-agent.jar 배치 → instance.cfg 의 JvmArgs 에 -javaagent 추가
    //   (c) options.txt 의 music 카테고리 0.0 으로 뮤트
    match install_cherish_boot_experience(dirs) {
        Ok(_) => info!("체리쉬월드 부팅 경험(핑크 로딩·BGM·음악 뮤트) 구성 완료"),
        Err(e) => warn_!("부팅 경험 구성 실패 (게임은 정상 작동): {e:#}"),
    }

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

/// Prism 경로용 부팅 경험 설정.
///   - installer exe 를 `<prism>/cherishworld.exe` 로 사본 (PreLaunchCommand 안정 참조)
///   - boot-agent.jar 를 `<prism>/cherish-boot-agent.jar` 에 배치
///   - instance.cfg 에 OverrideJavaArgs=true + JvmArgs=-javaagent + PreLaunchCommand 추가
///   - options.txt 의 music 카테고리를 0.0 으로
fn install_cherish_boot_experience(dirs: &crate::paths::AppDirs) -> Result<()> {
    let exe_src = std::env::current_exe().context("current_exe 실패")?;
    let exe_dst = dirs.prism_root.join("cherishworld.exe");
    if exe_src != exe_dst {
        std::fs::copy(&exe_src, &exe_dst)
            .with_context(|| format!("installer 사본 복사 실패: {}", exe_dst.display()))?;
    }

    let agent_dst = dirs.prism_root.join("cherish-boot-agent.jar");
    crate::boot_agent::ensure_installed(&agent_dst)?;

    // Prism libraries 경로 — 공유 설치면 prism_root/libraries, 인스턴스 전용이면 instance_root/libraries
    // Prism 은 기본적으로 `<prism>/libraries/` 에 공유.
    let libs_dir = dirs.prism_root.join("libraries");

    // 경로에 스페이스가 있을 수 있으니 반드시 따옴표로 감쌈.
    let pre_launch_cmd = format!(
        "\"{}\" --patch-libs \"{}\"",
        exe_dst.display(),
        libs_dir.display()
    );
    let jvm_args = format!("-javaagent:{}", agent_dst.display());

    crate::prism::set_instance_cfg_kv(dirs, &[
        ("OverrideJavaArgs", "true"),
        ("JvmArgs", &jvm_args),
        ("OverrideCommands", "true"),
        ("PreLaunchCommand", &pre_launch_cmd),
    ])?;

    // MC 자체 music 뮤트 (자체 부팅 BGM 과 겹치지 않도록)
    if let Err(e) = crate::prism::mute_mc_music(dirs) {
        tracing::warn!("options.txt music 뮤트 실패: {e:#}");
    }
    Ok(())
}
