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

#[cfg(feature = "offline")]
mod launcher;

use anyhow::Result;
use tracing::{error, info};

fn main() -> Result<()> {
    // 1. 경로 준비 (%LOCALAPPDATA%\CherishPack\)
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
