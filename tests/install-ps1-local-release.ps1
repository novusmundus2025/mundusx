$ErrorActionPreference = "Stop"

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$fixtureRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("opengpu-install-ps1-test-" + [System.Guid]::NewGuid().ToString("N"))
$releaseDir = Join-Path $fixtureRoot "release"
$installDir = Join-Path $fixtureRoot "bin"
$assetName = "opengpu-x86_64-pc-windows-msvc.exe"
$assetPath = Join-Path $releaseDir $assetName
$checksumPath = "$assetPath.sha256"
$agentAssetName = "opengpu-node-agent-x86_64-pc-windows-msvc.exe"
$agentAssetPath = Join-Path $releaseDir $agentAssetName
$agentChecksumPath = "$agentAssetPath.sha256"
$runtimeAssetName = "llama-runtime-x86_64-pc-windows-msvc-cuda.zip"
$runtimeAssetPath = Join-Path $releaseDir $runtimeAssetName
$runtimeChecksumPath = "$runtimeAssetPath.sha256"
$runtimeFixtureDir = Join-Path $fixtureRoot "runtime-fixture"
$opengpuHome = Join-Path $fixtureRoot "home"
$manifestPath = Join-Path $releaseDir "release-manifest.json"
$signaturePath = Join-Path $releaseDir "release-manifest.json.sig"

