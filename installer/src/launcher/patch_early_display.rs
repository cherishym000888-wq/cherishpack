//! NeoForge earlydisplay jar 자동 패치.
//!
//! NeoForge 의 빨간 부팅화면을 파스텔 핑크로 바꾼다. 라이브러리 재다운로드 시
//! sha1 검증으로 패치가 롤백되므로, 매 실행 전(run_inner / launch-only) 에
//! 호출해 자동 복원.

use anyhow::{Context, Result};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const TARGET_CLASS: &str = "net/neoforged/fml/earlydisplay/ColourScheme.class";

/// RED scheme 원본: BG (239,50,61) FG (255,255,255)
const OLD_BG: [u8; 7] = [0x11, 0x00, 0xEF, 0x10, 0x32, 0x10, 0x3D];
const OLD_FG: [u8; 9] = [0x11, 0x00, 0xFF, 0x11, 0x00, 0xFF, 0x11, 0x00, 0xFF];
/// 새 scheme: BG 파스텔 핑크(255,214,232) FG 딥 베리(128,40,70)
const NEW_BG: [u8; 9] = [0x11, 0x00, 0xFF, 0x11, 0x00, 0xD6, 0x11, 0x00, 0xE8];
const NEW_FG: [u8; 7] = [0x11, 0x00, 0x80, 0x10, 0x28, 0x10, 0x46];

/// `<libraries>/net/neoforged/fancymodloader/earlydisplay/<version>/earlydisplay-<version>.jar`
/// 를 찾아 ColourScheme.class 의 색을 핑크로 패치. 이미 패치돼 있으면 스킵.
pub fn apply_if_needed(libraries_dir: &Path) -> Result<()> {
    let jar = match find_jar(libraries_dir) {
        Some(p) => p,
        None => return Ok(()), // earlydisplay 없으면 스킵 (서버·offline 모드)
    };

    let bytes = fs::read(&jar).with_context(|| format!("jar 읽기 실패: {}", jar.display()))?;
    let cursor = std::io::Cursor::new(&bytes);
    let mut archive = zip::ZipArchive::new(cursor)?;

    // ColourScheme.class 추출 후 패턴 확인
    let mut class_bytes = Vec::new();
    {
        let mut entry = archive
            .by_name(TARGET_CLASS)
            .with_context(|| format!("{TARGET_CLASS} 엔트리 없음"))?;
        entry.read_to_end(&mut class_bytes)?;
    }

    // 이미 패치된 상태면 no-op
    if find_bytes(&class_bytes, &NEW_BG).is_some() {
        return Ok(());
    }
    // 원본 패턴 없으면 이미 수정됐거나 버전 불일치 — 건드리지 않음
    let bg_idx = match find_bytes(&class_bytes, &OLD_BG) {
        Some(i) => i,
        None => return Ok(()),
    };
    let fg_idx = match find_bytes(&class_bytes[bg_idx + OLD_BG.len()..], &OLD_FG) {
        Some(i) => bg_idx + OLD_BG.len() + i,
        None => return Ok(()),
    };

    // FG 먼저 (뒤쪽) 패치 — BG 패치로 오프셋 쉬프트 영향 없게.
    let mut patched = class_bytes.clone();
    patched.splice(fg_idx..fg_idx + OLD_FG.len(), NEW_FG.iter().copied());
    patched.splice(bg_idx..bg_idx + OLD_BG.len(), NEW_BG.iter().copied());

    // jar 의 해당 엔트리만 교체 (전체 재압축, in-place).
    let tmp = jar.with_extension("jar.patch.tmp");
    write_jar_replacing(&jar, &tmp, TARGET_CLASS, &patched)?;
    fs::rename(&tmp, &jar)?;
    tracing::info!("earlydisplay jar 핑크 패치 적용");
    Ok(())
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

fn write_jar_replacing(src: &Path, dst: &Path, target_name: &str, new_bytes: &[u8]) -> Result<()> {
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
        if name == target_name {
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
