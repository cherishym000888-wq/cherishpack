# pack-source/

CherishPack 모드팩 원본. NeoForge 1.21.1.

## 구조

- `modrinth.index.json` — 모드 목록·loader·minecraft 버전 (Modrinth `.mrpack` 표준 스키마)
- `overrides/` — 모드팩에 포함할 설정/리소스 (config, resourcepacks, shaderpacks 등)

## 빌드

`manifest-builder` CLI가 이 폴더를 입력받아 `.mrpack` + `dist/manifests/<ver>.json` 을 생성한다.
(Phase 2에서 구현)
