//! 사용자가 선택한 프리셋(low/medium/high)에 따라
//! options.txt 의 resource pack 목록과 iris.properties 의 shaderPack 을 설정한다.
//!
//! 동작 원칙:
//!  - **항상 덮어쓰지 않음**: 이미 있는 다른 설정(음량, 키 바인드 등)은 그대로 유지.
//!    특정 키만 교체/삽입.
//!  - 사용자가 인게임에서 리소스팩/쉐이더를 바꾸면 그 설정 존중 (다음 설치 시에도 덮지 않음).
//!    → 실제로는 매니페스트의 `preserve` 에 options.txt 가 들어있어 mrpack 적용 단계에서 보호됨.
//!      이 함수는 **최초 적용** 또는 **빈 값일 때만** 설정을 심는다.

use anyhow::Result;
use std::path::Path;

use crate::paths::AppDirs;

pub struct PresetAssets<'a> {
    /// 예: "ComplementaryReimagined.zip" — None이면 쉐이더 OFF
    pub shader_pack: Option<&'a str>,
    /// 리소스팩 파일명(확장자 포함, 상대는 resourcepacks/ 폴더). 왼쪽이 최상위(아래덮음).
    pub resource_packs: Vec<&'a str>,
}

/// 프리셋 이름("low"/"medium"/"high") → 실제 적용할 파일 세트.
pub fn preset_assets(preset: &str) -> PresetAssets<'static> {
    match preset {
        "high" => PresetAssets {
            shader_pack: Some("ComplementaryUnbound.zip"),
            resource_packs: vec!["FreshAnimations.zip", "SuperCute.zip"],
        },
        "medium" => PresetAssets {
            shader_pack: Some("ComplementaryReimagined.zip"),
            resource_packs: vec!["SuperCute.zip"],
        },
        _ => PresetAssets {
            shader_pack: None,
            resource_packs: vec!["SuperCute.zip"],
        },
    }
}

/// options.txt 와 iris.properties 에 프리셋을 적용한다.
pub fn apply(dirs: &AppDirs, preset: &str) -> Result<()> {
    let assets = preset_assets(preset);
    apply_options_txt(&dirs.minecraft_root, &assets)?;
    apply_iris_properties(&dirs.minecraft_root, &assets)?;
    Ok(())
}

/// options.txt 의 resourcePacks 항목만 정교하게 수정한다.
/// 전략:
///   1. 기존 resourcePacks 배열에서 **file/ 로 시작하는 항목은 제거** (구 프리셋 잔재 청소)
///   2. 그 외 항목(vanilla, mod_resources, moonlight:merged_pack 등)은 **보존**
///   3. 내 프리셋의 file/ 항목들을 **가장 뒤**에 추가 (우선순위 최상)
///   4. FreshAnimations 같은 mob 애니메이션 팩을 마지막에 두면 SuperCute 위에서 덮어씀
///
/// 파일 전체는 **원본 순서·형식 유지**하며 해당 한 줄만 교체.
fn apply_options_txt(mc_root: &Path, assets: &PresetAssets) -> Result<()> {
    let path = mc_root.join("options.txt");

    let mut lines: Vec<String> = if path.exists() {
        std::fs::read_to_string(&path)?.lines().map(String::from).collect()
    } else {
        Vec::new()
    };

    let new_rp_line = build_resource_packs_line(&lines, assets);
    set_line(&mut lines, "resourcePacks", &new_rp_line);

    // incompatibleResourcePacks 가 없으면 빈 배열로 초기화
    if !lines.iter().any(|l| l.starts_with("incompatibleResourcePacks:")) {
        lines.push("incompatibleResourcePacks:[]".into());
    }
    // lang / onboardAccessibility 없으면 추가 (있으면 사용자 선택 존중)
    if !lines.iter().any(|l| l.starts_with("lang:")) {
        lines.push("lang:ko_kr".into());
    }
    if !lines.iter().any(|l| l.starts_with("onboardAccessibility:")) {
        lines.push("onboardAccessibility:false".into());
    }

    std::fs::write(&path, lines.join("\n") + "\n")?;
    tracing::info!(path = %path.display(), "options.txt resourcePacks 갱신");
    Ok(())
}

