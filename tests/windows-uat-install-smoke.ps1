$ErrorActionPreference = "Stop"

if (-not $IsWindows -and $PSVersionTable.PSEdition -eq "Core") {
  throw "windows UAT install smoke must run on Windows"
}

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$fixtureRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("opengpu-windows-uat-smoke-" + [System.Guid]::NewGuid().ToString("N"))
$releaseDir = Join-Path $fixtureRoot "release"
$installDir = Join-Path $fixtureRoot "bin"
$opengpuHome = Join-Path $fixtureRoot "home"
$assetName = "opengpu-x86_64-pc-windows-msvc.exe"
$assetPath = Join-Path $releaseDir $assetName
$agentAssetName = "opengpu-node-agent-x86_64-pc-windows-msvc.exe"
$agentAssetPath = Join-Path $releaseDir $agentAssetName
$runtimeAssetName = "llama-runtime-x86_64-pc-windows-msvc-cuda.zip"
$runtimeAssetPath = Join-Path $releaseDir $runtimeAssetName
$runtimeFixtureDir = Join-Path $fixtureRoot "runtime-fixture"
$agentBuildPath = Join-Path $repoRoot "target\release\opengpu-node-agent.exe"

function Invoke-Checked {
  param(
    [string]$Step,
    [scriptblock]$Command
  )

  $previousErrorActionPreference = $ErrorActionPreference
  $ErrorActionPreference = "Continue"
  try {
    $output = & $Command 2>&1
    $exitCode = $LASTEXITCODE
  } finally {
    $ErrorActionPreference = $previousErrorActionPreference
  }
  if ($exitCode -ne 0) {
    throw "$Step failed with exit code $exitCode`n$($output -join "`n")"
  }
  return $output
}

try {
  New-Item -ItemType Directory -Force -Path $releaseDir | Out-Null
  New-Item -ItemType Directory -Force -Path $installDir | Out-Null
  New-Item -ItemType Directory -Force -Path $opengpuHome | Out-Null

  Invoke-Checked "cargo build opengpu release binary" {
    cargo build --manifest-path (Join-Path $repoRoot "apps\cli\Cargo.toml") --release
  } | Out-Null
  Invoke-Checked "cargo build node-agent release binary" {
    cargo build --manifest-path (Join-Path $repoRoot "agents\node\Cargo.toml") --release
  } | Out-Null

  Copy-Item -LiteralPath (Join-Path $repoRoot "target\release\opengpu.exe") -Destination $assetPath
  Copy-Item -LiteralPath $agentBuildPath -Destination $agentAssetPath
  New-Item -ItemType Directory -Force -Path $runtimeFixtureDir | Out-Null
  Copy-Item -LiteralPath $agentBuildPath -Destination (Join-Path $runtimeFixtureDir "llama-cli.exe")
  Set-Content -Path (Join-Path $runtimeFixtureDir "cudart64_11.dll") -Value "uat cuda runtime fixture" -NoNewline -Encoding ASCII
  Compress-Archive -Path (Join-Path $runtimeFixtureDir "*") -DestinationPath $runtimeAssetPath -Force

  $checksum = (Get-FileHash -Algorithm SHA256 -Path $assetPath).Hash.ToLowerInvariant()
  Set-Content -Path "$assetPath.sha256" -Value "$checksum  $assetName`n" -NoNewline -Encoding ASCII
  $agentChecksum = (Get-FileHash -Algorithm SHA256 -Path $agentAssetPath).Hash.ToLowerInvariant()
  Set-Content -Path "$agentAssetPath.sha256" -Value "$agentChecksum  $agentAssetName`n" -NoNewline -Encoding ASCII
  $runtimeChecksum = (Get-FileHash -Algorithm SHA256 -Path $runtimeAssetPath).Hash.ToLowerInvariant()
  Set-Content -Path "$runtimeAssetPath.sha256" -Value "$runtimeChecksum  $runtimeAssetName`n" -NoNewline -Encoding ASCII
  $runtimeExeChecksum = (Get-FileHash -Algorithm SHA256 -Path (Join-Path $runtimeFixtureDir "llama-cli.exe")).Hash.ToLowerInvariant()
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
    generated_at = "2026-06-29T00:00:00Z"
    tag = "uat-local-preview"
    version = "0.1.0"
  } | ConvertTo-Json | Set-Content -Path (Join-Path $releaseDir "release-manifest.json") -Encoding ASCII
  Set-Content -Path (Join-Path $releaseDir "release-manifest.json.sig") -Value "uat local preview signature fixture" -NoNewline -Encoding ASCII

  $previousHome = $env:OPENGPU_HOME
  $env:OPENGPU_HOME = $opengpuHome

  Invoke-Checked "install.ps1 local release install" {
    powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repoRoot "install.ps1") `
      -InstallDir $installDir `
      -ReleaseBaseUrl $releaseDir `
      -InstallCudaRuntime
  } | Out-Null

  $installedExe = Join-Path $installDir "opengpu.exe"
  $agentInstallPath = Join-Path $installDir "opengpu-node-agent.exe"
  $installedRuntime = Join-Path $opengpuHome "runtimes\llama\llama-cli.exe"
  $installedRuntimeDll = Join-Path $opengpuHome "runtimes\llama\cudart64_11.dll"
  if (-not (Test-Path -LiteralPath $installedExe)) {
    throw "install.ps1 did not install opengpu.exe"
  }
  if (-not (Test-Path -LiteralPath $agentInstallPath)) {
    throw "install.ps1 did not install opengpu-node-agent.exe"
  }
  if (-not (Test-Path -LiteralPath $installedRuntime)) {
    throw "install.ps1 did not install llama-cli.exe"
  }
  if (-not (Test-Path -LiteralPath $installedRuntimeDll)) {
    throw "install.ps1 did not extract CUDA runtime DLLs"
  }

  $trustedPath = Join-Path $opengpuHome "trusted-runtime-paths.json"
  if (-not (Test-Path -LiteralPath $trustedPath)) {
    throw "install.ps1 did not write trusted-runtime-paths.json"
  }
  $trusted = Get-Content -Path $trustedPath -Raw | ConvertFrom-Json
  if ($trusted.llama_cli.path -ne ([System.IO.Path]::GetFullPath($installedRuntime))) {
    throw "trusted runtime path does not point at installed llama-cli.exe"
  }
  if ($trusted.llama_cli.sha256 -ne $runtimeExeChecksum) {
    throw "trusted runtime checksum does not match extracted llama-cli.exe"
  }

  $modelDir = Join-Path $opengpuHome "models"
  New-Item -ItemType Directory -Force -Path $modelDir | Out-Null
  Set-Content -Path (Join-Path $modelDir "uat-local-fixture.gguf") -Value "local model fixture" -NoNewline -Encoding ASCII
  @{
    version = 1
    device_id = "node-uat-windows-smoke"
    public_key_fingerprint = $null
    profile_name = $null
    auth_token = $null
    connected = $false
    paused = $false
    backend_preference = "auto"
    contribution_percent = 0
    control_plane_url = "https://uat.mundusx.ai"
    active_model = "uat-local-fixture"
    models = @("uat-local-fixture")
    model_dir = $modelDir
    onboarding_completed = $false
  } | ConvertTo-Json | Set-Content -Path (Join-Path $opengpuHome "config.json") -Encoding ASCII

  Invoke-Checked "opengpu install UAT control-plane config" {
    & $installedExe install --private --control-plane-url http://127.0.0.1:8787 --cap-percent 30
  } | Out-Null
  Invoke-Checked "opengpu login protected token storage" {
    & $installedExe login --token "uat-smoke-token"
  } | Out-Null

  $configPath = Join-Path $opengpuHome "config.json"
  $configRaw = Get-Content -Path $configPath -Raw
  $config = $configRaw | ConvertFrom-Json
  if ($config.control_plane_url -ne "http://127.0.0.1:8787") {
    throw "CLI config did not point at the UAT/local control-plane URL"
  }
  if ($config.active_model -ne "uat-local-fixture") {
    throw "CLI install did not preserve the local UAT model fixture"
  }
  if ($configRaw -match "uat-smoke-token") {
    throw "config.json contains the plaintext operator token"
  }
  if (-not (Test-Path -LiteralPath (Join-Path $opengpuHome "operator-token.dpapi"))) {
    throw "protected Windows operator token blob was not created"
  }

  $doctor = (Invoke-Checked "opengpu doctor json" {
    & $installedExe doctor --json
  } | Out-String) | ConvertFrom-Json
  if ($doctor.auth_token_present -ne $true) {
    throw "doctor did not report the protected operator token as present"
  }
  if ($doctor.config_dir -ne $opengpuHome) {
    throw "doctor did not use the isolated OPENGPU_HOME"
  }

  $agentHealth = (Invoke-Checked "opengpu-node-agent health json" {
    & $agentInstallPath health --json
  } | Out-String) | ConvertFrom-Json
  if (-not $agentHealth.health) {
    throw "node-agent health did not return a health payload"
  }
  if (-not $agentHealth.policy) {
    throw "node-agent health did not return an actionable policy payload"
  }

  Write-Output "PASS: Windows UAT install smoke verified installer, CLI config, protected token storage, doctor, and node-agent health"
} finally {
  if (Get-Variable -Name previousHome -Scope Local -ErrorAction SilentlyContinue) {
    $env:OPENGPU_HOME = $previousHome
  }
  Remove-Item -Recurse -Force -LiteralPath $fixtureRoot -ErrorAction SilentlyContinue
}
