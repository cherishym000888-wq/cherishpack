//! 바닐라 Minecraft 실행기.
//!
//! version.json 의 `arguments.game` / `arguments.jvm` 을 평가하고 토큰을
//! 치환해 `java` 프로세스를 스폰한다. `--demo` 플래그는 여기서 절대
//! 주입하지 않는다 — Prism 우회의 핵심 목적.
//!
//! NeoForge 는 Day 2 후반에 추가 — 이 모듈은 바닐라 경로까지만 처리.

use anyhow::{Context, Result};
use std::{collections::HashMap, path::Path};
use tokio::process::Command;

use super::dirs::RuntimeLayout;
use super::libraries::{rules_allow, LibraryPlan};
use super::meta::{ArgEntry, ArgValue, VersionMeta};

/// 계정 정보 — offline / MSA 공용.
pub struct Account<'a> {
    pub username: &'a str,
    pub uuid: &'a str,
    /// Offline 계정이면 "0".
    pub access_token: &'a str,
    pub user_type: &'a str, // "legacy" | "msa"
}

/// 런처 식별 정보 (telemetry 토큰 치환 용).
pub struct LauncherInfo<'a> {
    pub name: &'a str,
    pub version: &'a str,
}

/// 런치에 필요한 모든 런타임 정보 묶음.
pub struct LaunchContext<'a> {
    pub java: &'a Path,
    pub layout: &'a RuntimeLayout<'a>,
    pub account: &'a Account<'a>,
    pub launcher: &'a LauncherInfo<'a>,
}

/// client.jar 다운로드 (sha1 검증, 기존 파일 일치시 스킵).
pub async fn fetch_client_jar(meta: &VersionMeta, dst: &Path) -> Result<()> {
    if let Some(parent) = dst.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    let expected = &meta.downloads.client.sha1;

    if dst.exists() {
        if let Ok(got) = crate::hash::sha1_file(dst) {
            if got.eq_ignore_ascii_case(expected) {
                return Ok(());
            }
        }
    }

    let bytes = crate::net::fetch_bytes(&meta.downloads.client.url)
        .await
        .context("client.jar 다운로드 실패")?;

    use sha1::{Digest, Sha1};
    let mut h = Sha1::new();
    h.update(&bytes);
    let got = hex::encode(h.finalize());
    if !got.eq_ignore_ascii_case(expected) {
        anyhow::bail!("client.jar sha1 불일치: expected={}, got={}", expected, got);
    }
    tokio::fs::write(dst, &bytes).await?;
    Ok(())
}

/// options.txt 의 `soundCategory_music:` 값을 0.0 으로 설정. 없으면 추가.
/// 다른 사운드 카테고리(master/record/weather/block/...) 은 건드리지 않음.
fn mute_mc_music_if_needed(mc_root: &std::path::Path) {
    let path = mc_root.join("options.txt");
    let Ok(content) = std::fs::read_to_string(&path) else { return; };
    let mut found = false;
    let mut changed = false;
    let mut lines: Vec<String> = content.lines().map(|l| {
        if l.starts_with("soundCategory_music:") {
            found = true;
            if l != "soundCategory_music:0.0" {
                changed = true;
                "soundCategory_music:0.0".into()
            } else {
                l.to_string()
            }
        } else {
            l.to_string()
        }
    }).collect();
    if !found {
        lines.push("soundCategory_music:0.0".into());
        changed = true;
    }
    if changed {
        let _ = std::fs::write(&path, lines.join("\n") + "\n");
        tracing::info!("options.txt soundCategory_music 0.0 설정 (BGM agent 충돌 회피)");
    }
}

