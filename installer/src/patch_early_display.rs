//! NeoForge earlydisplay jar 자동 패치.
//!
//! NeoForge 의 빨간 부팅화면을 동화풍 파스텔로 바꾼다. 라이브러리 재다운로드
//! (sha1 검증 실패) 시 패치가 롤백되므로, 매 실행 전 호출해 자동 복원.
//!
//! ## 동작 원리
//! `ColourScheme.<clinit>` 에서 RED enum 을 만드는 sipush/bipush 시퀀스를 직접
//! 교체한다. JVM Code attribute 의 `code_length` 를 건드리지 않으려고
//! **BG·FG 합산 바이트 길이는 16 으로 고정** (channel 6 개 중 정확히 2 개가
//! `< 128` 이어서 bipush 로 인코딩되도록 팔레트를 골라야 한다).
//!
//! `ColourScheme.foreground` 는 텍스트 + 진행 게이지 색으로 동시에 쓰이므로
//! FG 한 값으로 두 가지가 같이 바뀐다.
//!
//! ## 업그레이드 안전
//! v0 (NeoForge 원본 RED) → v2, v1 (구 핑크 배포본) → v2 둘 다 인식.

use anyhow::{Context, Result};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const TARGET_CLASS: &str = "net/neoforged/fml/earlydisplay/ColourScheme.class";

// === Phase 1 리소스 교체 (jar 내부 파일 통째로 swap) ===
// 폰트: Monocraft → Cafe24 Ssurround (동화풍). ASCII 95글자만 atlas 에 굽기 때문에
// 한글은 안 나오지만 영어 텍스트의 시각적 톤이 부드러워진다.
const FONT_TARGET: &str = "Monocraft.ttf";
const FONT_BYTES: &[u8] = include_bytes!("../assets/cafe24ssurround.ttf");

// 여우/다람쥐 제거 — 동일 차원 투명 PNG 로 교체.
// 차원이 같아야 STBImage 디코딩과 fox 의 14프레임 strip 가정이 안전하게 유지됨.
const FOX_TARGET: &str = "fox_running.png";
const FOX_BYTES: &[u8] = include_bytes!("../assets/fox_running_blank.png");
const SQUIR_TARGET: &str = "squirrel.png";
const SQUIR_BYTES: &[u8] = include_bytes!("../assets/squirrel_blank.png");

// === v0: NeoForge 원본 RED scheme ===
// BG (239,50,61) FG (255,255,255)
const V0_BG: &[u8] = &[0x11, 0x00, 0xEF, 0x10, 0x32, 0x10, 0x3D]; // 7 bytes
const V0_FG: &[u8] = &[0x11, 0x00, 0xFF, 0x11, 0x00, 0xFF, 0x11, 0x00, 0xFF]; // 9

// === v1: 구 배포본 (페일 핑크 + 딥 베리) ===
// BG (255,214,232) FG (128,40,70)
const V1_BG: &[u8] = &[0x11, 0x00, 0xFF, 0x11, 0x00, 0xD6, 0x11, 0x00, 0xE8]; // 9
const V1_FG: &[u8] = &[0x11, 0x00, 0x80, 0x10, 0x28, 0x10, 0x46]; // 7

// === v2: 현재 타깃 — 동화풍 파스텔 ===
// BG (250,240,248) 페일 페탈, FG (155,95,125) 더스티 모브 로즈
const V2_BG: &[u8] = &[0x11, 0x00, 0xFA, 0x11, 0x00, 0xF0, 0x11, 0x00, 0xF8]; // 9
const V2_FG: &[u8] = &[0x11, 0x00, 0x9B, 0x10, 0x5F, 0x10, 0x7D]; // 7

