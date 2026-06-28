$ErrorActionPreference = "Stop"

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$fixtureRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("opengpu-install-ps1-test-" + [System.Guid]::NewGuid().ToString("N"))
$releaseDir = Join-Path $fixtureRoot "release"
$installDir = Join-Path $fixtureRoot "bin"
$assetName = "opengpu-x86_64-pc-windows-msvc.exe"
$assetPath = Join-Path $releaseDir $assetName
$checksumPath = "$assetPath.sha256"
$manifestPath = Join-Path $releaseDir "release-manifest.json"
$signaturePath = Join-Path $releaseDir "release-manifest.json.sig"

try {
  New-Item -ItemType Directory -Force -Path $releaseDir | Out-Null
  New-Item -ItemType Directory -Force -Path $installDir | Out-Null

  Set-Content -Path $assetPath -Value "fake opengpu windows binary" -NoNewline -Encoding ASCII
  $checksum = (Get-FileHash -Algorithm SHA256 -Path $assetPath).Hash.ToLowerInvariant()
  Set-Content -Path $checksumPath -Value "$checksum  $assetName`n" -NoNewline -Encoding ASCII
  @{
    artifact_kind = "release-binary"
    binary_name = $assetName
    checksum_sha256 = $checksum
    generated_at = "2026-06-28T00:00:00Z"
    tag = "local-preview"
    version = "0.1.0"
  } | ConvertTo-Json | Set-Content -Path $manifestPath -Encoding ASCII
  Set-Content -Path $signaturePath -Value "local preview signature fixture" -NoNewline -Encoding ASCII

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
  if ($joinedOutput -notmatch "Checking signed release manifest") {
    throw "installer did not check signed release manifest"
  }
  if ($joinedOutput -notmatch "strict enterprise") {
    throw "installer did not report strict enterprise verification mode"
  }

  Remove-Item -LiteralPath $checksumPath -Force
  Remove-Item -LiteralPath $installedExe -Force

  $previousErrorActionPreference = $ErrorActionPreference
  $ErrorActionPreference = "Continue"
  $missingChecksumOutput = & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repoRoot "install.ps1") `
    -InstallDir $installDir `
    -ReleaseBaseUrl $releaseDir 2>&1
  $missingChecksumExitCode = $LASTEXITCODE
  $ErrorActionPreference = $previousErrorActionPreference

  if ($missingChecksumExitCode -eq 0) {
    throw "install.ps1 succeeded without checksum in strict mode"
  }

  if (($missingChecksumOutput -join "`n") -notmatch "strict installer verification failed: checksum unavailable or invalid") {
    throw "missing checksum failure did not explain strict verification"
  }

  $previewOutput = & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repoRoot "install.ps1") `
    -InstallDir $installDir `
    -ReleaseBaseUrl $releaseDir `
    -AllowUnsignedLocalPreview 2>&1

  if ($LASTEXITCODE -ne 0) {
    throw "install.ps1 failed with local preview override: $previewOutput"
  }

  if (-not (Test-Path -LiteralPath $installedExe)) {
    throw "expected installer to write opengpu.exe with local preview override"
  }

  $joinedPreviewOutput = $previewOutput -join "`n"
  if ($joinedPreviewOutput -notmatch "local preview override") {
    throw "installer did not report local preview override mode"
  }
  if ($joinedPreviewOutput -notmatch "dev-only local preview override: checksum unavailable") {
    throw "missing checksum override warning was not printed"
  }

  Remove-Item -LiteralPath $installedExe -Force
  Set-Content -Path $checksumPath -Value "$checksum  $assetName`n" -NoNewline -Encoding ASCII
  Remove-Item -LiteralPath $manifestPath -Force

  $previousErrorActionPreference = $ErrorActionPreference
  $ErrorActionPreference = "Continue"
  $missingManifestOutput = & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repoRoot "install.ps1") `
    -InstallDir $installDir `
    -ReleaseBaseUrl $releaseDir 2>&1
  $missingManifestExitCode = $LASTEXITCODE
  $ErrorActionPreference = $previousErrorActionPreference

  if ($missingManifestExitCode -eq 0) {
    throw "install.ps1 succeeded without signed manifest in strict mode"
  }

  if (($missingManifestOutput -join "`n") -notmatch "strict installer verification failed: signed release manifest unavailable or invalid") {
    throw "missing signed manifest failure did not explain strict verification"
  }

  Write-Output "PASS: install.ps1 enforces strict Windows release verification with local preview override"
} finally {
  Remove-Item -Recurse -Force -LiteralPath $fixtureRoot -ErrorAction SilentlyContinue
}