/// 기존 resourcePacks 줄을 파싱해서 file/ 이 아닌 엔트리만 보존하고,
/// 프리셋의 새 file/ 엔트리를 뒤에 추가.
fn build_resource_packs_line(lines: &[String], assets: &PresetAssets) -> String {
    // 기존 줄 찾기
    let existing_value = lines
        .iter()
        .find_map(|l| l.strip_prefix("resourcePacks:"))
        .unwrap_or("[\"vanilla\"]");

    // 배열 내부 파싱 — 단순 분리 (JSON 파서 쓰기엔 과함)
    let inner = existing_value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']');
    let existing_entries: Vec<String> = if inner.trim().is_empty() {
        Vec::new()
    } else {
        inner
            .split(',')
            .map(|s| s.trim().trim_matches('"').to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };

    // file/ 로 시작하는 엔트리 제거
    let mut kept: Vec<String> = existing_entries
        .into_iter()
        .filter(|e| !e.starts_with("file/"))
        .collect();
    // vanilla 가 없으면 맨 앞에 추가
    if !kept.iter().any(|e| e == "vanilla") {
        kept.insert(0, "vanilla".to_string());
    }

    // 프리셋 리소스팩 추가 — resource_packs는 왼쪽이 우선순위 최상이므로,
    // Minecraft 배열 마지막 = 최상위 라서 역순으로 append
    for rp in assets.resource_packs.iter().rev() {
        kept.push(format!("file/{}", rp));
    }

    // 따옴표로 감싸서 배열 포맷으로 직렬화
    let quoted: Vec<String> = kept.iter().map(|e| format!("\"{}\"", e)).collect();
    format!("[{}]", quoted.join(","))
}

/// 특정 key 로 시작하는 줄이 있으면 교체, 없으면 끝에 추가.
fn set_line(lines: &mut Vec<String>, key: &str, value: &str) {
    let prefix = format!("{key}:");
    let new_line = format!("{key}:{value}");
    if let Some(pos) = lines.iter().position(|l| l.starts_with(&prefix)) {
        lines[pos] = new_line;
    } else {
        lines.push(new_line);
    }
}

/// `config/iris.properties` 에 `shaderPack` 기록. 없으면 새로 생성.
fn apply_iris_properties(mc_root: &Path, assets: &PresetAssets) -> Result<()> {
    let path = mc_root.join("config").join("iris.properties");
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }

    let mut lines: Vec<String> = if path.exists() {
        std::fs::read_to_string(&path)?
            .lines()
            .map(|l| l.to_string())
            .collect()
    } else {
        Vec::new()
    };

    let shader_value = assets.shader_pack.unwrap_or("(internal)");
    let shader_line = format!("shaderPack={shader_value}");
    let enable_shaders_line = format!(
        "enableShaders={}",
        if assets.shader_pack.is_some() {
            "true"
        } else {
            "false"
        }
    );

    set_or_insert(&mut lines, "shaderPack", &shader_line);
    set_or_insert(&mut lines, "enableShaders", &enable_shaders_line);

    let out = lines.join("\n") + "\n";
    std::fs::write(&path, out)?;
    tracing::info!(path = %path.display(), shader = shader_value, "iris.properties 갱신");
    Ok(())
}

fn set_or_insert(lines: &mut Vec<String>, key: &str, full_line: &str) {
    let prefix = format!("{key}=");
    if let Some(pos) = lines.iter().position(|l| l.starts_with(&prefix)) {
        lines[pos] = full_line.to_string();
    } else {
        lines.push(full_line.to_string());
    }
}

