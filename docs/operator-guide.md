# Operator Guide (운영자용)

## 패치 배포 절차 (Phase 2 이후 확정 예정)

1. `pack-source/` 에서 모드 교체·설정 변경
2. `cherishpack-manifest-builder` 실행 → `dist/<ver>.mrpack` + `dist/manifests/<ver>.json` 생성
3. `dist/version.json` 업데이트 (stable/beta 버전·`min_required` 지정)
4. GitHub Release 생성:
   ```
   gh release create v<ver> dist/<ver>.mrpack dist/manifests/<ver>.json
   ```
5. `version.json` 은 main 브랜치에 커밋 (raw.githubusercontent.com 에서 서빙)

## 강제 업데이트

`version.json` 의 `min_required` 를 올리면 그 미만 버전 설치본은 실행 차단된다.
신중하게 사용할 것 — 사용자는 업데이트 외에 선택지가 없다.

## 크래시 리포트 엔드포인트

- URL: `POST /api/crash-report` (138.2.127.45, Cloudflare 앞단 예정)
- 크기 제한: 1MB, Rate limit: IP당 10건/시간
- 보관: 7일 자동 삭제
- 사용자명 마스킹은 **클라이언트에서** 수행 후 업로드