try {
  New-Item -ItemType Directory -Force -Path $releaseDir | Out-Null
  New-Item -ItemType Directory -Force -Path $installDir | Out-Null
  New-Item -ItemType Directory -Force -Path $opengpuHome | Out-Null

  Set-Content -Path $assetPath -Value "fake opengpu windows binary" -NoNewline -Encoding ASCII
  $checksum = (Get-FileHash -Algorithm SHA256 -Path $assetPath).Hash.ToLowerInvariant()
  Set-Content -Path $checksumPath -Value "$checksum  $assetName`n" -NoNewline -Encoding ASCII
  Set-Content -Path $agentAssetPath -Value "fake node agent windows binary" -NoNewline -Encoding ASCII
  $agentChecksum = (Get-FileHash -Algorithm SHA256 -Path $agentAssetPath).Hash.ToLowerInvariant()
  Set-Content -Path $agentChecksumPath -Value "$agentChecksum  $agentAssetName`n" -NoNewline -Encoding ASCII
  New-Item -ItemType Directory -Force -Path $runtimeFixtureDir | Out-Null
  Set-Content -Path (Join-Path $runtimeFixtureDir "llama-cli.exe") -Value "fake cuda llama runtime" -NoNewline -Encoding ASCII
  Set-Content -Path (Join-Path $runtimeFixtureDir "cudart64_11.dll") -Value "fake cuda runtime dll" -NoNewline -Encoding ASCII
  Compress-Archive -Path (Join-Path $runtimeFixtureDir "*") -DestinationPath $runtimeAssetPath -Force
  $runtimeExeChecksum = (Get-FileHash -Algorithm SHA256 -Path (Join-Path $runtimeFixtureDir "llama-cli.exe")).Hash.ToLowerInvariant()
  $runtimeChecksum = (Get-FileHash -Algorithm SHA256 -Path $runtimeAssetPath).Hash.ToLowerInvariant()
  Set-Content -Path $runtimeChecksumPath -Value "$runtimeChecksum  $runtimeAssetName`n" -NoNewline -Encoding ASCII
  @{
    artifact_kind = "release-binary"
    binary_name = $assetName
    checksum_sha256 = $checksum
    assets = @(
      @{
        name = $agentAssetName
        install_as = "opengpu-node-agent.exe"
        kind = "node-agent-binary"
        checksum_sha256 = $agentChecksum
      }
    )
    runtime_assets = @(
      @{
        name = $runtimeAssetName
        install_as = "runtimes/llama"
        kind = "llama-cpp-cuda-runtime-bundle"
        checksum_sha256 = $runtimeChecksum
      }
    )
    generated_at = "2026-06-28T00:00:00Z"
    tag = "local-preview"
    version = "0.1.0"
  } | ConvertTo-Json | Set-Content -Path $manifestPath -Encoding ASCII
  Set-Content -Path $signaturePath -Value "local preview signature fixture" -NoNewline -Encoding ASCII

  $previousHome = $env:OPENGPU_HOME
  $env:OPENGPU_HOME = $opengpuHome

  $output = & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repoRoot "install.ps1") `
    -InstallDir $installDir `
    -ReleaseBaseUrl $releaseDir `
    -InstallCudaRuntime 2>&1

  if ($LASTEXITCODE -ne 0) {
    throw "install.ps1 failed with checksum present: $output"
  }

  $installedExe = Join-Path $installDir "opengpu.exe"
  $installedAgent = Join-Path $installDir "opengpu-node-agent.exe"
  $installedRuntime = Join-Path $opengpuHome "runtimes\llama\llama-cli.exe"
  $installedRuntimeDll = Join-Path $opengpuHome "runtimes\llama\cudart64_11.dll"
  if (-not (Test-Path -LiteralPath $installedExe)) {
    throw "expected installer to write opengpu.exe"
  }
  if (-not (Test-Path -LiteralPath $installedAgent)) {
    throw "expected installer to write opengpu-node-agent.exe"
  }
  if (-not (Test-Path -LiteralPath $installedRuntime)) {
    throw "expected installer to write llama-cli.exe"
  }
  if (-not (Test-Path -LiteralPath $installedRuntimeDll)) {
    throw "expected installer to extract CUDA runtime DLLs"
  }

  if ((Get-Content -Path $installedExe -Raw) -ne "fake opengpu windows binary") {
    throw "installed executable contents did not match release asset"
  }
  if ((Get-Content -Path $installedRuntime -Raw) -ne "fake cuda llama runtime") {
    throw "installed runtime contents did not match release asset"
  }
  if ((Get-Content -Path $installedAgent -Raw) -ne "fake node agent windows binary") {
    throw "installed node agent contents did not match release asset"
  }

  $trustedPath = Join-Path $opengpuHome "trusted-runtime-paths.json"
  if (-not (Test-Path -LiteralPath $trustedPath)) {
    throw "installer did not write trusted-runtime-paths.json"
  }
  $trusted = Get-Content -Path $trustedPath -Raw | ConvertFrom-Json
  if ($trusted.llama_cli.path -ne ([System.IO.Path]::GetFullPath($installedRuntime))) {
    throw "trusted runtime path did not point at installed llama-cli.exe"
  }
  if ($trusted.llama_cli.sha256 -ne $runtimeExeChecksum) {
    throw "trusted runtime checksum did not match extracted llama-cli.exe"
  }

  $joinedOutput = $output -join "`n"
  if ($joinedOutput -notmatch [regex]::Escape($assetName)) {
    throw "installer output did not mention expected Windows asset name"
  }
  if ($joinedOutput -notmatch [regex]::Escape($runtimeAssetName)) {
    throw "installer output did not mention expected CUDA runtime asset name"
  }
  if ($joinedOutput -notmatch [regex]::Escape($agentAssetName)) {
    throw "installer output did not mention expected node agent asset name"
  }
  if ($joinedOutput -notmatch "Verifying checksum") {
    throw "installer did not verify checksum"
  }
  if ($joinedOutput -notmatch "Verifying CUDA runtime checksum") {
    throw "installer did not verify CUDA runtime checksum"
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
    -ReleaseBaseUrl $releaseDir `
    -InstallCudaRuntime 2>&1
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
    -InstallCudaRuntime `
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
    -ReleaseBaseUrl $releaseDir `
    -InstallCudaRuntime 2>&1
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
  if (Get-Variable -Name previousHome -Scope Local -ErrorAction SilentlyContinue) {
    $env:OPENGPU_HOME = $previousHome
  }
  Remove-Item -Recurse -Force -LiteralPath $fixtureRoot -ErrorAction SilentlyContinue
}