/// `<libraries>/net/neoforged/fancymodloader/earlydisplay/<version>/earlydisplay-<version>.jar`
/// 를 찾아 ColourScheme 색을 v2 로 패치. 이미 v2 면 스킵.
pub fn apply_if_needed(libraries_dir: &Path) -> Result<()> {
    let jar = match find_jar(libraries_dir) {
        Some(p) => p,
        None => return Ok(()), // earlydisplay 없으면 스킵 (서버·offline 모드)
    };

    let bytes = fs::read(&jar).with_context(|| format!("jar 읽기 실패: {}", jar.display()))?;
    let cursor = std::io::Cursor::new(&bytes);
    let mut archive = zip::ZipArchive::new(cursor)?;

    // 1) ColourScheme 바이트코드 패치
    let mut class_bytes = Vec::new();
    {
        let mut entry = archive
            .by_name(TARGET_CLASS)
            .with_context(|| format!("{TARGET_CLASS} 엔트리 없음"))?;
        entry.read_to_end(&mut class_bytes)?;
    }
    let class_already_v2 =
        find_bytes(&class_bytes, V2_BG).is_some() && find_bytes(&class_bytes, V2_FG).is_some();
    let new_class = if class_already_v2 {
        None
    } else if let Some(p) = try_upgrade(&class_bytes, V0_BG, V0_FG) {
        Some(p)
    } else if let Some(p) = try_upgrade(&class_bytes, V1_BG, V1_FG) {
        Some(p)
    } else {
        None // 알려진 패턴 없음 — 다른 NeoForge 버전. 클래스 패치만 스킵.
    };

    // 2) 폰트 / 여우 / 다람쥐 리소스 비교 — 다르면 교체
    let font_needs_swap = !entry_matches(&mut archive, FONT_TARGET, FONT_BYTES);
    let fox_needs_swap = !entry_matches(&mut archive, FOX_TARGET, FOX_BYTES);
    let squir_needs_swap = !entry_matches(&mut archive, SQUIR_TARGET, SQUIR_BYTES);

    if new_class.is_none() && !font_needs_swap && !fox_needs_swap && !squir_needs_swap {
        return Ok(()); // 이미 모두 적용된 상태
    }

    let mut replacements: Vec<(&str, &[u8])> = Vec::new();
    let class_buf;
    if let Some(p) = new_class {
        class_buf = p;
        replacements.push((TARGET_CLASS, &class_buf));
    }
    if font_needs_swap { replacements.push((FONT_TARGET, FONT_BYTES)); }
    if fox_needs_swap { replacements.push((FOX_TARGET, FOX_BYTES)); }
    if squir_needs_swap { replacements.push((SQUIR_TARGET, SQUIR_BYTES)); }

    let tmp = jar.with_extension("jar.patch.tmp");
    write_jar_replacing(&jar, &tmp, &replacements)?;
    fs::rename(&tmp, &jar)?;
    tracing::info!(
        "earlydisplay jar 패치 — colour:{} font:{} fox:{} squir:{}",
        if class_already_v2 { "v2(skip)" } else { "v2" },
        font_needs_swap, fox_needs_swap, squir_needs_swap
    );
    Ok(())
}

/// `class_bytes` 에서 `(old_bg, old_fg)` 쌍을 찾아 `(V2_BG, V2_FG)` 로 교체.
/// 패턴이 없으면 None.
///
/// FG 가 BG 보다 뒤에 위치한다고 가정 (실제 `<clinit>` 에서 그러함).
/// FG 를 먼저 splice 해서 BG 인덱스가 흔들리지 않게 한다.
fn try_upgrade(class_bytes: &[u8], old_bg: &[u8], old_fg: &[u8]) -> Option<Vec<u8>> {
    let bg_idx = find_bytes(class_bytes, old_bg)?;
    let fg_idx = find_bytes(&class_bytes[bg_idx + old_bg.len()..], old_fg)
        .map(|i| bg_idx + old_bg.len() + i)?;

    // 길이 합 보존 검증 (16 bytes) — 깨지면 Code attribute 가 어긋남
    debug_assert_eq!(old_bg.len() + old_fg.len(), V2_BG.len() + V2_FG.len());

    let mut patched = class_bytes.to_vec();
    patched.splice(fg_idx..fg_idx + old_fg.len(), V2_FG.iter().copied());
    patched.splice(bg_idx..bg_idx + old_bg.len(), V2_BG.iter().copied());
    Some(patched)
}

fn find_jar(libraries_dir: &Path) -> Option<PathBuf> {
    let base = libraries_dir.join("net/neoforged/fancymodloader/earlydisplay");
    let entries = fs::read_dir(&base).ok()?;
    for version_dir in entries.flatten() {
        let p = version_dir.path();
        if !p.is_dir() { continue; }
        if let Ok(inner) = fs::read_dir(&p) {
            for f in inner.flatten() {
                let fp = f.path();
                if fp.extension().and_then(|s| s.to_str()) == Some("jar") {
                    return Some(fp);
                }
            }
        }
    }
    None
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// `replacements` 의 이름과 일치하는 엔트리를 새 바이트로 교체.
/// 그 외 엔트리는 raw_copy_file 로 무손실 복사 (압축 메서드·CRC 보존).
fn write_jar_replacing(src: &Path, dst: &Path, replacements: &[(&str, &[u8])]) -> Result<()> {
    let src_bytes = fs::read(src)?;
    let src_cursor = std::io::Cursor::new(&src_bytes);
    let mut src_archive = zip::ZipArchive::new(src_cursor)?;

    let dst_file = fs::File::create(dst)?;
    let mut out = zip::ZipWriter::new(std::io::BufWriter::new(dst_file));
    let opts = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    for i in 0..src_archive.len() {
        let entry = src_archive.by_index(i)?;
        let name = entry.name().to_string();
        if let Some(&(_, new_bytes)) = replacements.iter().find(|(n, _)| *n == name) {
            drop(entry);
            out.start_file(&name, opts)?;
            out.write_all(new_bytes)?;
        } else {
            out.raw_copy_file(entry)?;
        }
    }
    out.finish()?;
    Ok(())
}

/// 아카이브 안의 `name` 엔트리 내용이 `expected` 와 동일한지 검사.
/// 엔트리가 없거나 다르면 false.
fn entry_matches(archive: &mut zip::ZipArchive<std::io::Cursor<&Vec<u8>>>, name: &str, expected: &[u8]) -> bool {
    let mut entry = match archive.by_name(name) {
        Ok(e) => e,
        Err(_) => return false,
    };
    if entry.size() as usize != expected.len() {
        return false;
    }
    let mut buf = Vec::with_capacity(expected.len());
    if entry.read_to_end(&mut buf).is_err() {
        return false;
    }
    buf == expected
}
