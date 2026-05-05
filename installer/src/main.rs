//! CherishPack Installer — 진입점
//!
//! Phase 1 구현 범위:
//!  - 로깅 초기화
//!  - 경로·상태 디렉터리 준비
//!  - 원격 version.json 로드 (skeleton)
//!  - GUI 띄우기 (skeleton)
//!
//! 이후 Phase에서 prism / mrpack / patcher / gui 본체를 채운다.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod apply_preset;
mod channel;
mod config;
mod crash;
mod gui;
mod hash;
mod hwdetect;
mod java;
mod logger;
mod mrpack;
mod net;
mod orchestrator;
mod paths;
mod patcher;
mod preserve;
mod preset;
mod prism;
mod shortcut;
mod state;
mod uninstall;
mod boot_agent;
mod migrate;
mod options_fixup;
mod patch_early_display;

#[cfg(feature = "offline")]
mod launcher;

use anyhow::Result;
use tracing::{error, info};

fn main() -> Result<()> {
    // 0. 구 레이아웃 이사 (CherishPack → CherishWorld, launcher 서브폴더 격리)
    let _ = migrate::migrate();

    // 1. 경로 준비 (%LOCALAPPDATA%\CherishWorld\)
    let dirs = paths::AppDirs::resolve()?;
    dirs.ensure_exists()?;

    // 2. 로깅 초기화 (콘솔 + install.log 롤링)
    let _guard = logger::init(&dirs)?;

    info!(version = env!("CARGO_PKG_VERSION"), "CherishPack installer 시작");
    info!(?dirs, "경로 확인");

    // 3. CLI 인자 분기 (--uninstall 등)
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--uninstall") {
        return uninstall::run(&dirs);
    }

    // 3.1 Prism PreLaunchCommand 훅 — NeoForge earlydisplay 핑크 패치 재적용.
    //   `--patch-libs <libraries_dir>` 로 호출. Prism 이 게임 시작 전에 실행.
    //   빠르게 끝나고 exit 0 반환 (Prism 이 이어서 게임 launch).
    if let Some(idx) = args.iter().position(|a| a == "--patch-libs") {
        let libs = args.get(idx + 1).cloned().unwrap_or_default();
        if libs.is_empty() {
            eprintln!("--patch-libs 인자에 libraries 경로 필요");
            return Ok(());
        }
        match patch_early_display::apply_if_needed(std::path::Path::new(&libs)) {
            Ok(_) => info!("earlydisplay 핑크 패치 확인/적용 완료"),
            Err(e) => error!("earlydisplay 패치 실패 (게임 계속 진행): {e:#}"),
        }
        return Ok(());
    }

    // 3.2 바탕화면 바로가기 launcher wrapper — `--launch-game` 으로 호출되면
    //   (1) earlydisplay jar 패치 (fox/squirrel/Monocraft 리소스 교체) 후
    //   (2) Prism Launcher 를 인스턴스 자동실행 모드로 spawn 한 뒤 즉시 종료.
    //   prism 의 PreLaunchCommand 가 작동하지 않는 문제(prism 이 키를 인식 안 함)
    //   를 우회. boot-agent javaagent 의 ColourSchemeTransformer 가 색상 변환을,
    //   여기서 jar 패치가 fox/squirrel/font 교체를 담당.
    if args.iter().any(|a| a == "--launch-game") {
        let libs = dirs.prism_root.join("libraries");
        if let Err(e) = patch_early_display::apply_if_needed(&libs) {
            error!("earlydisplay 패치 실패 (계속 진행): {e:#}");
        }
        let prism_exe = dirs.prism_root.join("prismlauncher.exe");
        if let Err(e) = prism::spawn_detached(&prism_exe, &dirs.prism_root) {
            error!("Prism 실행 실패: {e:#}");
            return Err(e);
        }
        return Ok(());
    }

    // 3.5. (offline 빌드 한정) `--offline <nick>` — Prism 우회, 자체 런처로 헤드리스 실행.
    //      `-l` — launch-only (캐시된 닉네임으로 바로 게임 실행). 바탕화면 바로가기에서 사용.
    #[cfg(feature = "offline")]
    {
        if let Some(idx) = args.iter().position(|a| a == "--offline") {
            let nick = args.get(idx + 1).cloned().unwrap_or_else(|| "tester".into());
            return run_offline_headless(nick);
        }
        if args.iter().any(|a| a == "-l") {
            return run_offline_launch_only();
        }
    }

    // 4. GUI 실행 — 이후 Phase에서 실제 화면 구현
    if let Err(e) = gui::run(dirs) {
        error!(error = ?e, "GUI 종료 오류");
        return Err(e);
    }

    Ok(())
}

#[cfg(feature = "offline")]
fn run_offline_launch_only() -> Result<()> {
    use launcher::orchestrator::{run_launch_only_offline, Event};
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    rt.block_on(async move {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Event>();
        let h = tokio::spawn(run_launch_only_offline(tx));
        while let Some(ev) = rx.recv().await {
            match ev {
                Event::Status(s) => println!("[*] {s}"),
                Event::Info(s)   => if !s.is_empty() { println!("    {s}") },
                Event::Warn(s)   => println!("[!] {s}"),
                Event::Error(e)  => println!("[x] {e}"),
                Event::Done { .. } => println!("[v] 종료"),
                _ => {}
            }
        }
        let _ = h.await;
    });
    Ok(())
}

#[cfg(feature = "offline")]
fn run_offline_headless(nickname: String) -> Result<()> {
    use launcher::orchestrator::{run_launcher, Event, RunOptions};
    use channel::Channel;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Event>();
        let opts = RunOptions {
            channel: Channel::Stable,
            auto_launch: true,
            preset: Some("medium".into()),
            offline_nickname: Some(nickname),
        };
        let h = tokio::spawn(run_launcher(opts, tx));
        while let Some(ev) = rx.recv().await {
            match ev {
                Event::Status(s)   => println!("[*] {s}"),
                Event::Info(s)     => if !s.is_empty() { println!("    {s}") },
                Event::Warn(s)     => println!("[!] {s}"),
                Event::Progress { done, total, label } => {
                    match total {
                        Some(t) => println!("    {} {}/{}", label, done, t),
                        None    => println!("    {} {}", label, done),
                    }
                }
                Event::AuthChallenge { user_code, verification_uri, .. } => {
                    println!("[MSA] {} → {}", user_code, verification_uri);
                }
                Event::Done { launched } => { println!("[v] 완료 (launched={launched})"); }
                Event::Error(e) => { println!("[x] {e}"); }
            }
        }
        let _ = h.await;
    });
    Ok(())
}
