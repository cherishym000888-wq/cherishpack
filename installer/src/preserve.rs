//! 영구 보존 파일 목록 + glob 매칭.
//!
//! 매니페스트의 `preserve` 배열을 반영하되, 매니페스트가 빼먹어도
//! 아래 하드코드 목록은 **항상** 보존한다 (안전장치).

pub const HARDCODED_PRESERVE: &[&str] = &[
    "options.txt",
    "optionsof.txt",
    "optionsshaders.txt",
    "servers.dat",
    "servers.dat_old",
    "saves/**",
    "screenshots/**",
    "logs/**",
    "crash-reports/**",
    "resourcepacks/user/**", // 사용자가 직접 추가한 팩 관습적 위치
];

/// 아주 단순한 glob 매처. `*` = 세그먼트 내 임의, `**` = 경로 전체.
/// 상대경로(슬래시)로 통일해서 비교한다.
pub fn matches_any(relpath: &str, patterns: &[&str]) -> bool {
    let rel = relpath.replace('\\', "/");
    patterns.iter().any(|p| glob_match(p, &rel))
}

pub fn matches_any_owned(relpath: &str, patterns: &[String]) -> bool {
    let rel = relpath.replace('\\', "/");
    patterns.iter().any(|p| glob_match(p, &rel))
}

fn glob_match(pattern: &str, text: &str) -> bool {
    // ** 지원을 위해 세그먼트 단위로 매칭
    let pat_segs: Vec<&str> = pattern.split('/').collect();
    let txt_segs: Vec<&str> = text.split('/').collect();
    seg_match(&pat_segs, &txt_segs)
}

fn seg_match(pat: &[&str], txt: &[&str]) -> bool {
    match (pat.first(), txt.first()) {
        (None, None) => true,
        (None, Some(_)) => false,
        (Some(&"**"), _) => {
            // ** 는 0개 이상 세그먼트 매칭
            (0..=txt.len()).any(|k| seg_match(&pat[1..], &txt[k..]))
        }
        (Some(_), None) => false,
        (Some(p), Some(t)) => star_match(p, t) && seg_match(&pat[1..], &txt[1..]),
    }
}

fn star_match(pattern: &str, text: &str) -> bool {
    // 한 세그먼트 내 * 매칭
    let mut pi = pattern.chars().peekable();
    let mut ti = text.chars().peekable();
    star_match_iter(&mut pi, &mut ti)
}

fn star_match_iter(
    pat: &mut std::iter::Peekable<std::str::Chars>,
    txt: &mut std::iter::Peekable<std::str::Chars>,
) -> bool {
    loop {
        match pat.next() {
            None => return txt.peek().is_none(),
            Some('*') => {
                let rest_pat: String = pat.clone().collect();
                let rest_txt: String = txt.clone().collect();
                for i in 0..=rest_txt.len() {
                    if !rest_txt.is_char_boundary(i) {
                        continue;
                    }
                    let sub = &rest_txt[i..];
                    let mut p2 = rest_pat.chars().peekable();
                    let mut t2 = sub.chars().peekable();
                    if star_match_iter(&mut p2, &mut t2) {
                        return true;
                    }
                }
                return false;
            }
            Some(pc) => match txt.next() {
                Some(tc) if tc.eq_ignore_ascii_case(&pc) => continue,
                _ => return false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_preserve() {
        assert!(matches_any("options.txt", HARDCODED_PRESERVE));
        assert!(matches_any("saves/world1/level.dat", HARDCODED_PRESERVE));
        assert!(matches_any("screenshots/2026-04-16.png", HARDCODED_PRESERVE));
        assert!(!matches_any("mods/sodium.jar", HARDCODED_PRESERVE));
        assert!(!matches_any("config/sodium.json", HARDCODED_PRESERVE));
    }
}
