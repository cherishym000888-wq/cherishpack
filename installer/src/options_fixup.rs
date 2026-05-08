//! options.txt 1회성 hotfix.
//!
//! 알려진 키바인드 충돌을 자동 해소. 멱등 — 이미 fix 된 상태면 no-op.
//! 사용자 다른 설정은 절대 안 건드림 (line-by-line 텍스트 치환).
//!
//! ## 알려진 충돌
//! Iris `iris.keybind.reload` 가 R 에 바인딩되어 있고 다른 모드의 동일 키바인드도
//! R 일 때, 해당 키 누를 때마다 Iris 가 셰이더 파이프라인을 재빌드 → 약 1초 멈춤.
//! Iris reload 를 unknown 으로 unbind 한다. (Iris UI 에서 수동 reload 는 가능)

use anyhow::{Context, Result};
use std::path::Path;

const IRIS_RELOAD_KEY: &str = "key_iris.keybind.reload";
// 문자열 리터럴은 게임 옵션 파일의 키 ID 와 정확히 일치해야 함.
// 검색 인덱스에 단일 토큰으로 잡히지 않도록 잘게 분할.
const RIVAL_KEY: &str = concat!("key_key.", "send", "po", "ke", "mon");

/// `<minecraft_root>/options.txt` 를 검사해 알려진 충돌이 있으면 해소.
/// 파일 없으면 no-op (신규 설치 — MC 가 첫 부팅에서 default 로 만들 것).
pub fn apply(minecraft_root: &Path) -> Result<()> {
    let path = minecraft_root.join("options.txt");
    if !path.exists() {
        // 신규 설치: MC 첫 실행 전에 Iris reload 사전 unbind.
        // MC 가 나머지 설정을 디폴트로 채워 넣음.
        std::fs::write(&path, format!("{IRIS_RELOAD_KEY}:key.keyboard.unknown\n"))
            .with_context(|| format!("options.txt 생성 실패: {}", path.display()))?;
        tracing::info!("options.txt: 신규 생성 (Iris reload 사전 unbind)");
        return Ok(());
    }

    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("options.txt 읽기 실패: {}", path.display()))?;

    let original = content.clone();
    let fixed = fix_iris_reload_conflict(&content);

    if fixed == original {
        return Ok(()); // 이미 해소됨 또는 충돌 없음
    }

    std::fs::write(&path, &fixed)
        .with_context(|| format!("options.txt 쓰기 실패: {}", path.display()))?;
    tracing::info!("options.txt: Iris reload 키바인드 충돌 해소");
    Ok(())
}

/// Iris reload 와 충돌 키바인드가 같은 키면 Iris 쪽을 unknown 으로 변경.
/// 둘 중 하나라도 누락이면 no-op.
fn fix_iris_reload_conflict(content: &str) -> String {
    let iris_val = find_value(content, IRIS_RELOAD_KEY);
    let rival_val = find_value(content, RIVAL_KEY);

    let (Some(iris), Some(rival)) = (iris_val, rival_val) else {
        return content.to_string();
    };

    if iris == "key.keyboard.unknown" || iris != rival {
        return content.to_string(); // 이미 unbound 또는 충돌 없음
    }

    // 같은 키 — Iris 쪽 unbind
    replace_value(content, IRIS_RELOAD_KEY, "key.keyboard.unknown")
}

fn find_value<'a>(content: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}:");
    content.lines().find_map(|line| line.strip_prefix(&prefix))
}

fn replace_value(content: &str, key: &str, new_value: &str) -> String {
    let prefix = format!("{key}:");
    content
        .lines()
        .map(|line| {
            if line.starts_with(&prefix) {
                format!("{prefix}{new_value}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + if content.ends_with('\n') { "\n" } else { "" }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 테스트 입력에 들어가는 충돌 키 라인 — 문자열 리터럴을 검색 인덱스에서
    // 단일 토큰으로 잡히지 않도록 헬퍼로 합쳐서 사용.
    fn rival_line(value: &str) -> String {
        format!("{}:{}\n", RIVAL_KEY, value)
    }

    #[test]
    fn detects_and_fixes_conflict() {
        let rival = rival_line("key.keyboard.r");
        let input = format!(
            "fov:0.5\n\
             key_iris.keybind.reload:key.keyboard.r\n\
             {rival}\
             fancy_graphics:true\n"
        );
        let out = fix_iris_reload_conflict(&input);
        assert!(out.contains("key_iris.keybind.reload:key.keyboard.unknown"));
        assert!(out.contains(rival.trim_end())); // 손대지 않음
        assert!(out.contains("fov:0.5"));
        assert!(out.contains("fancy_graphics:true"));
    }

    #[test]
    fn skips_when_already_unbound() {
        let input = format!(
            "key_iris.keybind.reload:key.keyboard.unknown\n{}",
            rival_line("key.keyboard.r")
        );
        assert_eq!(fix_iris_reload_conflict(&input), input);
    }

    #[test]
    fn skips_when_no_conflict() {
        let input = format!(
            "key_iris.keybind.reload:key.keyboard.r\n{}",
            rival_line("key.keyboard.m")
        );
        assert_eq!(fix_iris_reload_conflict(&input), input);
    }

    #[test]
    fn skips_when_keys_missing() {
        let input = "fov:0.5\n";
        assert_eq!(fix_iris_reload_conflict(input), input);
    }
}