/// 최종 java 명령 조립. 프로세스는 아직 스폰하지 않는다.
pub fn build_command(
    meta: &VersionMeta,
    plan: &LibraryPlan,
    ctx: &LaunchContext<'_>,
) -> Result<Command> {
    let classpath = plan.classpath(ctx.layout.extra_classpath());
    let tokens = build_tokens(meta, ctx, &classpath);
    let features: HashMap<String, bool> = HashMap::new();

    let args = meta
        .arguments
        .as_ref()
        .context("arguments 필드 없음 — 1.13 이전 버전은 미지원")?;

    let mut jvm_args: Vec<String> = Vec::new();
    for a in &args.jvm {
        collect_arg(a, &tokens, &features, &mut jvm_args);
    }

    // NeoForge earlydisplay jar 재패치 — 라이브러리 다운로드 시 sha1 검증으로
    // 롤백되므로 java 실행 직전 매번 확인 후 재적용.
    if let Err(e) = crate::patch_early_display::apply_if_needed(ctx.layout.libraries_dir()) {
        tracing::warn!("earlydisplay 패치 실패 (계속 진행): {e:#}");
    }

    // options.txt 의 music volume 을 0 으로 — boot-agent BGM 과 겹치지 않게.
    mute_mc_music_if_needed(ctx.layout.dirs.instance.as_path());

    // 부팅 BGM agent — exe 에 embed 된 jar 를 매번 root 에 배치.
    let agent_path = ctx.layout.dirs.root.join("boot-agent.jar");
    if let Err(e) = crate::boot_agent::ensure_installed(&agent_path) {
        tracing::warn!("boot-agent 배치 실패: {e:#}");
    }
    if agent_path.exists() {
        jvm_args.insert(0, format!("-javaagent:{}", agent_path.display()));
    }
    // mainClass 는 jvm 인자 뒤, game 인자 앞.
    let mut game_args: Vec<String> = Vec::new();
    for a in &args.game {
        collect_arg(a, &tokens, &features, &mut game_args);
    }

    // 안전장치: --demo 가 어떤 경로로든 끼어들면 제거.
    jvm_args.retain(|a| a != "--demo");
    game_args.retain(|a| a != "--demo");

    // ZGC 로 GC pause 최소화 — Java 21 generational ZGC.
    // 기존 G1 관련 인자 제거 후 ZGC 인자 추가.
    jvm_args.retain(|a|
        !a.starts_with("-XX:+UseG1GC")
        && !a.starts_with("-XX:+UseZGC")
        && !a.starts_with("-XX:+ZGenerational")
    );
    jvm_args.push("-XX:+UseZGC".into());
    jvm_args.push("-XX:+ZGenerational".into());

    let mut cmd = Command::new(ctx.java);
    cmd.current_dir(ctx.layout.game_dir());
    cmd.args(&jvm_args);
    cmd.arg(&meta.main_class);
    cmd.args(&game_args);

    tracing::info!(
        main_class = %meta.main_class,
        jvm_args = jvm_args.len(),
        game_args = game_args.len(),
        "java 명령 조립 완료",
    );
    Ok(cmd)
}

/// 스폰 + 종료까지 대기.
pub async fn run(
    meta: &VersionMeta,
    plan: &LibraryPlan,
    ctx: &LaunchContext<'_>,
) -> Result<std::process::ExitStatus> {
    let mut cmd = build_command(meta, plan, ctx)?;
    let status = cmd.status().await.context("java 프로세스 실행 실패")?;
    Ok(status)
}

/// 자식 프로세스 spawn 만. wait 는 호출자가 자유롭게 처리.
///
/// 사용 흐름:
///   1. spawn 으로 Child 받음
///   2. 즉시 GUI 에 Done emit (설치기는 \"설치 완료\" 화면 전환)
///   3. child.wait().await 로 백그라운드 종료 대기 (선택)
///
/// 설치기 창은 이미 \"설치 완료\" 상태라 사용자가 \"종료\" 버튼으로 닫을 수 있고,
/// 게임은 별도 process 로 살아있다.
pub async fn spawn(
    meta: &VersionMeta,
    plan: &LibraryPlan,
    ctx: &LaunchContext<'_>,
) -> Result<tokio::process::Child> {
    let mut cmd = build_command(meta, plan, ctx)?;
    let child = cmd.spawn().context("java 프로세스 실행 실패")?;
    Ok(child)
}

