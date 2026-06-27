$ErrorActionPreference = "Stop"

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$fixtureRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("opengpu-install-ps1-test-" + [System.Guid]::NewGuid().ToString("N"))
$releaseDir = Join-Path $fixtureRoot "release"
$installDir = Join-Path $fixtureRoot "bin"
$assetName = "opengpu-x86_64-pc-windows-msvc.exe"
$assetPath = Join-Path $releaseDir $assetName
$checksumPath = "$assetPath.sha256"

try {
  New-Item -ItemType Directory -Force -Path $releaseDir | Out-Null
  New-Item -ItemType Directory -Force -Path $installDir | Out-Null

  Set-Content -Path $assetPath -Value "fake opengpu windows binary" -NoNewline -Encoding ASCII
  $checksum = (Get-FileHash -Algorithm SHA256 -Path $assetPath).Hash.ToLowerInvariant()
  Set-Content -Path $checksumPath -Value "$checksum  $assetName`n" -NoNewline -Encoding ASCII

  $output = & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repoRoot "install.ps1") `
    -InstallDir $installDir `
    -ReleaseBaseUrl $releaseDir 2>&1

  if ($LASTEXITCODE -ne 0) {
    throw "install.ps1 failed with checksum present: $output"
  }

  $installedExe = Join-Path $installDir "opengpu.exe"
  if (-not (Test-Path -LiteralPath $installedExe)) {
    throw "expected installer to write opengpu.exe"
  }

  if ((Get-Content -Path $installedExe -Raw) -ne "fake opengpu windows binary") {
    throw "installed executable contents did not match release asset"
  }

  $joinedOutput = $output -join "`n"
  if ($joinedOutput -notmatch [regex]::Escape($assetName)) {
    throw "installer output did not mention expected Windows asset name"
  }
  if ($joinedOutput -notmatch "Verifying checksum") {
    throw "installer did not verify checksum"
  }

  Remove-Item -LiteralPath $checksumPath -Force
  Remove-Item -LiteralPath $installedExe -Force

  $missingChecksumOutput = & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repoRoot "install.ps1") `
    -InstallDir $installDir `
    -ReleaseBaseUrl $releaseDir 2>&1

  if ($LASTEXITCODE -ne 0) {
    throw "install.ps1 failed when checksum was missing: $missingChecksumOutput"
  }

  if (-not (Test-Path -LiteralPath $installedExe)) {
    throw "expected installer to write opengpu.exe without checksum"
  }

  if (($missingChecksumOutput -join "`n") -notmatch "checksum unavailable") {
    throw "missing checksum warning was not printed"
  }

  Write-Output "PASS: install.ps1 handles local Windows release preview assets"
} finally {
  Remove-Item -Recurse -Force -LiteralPath $fixtureRoot -ErrorAction SilentlyContinue
}
