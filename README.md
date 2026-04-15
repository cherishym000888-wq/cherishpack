# CherishPack

마인크래프트 NeoForge 1.21.1 모드서버 클라이언트팩 설치 프로그램.

실행파일 하나로 **Prism Launcher + 모드팩 + 설정파일** 설치/패치/실행을 담당한다.

## 구성

- `installer/` — Rust 단일 exe 부트스트래퍼 (iced GUI)
- `manifest-builder/` — 모드팩 원본 → 매니페스트 JSON 자동 생성 CLI
- `pack-source/` — 모드팩 원본 (`modrinth.index.json` + overrides)
- `dist/` — 릴리스 산출물 (Git 무시, GitHub Release 업로드 전용)
- `docs/` — 사용자/운영자 가이드

## 핵심 정책

- **매니페스트 기반 파일 관리** — 설치한 파일만 기록·비교·삭제. 사용자가 직접 둔 파일은 절대 건드리지 않는다.
- **휴지통 이동** — 삭제는 영구 삭제 아님. 롤백 여지 보존.
- **보호 목록** — `options.txt`, `servers.dat`, `saves/**`, `screenshots/**`, `logs/**`, `crash-reports/**` 는 영구 보존.
- **강제 업데이트** — `min_required` 버전 미만이면 실행 차단.
- **사양 기반 프리셋 추천** — 사용자 최종 선택권은 유지.

## 빌드

```bash
cd installer
cargo build --release
```

산출물: `installer/target/release/cherishpack-installer.exe`

## 배포

GitHub Release (public 레포 `cherishpack`) 기반 하이브리드 배포.
상세는 `docs/operator-guide.md`.

## 라이선스

MIT. Prism Launcher(GPL-3.0)는 **재배포하지 않고** 설치 시 공식 릴리스 URL에서 다운로드한다.
