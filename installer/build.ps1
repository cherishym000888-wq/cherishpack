#Requires -Version 5.1
<#
.SYNOPSIS
  CherishWorld 이중 빌드 — 배포판 + 오프라인 테스트판 동시 생성.

.DESCRIPTION
  Cargo.toml 의 feature flag 로 두 가지 빌드를 분리.

  - 배포판   (CherishWorld.exe)        : MSA 로그인 전용. GitHub 릴리스 업로드용.
  - 테스트판 (CherishWorld-test.exe)   : --features offline 로 빌드.
                                         GUI 에 닉네임 입력칸 노출, CLI --offline <nick> 지원.
                                         배포 금지 — 내부 테스트 전용.

  의존성 컴파일은 두 빌드가 캐시를 공유하므로 두 번째는 빠르게 끝남.
  전체 소요 1~2분 예상.

.EXAMPLE
  .\build.ps1            # 둘 다 빌드
  .\build.ps1 -Clean     # target 폴더 비우고 새로 빌드
#>
[CmdletBinding()]
param(
    [switch]$Clean
)

# cargo 는 진행 메시지를 stderr 에 쏟아내므로 ErrorActionPreference=Stop 면 가짜 오류가 뜬다.
# 네이티브 프로세스는 $LASTEXITCODE 로만 판정한다.
$ErrorActionPreference = 'Continue'
# 콘솔 UTF-8 출력 — 한글 깨짐 방지
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$OutputEncoding = [System.Text.Encoding]::UTF8
Set-Location $PSScriptRoot

$target = Join-Path $PSScriptRoot 'target\release'
$dist   = Join-Path $PSScriptRoot 'dist'

if ($Clean) {
    Write-Host "[*] cargo clean ..." -ForegroundColor Cyan
    cargo clean
}
New-Item -ItemType Directory -Force -Path $dist | Out-Null

# ─────────────────────── 1) 배포판 ───────────────────────
Write-Host ''
Write-Host "[1/2] 배포판 빌드 (MSA 전용) ..." -ForegroundColor Cyan
cargo build --release
if ($LASTEXITCODE -ne 0) { throw "배포판 빌드 실패" }

Copy-Item (Join-Path $target 'CherishWorld.exe') (Join-Path $dist 'CherishWorld.exe') -Force
$sizeA = (Get-Item (Join-Path $dist 'CherishWorld.exe')).Length
Write-Host ("    → dist\CherishWorld.exe  ({0:N0} bytes / {1:N2} MB)" -f $sizeA, ($sizeA/1MB)) -ForegroundColor Green

# ─────────────────────── 2) 테스트판 ─────────────────────
Write-Host ''
Write-Host "[2/2] 테스트판 빌드 (offline 포함) ..." -ForegroundColor Cyan
cargo build --release --features offline
if ($LASTEXITCODE -ne 0) { throw "테스트판 빌드 실패" }

Copy-Item (Join-Path $target 'CherishWorld.exe') (Join-Path $dist 'CherishWorld-test.exe') -Force
$sizeB = (Get-Item (Join-Path $dist 'CherishWorld-test.exe')).Length
Write-Host ("    → dist\CherishWorld-test.exe  ({0:N0} bytes / {1:N2} MB)" -f $sizeB, ($sizeB/1MB)) -ForegroundColor Green

# ─────────────────────── 요약 ───────────────────────────
Write-Host ''
Write-Host '=== 빌드 완료 ===' -ForegroundColor Green
Write-Host ("  배포판:   {0}" -f (Join-Path $dist 'CherishWorld.exe'))
Write-Host ("  테스트판: {0}" -f (Join-Path $dist 'CherishWorld-test.exe'))
Write-Host ''
Write-Host '배포판만 GitHub 릴리스에 업로드하세요. 테스트판은 내부 보관.' -ForegroundColor Yellow
