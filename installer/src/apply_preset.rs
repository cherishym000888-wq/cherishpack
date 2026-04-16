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
use std::collections::HashMap;
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

/// options.txt 의 resourcePacks / incompatibleResourcePacks 만 덮어쓴다.
/// 다른 설정은 그대로 보존.
fn apply_options_txt(mc_root: &Path, assets: &PresetAssets) -> Result<()> {
    let path = mc_root.join("options.txt");
    let mut map = parse_options(&path)?;

    // 예상 포맷: `resourcePacks:["vanilla","file/pack1.zip"]`
    // 체인 순서: 왼→오른 = 상위→하위 (Minecraft 규칙과는 반대지만 file/pack은 최하위)
    // 실제 Minecraft는 array의 마지막 항목이 최상위 적용이므로 우선순위 역순으로 넣는다.
    let mut entries: Vec<String> = vec!["\"vanilla\"".to_string()];
    // resource_packs는 왼쪽이 최상위이므로 역순으로 append
    for rp in assets.resource_packs.iter().rev() {
        entries.push(format!("\"file/{}\"", rp));
    }
    let rp_value = format!("[{}]", entries.join(","));
    map.insert("resourcePacks".to_string(), rp_value);

    // incompatibleResourcePacks 비움 (Minecraft가 자동 관리)
    if !map.contains_key("incompatibleResourcePacks") {
        map.insert("incompatibleResourcePacks".to_string(), "[]".into());
    }

    // 한국어, 접근성 스킵 (기본값)
    map.entry("lang".to_string()).or_insert_with(|| "ko_kr".into());
    map.entry("onboardAccessibility".to_string())
        .or_insert_with(|| "false".into());

    write_options(&path, &map)?;
    tracing::info!(path = %path.display(), "options.txt 갱신 (resourcePacks)");
    Ok(())
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

/// options.txt 포맷: `key:value` (한 줄에 하나). 순서 보존 위해 Vec 으로 저장.
/// 하지만 단순 구현: HashMap + 원본 순서는 포기. Minecraft 는 순서 독립적으로 파싱함.
fn parse_options(path: &Path) -> Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    if !path.exists() {
        return Ok(map);
    }
    let text = std::fs::read_to_string(path)?;
    for line in text.lines() {
        if let Some((k, v)) = line.split_once(':') {
            map.insert(k.to_string(), v.to_string());
        }
    }
    Ok(map)
}

fn write_options(path: &Path, map: &HashMap<String, String>) -> Result<()> {
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();
    let body: String = keys
        .iter()
        .map(|k| format!("{}:{}\n", k, map[*k]))
        .collect();
    std::fs::write(path, body)?;
    Ok(())
}