// ─────────────────────── 내부 ───────────────────────

fn collect_arg(
    entry: &ArgEntry,
    tokens: &HashMap<&str, String>,
    features: &HashMap<String, bool>,
    out: &mut Vec<String>,
) {
    match entry {
        ArgEntry::Simple(s) => out.push(substitute(s, tokens)),
        ArgEntry::Conditional { rules, value } => {
            if !rules_allow(rules, features) {
                return;
            }
            match value {
                ArgValue::One(s) => out.push(substitute(s, tokens)),
                ArgValue::Many(v) => {
                    for s in v {
                        out.push(substitute(s, tokens));
                    }
                }
            }
        }
    }
}

/// `${token}` 단순 치환. 알 수 없는 토큰은 원문 유지 (경고 로그).
fn substitute(input: &str, tokens: &HashMap<&str, String>) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'$' && bytes[i + 1] == b'{' {
            if let Some(end) = input[i + 2..].find('}') {
                let key = &input[i + 2..i + 2 + end];
                if let Some(v) = tokens.get(key) {
                    out.push_str(v);
                } else {
                    tracing::warn!(token = key, "알 수 없는 런치 토큰 — 원문 유지");
                    out.push_str("${");
                    out.push_str(key);
                    out.push('}');
                }
                i += 2 + end + 1;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn build_tokens<'a>(
    meta: &'a VersionMeta,
    ctx: &LaunchContext<'_>,
    classpath: &str,
) -> HashMap<&'a str, String> {
    let mut t: HashMap<&str, String> = HashMap::new();
    let acct = ctx.account;
    let layout = ctx.layout;

    // 계정
    t.insert("auth_player_name", acct.username.to_string());
    t.insert("auth_uuid", acct.uuid.to_string());
    t.insert("auth_access_token", acct.access_token.to_string());
    t.insert("auth_xuid", String::new());
    t.insert("clientid", String::new());
    t.insert("user_type", acct.user_type.to_string());
    t.insert("user_properties", "{}".to_string());

    // 버전/경로
    t.insert("version_name", meta.id.clone());
    t.insert(
        "version_type",
        meta.kind.clone().unwrap_or_else(|| "release".to_string()),
    );
    t.insert("game_directory", layout.game_dir().to_string_lossy().into_owned());
    t.insert("assets_root", layout.assets_root().to_string_lossy().into_owned());
    t.insert("game_assets", layout.assets_root().to_string_lossy().into_owned());
    t.insert("assets_index_name", meta.asset_index.id.clone());
    t.insert(
        "natives_directory",
        layout.natives_dir().to_string_lossy().into_owned(),
    );
    t.insert(
        "library_directory",
        layout.libraries_dir().to_string_lossy().into_owned(),
    );
    t.insert("classpath", classpath.to_string());
    t.insert(
        "classpath_separator",
        if cfg!(windows) { ";".into() } else { ":".into() },
    );

    // 런처 자기정보
    t.insert("launcher_name", ctx.launcher.name.to_string());
    t.insert("launcher_version", ctx.launcher.version.to_string());

    // piston-meta 일부 버전에서 쓰는 resolution 토큰 — 기능 비활성이라 참조되지 않음.
    t.insert("resolution_width", "854".into());
    t.insert("resolution_height", "480".into());

    t
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitute_basic() {
        let mut t = HashMap::new();
        t.insert("name", "Arceus".to_string());
        t.insert("ver", "1.21.1".to_string());
        assert_eq!(substitute("hello ${name} / ${ver}!", &t), "hello Arceus / 1.21.1!");
    }

    #[test]
    fn substitute_unknown_preserves() {
        let t = HashMap::new();
        assert_eq!(substitute("${nope}", &t), "${nope}");
    }

    #[test]
    fn substitute_no_tokens() {
        let t = HashMap::new();
        assert_eq!(substitute("plain text $ { not a token", &t), "plain text $ { not a token");
    }
}
