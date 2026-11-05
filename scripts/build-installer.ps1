# Собирает установщик из уже собранного target\release\bolh-place.exe.
# Версия берётся из Cargo.toml — она же номер выпуска на GitHub.
# Результат: dist\BolhPlace-<версия>-setup.exe
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
if (-not $version) { throw "Не нашёл версию в Cargo.toml" }
if (-not (Test-Path target\release\bolh-place.exe)) { throw "Сначала: cargo build --release -p bolh-place" }

$iscc = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"
if (-not (Test-Path $iscc)) {
    # На раннере GitHub Inno Setup может не оказаться — ставим.
    choco install innosetup -y --no-progress | Out-Null
}
if (-not (Test-Path $iscc)) { throw "Inno Setup не установлен" }

New-Item -ItemType Directory -Force dist | Out-Null
& $iscc /Qp "/DAppVersion=$version" packaging\bolh-place.iss
if ($LASTEXITCODE -ne 0) { throw "ISCC завершился с кодом $LASTEXITCODE" }

$out = "dist\BolhPlace-$version-setup.exe"
if (-not (Test-Path $out)) { throw "Нет файла $out" }
if ($env:GITHUB_OUTPUT) {
    "version=$version" | Out-File -Append -Encoding utf8 $env:GITHUB_OUTPUT
    "installer=$out" | Out-File -Append -Encoding utf8 $env:GITHUB_OUTPUT
}
Write-Host "Установщик: $out"
