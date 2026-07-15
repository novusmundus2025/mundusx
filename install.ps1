param(
  [string]$InstallDir = "$env:USERPROFILE\.opengpu\bin",
  [string]$ReleaseBaseUrl = "https://github.com/mundusx/mundusx/releases/latest/download",
  [string]$GitHubToken = "",
  [switch]$AllowUnsignedLocalPreview,
  [switch]$InstallCudaRuntime,
  [switch]$SkipTrayAutoStart,
  [switch]$Help
)

$ErrorActionPreference = "Stop"
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

function Show-Usage {
  @"
Usage:
  powershell -ExecutionPolicy Bypass -File .\install.ps1

Options:
  -InstallDir <path>       Directory where opengpu.exe will be installed.
  -ReleaseBaseUrl <url>    Release download base URL.
  -GitHubToken <token>     Optional token for private GitHub release assets.
  -InstallCudaRuntime      Force CUDA llama runtime installation and pinning.
  -SkipTrayAutoStart       Install the tray companion without starting it at sign-in.
  -AllowUnsignedLocalPreview
                          Dev-only: allow missing checksum or signed manifest
                          when testing a local release preview.
  -Help                    Print this help and exit.

After this bootstrapper installs the binary, run:
  opengpu install
"@ | Write-Output
}

function Get-GitHubToken {
  if ($GitHubToken) {
    return $GitHubToken
  }
  if ($env:GITHUB_TOKEN) {
    return $env:GITHUB_TOKEN
  }
  if ($env:GH_TOKEN) {
    return $env:GH_TOKEN
  }
  return $null
}

function Assert-DownloadedReleaseFile {
  param(
    [string]$Source,
    [string]$Destination
  )

  if (-not (Test-Path -LiteralPath $Destination)) {
    throw "download failed for $Source"
  }

  $item = Get-Item -LiteralPath $Destination
  if ($item.Length -le 0) {
    throw "downloaded empty file from $Source"
  }

  $bufferSize = [Math]::Min(512, [int]$item.Length)
  $buffer = New-Object byte[] $bufferSize
  $stream = [System.IO.File]::OpenRead($Destination)
  try {
    [void]$stream.Read($buffer, 0, $bufferSize)
  } finally {
    $stream.Dispose()
  }
  $prefix = [System.Text.Encoding]::UTF8.GetString($buffer).TrimStart()
  if ($prefix.StartsWith("<!DOCTYPE", [System.StringComparison]::OrdinalIgnoreCase) -or
      $prefix.StartsWith("<html", [System.StringComparison]::OrdinalIgnoreCase)) {
    if ($Source -like "https://github.com/*") {
      throw "GitHub returned an HTML page instead of the release asset. If this repository or release is private, pass -GitHubToken or set GITHUB_TOKEN/GH_TOKEN with release read access."
    }
    throw "downloaded HTML instead of the expected release asset from $Source"
  }
}

function Resolve-GitHubReleaseAssetApiUrl {
  param([string]$Source)

  $uri = $null
  if (-not [System.Uri]::TryCreate($Source, [System.UriKind]::Absolute, [ref]$uri)) {
    return $null
  }
  if ($uri.Host -ne "github.com") {
    return $null
  }

  $parts = $uri.AbsolutePath.Trim("/") -split "/"
  if ($parts.Length -lt 5) {
    return $null
  }
  if ($parts[2] -ne "releases") {
    return $null
  }

  $owner = $parts[0]
  $repo = $parts[1]
  $releaseApi = $null
  $assetName = $null
  if ($parts[3] -eq "download" -and $parts.Length -ge 6) {
    $tag = [System.Uri]::UnescapeDataString($parts[4])
    $assetName = [System.Uri]::UnescapeDataString(($parts[5..($parts.Length - 1)] -join "/"))
    $releaseApi = "https://api.github.com/repos/$owner/$repo/releases/tags/$tag"
  } elseif ($parts[3] -eq "latest" -and $parts.Length -ge 6 -and $parts[4] -eq "download") {
    $assetName = [System.Uri]::UnescapeDataString(($parts[5..($parts.Length - 1)] -join "/"))
    $releaseApi = "https://api.github.com/repos/$owner/$repo/releases/latest"
  } else {
    return $null
  }
  $token = Get-GitHubToken
  if (-not $token) {
    return $null
  }

  $headers = @{
    Authorization = "Bearer $token"
    Accept = "application/vnd.github+json"
    "X-GitHub-Api-Version" = "2022-11-28"
  }
  $release = Invoke-RestMethod -Uri $releaseApi -Headers $headers
  foreach ($asset in @($release.assets)) {
    if ($asset.name -eq $assetName) {
      return $asset.url
    }
  }

  throw "GitHub release $releaseApi does not include asset $assetName"
}

function Find-NvidiaGpu {
  $nvidiaSmi = Get-Command nvidia-smi -ErrorAction SilentlyContinue
  if (-not $nvidiaSmi) {
    return $null
  }

  try {
    $line = & $nvidiaSmi.Source --query-gpu=name,memory.total --format=csv,noheader,nounits 2>$null |
      Select-Object -First 1
    if (-not $line) {
      return $null
    }

    $parts = $line -split ",", 2
    $name = $parts[0].Trim()
    $vramMb = $null
    if ($parts.Count -gt 1) {
      $parsed = 0
      if ([int]::TryParse($parts[1].Trim(), [ref]$parsed)) {
        $vramMb = $parsed
      }
    }

    return [pscustomobject]@{
      Name = $name
      VramMb = $vramMb
    }
  } catch {
    return $null
  }
}

function Get-WindowsTarget {
  if (-not $IsWindows -and $PSVersionTable.PSEdition -eq "Core") {
    throw "unsupported operating system: this bootstrapper is for Windows"
  }

  $arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
  switch ($arch.ToString()) {
    "X64" { return "x86_64-pc-windows-msvc" }
    default { throw "unsupported Windows architecture: $arch" }
  }
}

function Read-ChecksumHash {
  param([string]$Path)

  $content = (Get-Content -Path $Path -Raw).Trim()
  if (-not $content) {
    throw "checksum file is empty"
  }

  return ($content -split "\s+")[0].ToUpperInvariant()
}

function Read-ReleaseManifest {
  param([string]$Path)

  try {
    return Get-Content -Path $Path -Raw | ConvertFrom-Json
  } catch {
    throw "release manifest is not valid JSON"
  }
}

function Get-OpenGpuHome {
  if ($env:OPENGPU_HOME) {
    return $env:OPENGPU_HOME
  }

  return (Join-Path $env:USERPROFILE ".opengpu")
}

function ConvertTo-Hashtable {
  param([object]$Value)

  if ($null -eq $Value) {
    return $null
  }

  if ($Value -is [System.Collections.IDictionary]) {
    $table = [ordered]@{}
    foreach ($key in $Value.Keys) {
      $table[$key] = ConvertTo-Hashtable -Value $Value[$key]
    }
    return $table
  }

  if ($Value -is [System.Management.Automation.PSCustomObject]) {
    $table = [ordered]@{}
    foreach ($property in $Value.PSObject.Properties) {
      $table[$property.Name] = ConvertTo-Hashtable -Value $property.Value
    }
    return $table
  }

  if ($Value -is [System.Collections.IEnumerable] -and $Value -isnot [string]) {
    $items = @()
    foreach ($item in $Value) {
      $items += ConvertTo-Hashtable -Value $item
    }
    return $items
  }

  return $Value
}

function Find-ManifestRuntimeAsset {
  param(
    [object]$Manifest,
    [string]$Name
  )

  if (-not $Manifest -or -not $Manifest.runtime_assets) {
    return $null
  }

  foreach ($asset in @($Manifest.runtime_assets)) {
    if ($asset.name -eq $Name) {
      return $asset
    }
  }

  return $null
}

function Find-ManifestReleaseAsset {
  param(
    [object]$Manifest,
    [string]$Name
  )

  if ($Manifest -and $Manifest.binary_name -eq $Name) {
    return [pscustomobject]@{
      name = $Manifest.binary_name
      checksum_sha256 = $Manifest.checksum_sha256
    }
  }

  if (-not $Manifest -or -not $Manifest.assets) {
    return $null
  }

  foreach ($asset in @($Manifest.assets)) {
    if ($asset.name -eq $Name) {
      return $asset
    }
  }

  return $null
}

function Save-TrustedRuntimePath {
  param(
    [string]$RuntimeName,
    [string]$RuntimePath,
    [string]$Checksum
  )

  $opengpuHome = Get-OpenGpuHome
  New-Item -ItemType Directory -Force -Path $opengpuHome | Out-Null
  $trustedPath = Join-Path $opengpuHome "trusted-runtime-paths.json"
  $trusted = [ordered]@{}

  if (Test-Path -LiteralPath $trustedPath) {
    $existing = Get-Content -Path $trustedPath -Raw | ConvertFrom-Json
    $trusted = ConvertTo-Hashtable -Value $existing
  }

  $trusted[$RuntimeName] = [ordered]@{
    path = [System.IO.Path]::GetFullPath($RuntimePath)
    sha256 = $Checksum.ToLowerInvariant()
  }

  $trusted | ConvertTo-Json -Depth 8 | Set-Content -Path $trustedPath -Encoding ASCII
  return $trustedPath
}

function Copy-ReleaseFile {
  param(
    [string]$Source,
    [string]$Destination
  )

  $uri = $null
  if ([System.Uri]::TryCreate($Source, [System.UriKind]::Absolute, [ref]$uri) -and $uri.IsFile) {
    Copy-Item -LiteralPath $uri.LocalPath -Destination $Destination
    return
  }

  if (Test-Path -LiteralPath $Source) {
    Copy-Item -LiteralPath $Source -Destination $Destination
    return
  }

  $token = Get-GitHubToken
  $downloadSource = $Source
  $apiSource = Resolve-GitHubReleaseAssetApiUrl -Source $Source
  if ($apiSource) {
    $downloadSource = $apiSource
  }
  $headers = @{}
  if ($token -and ($Source -like "https://github.com/*" -or $downloadSource -like "https://api.github.com/*")) {
    $headers["Authorization"] = "Bearer $token"
    $headers["Accept"] = "application/octet-stream"
    $headers["X-GitHub-Api-Version"] = "2022-11-28"
  }

  try {
    if ($headers.Count -gt 0) {
      Invoke-WebRequest -Uri $downloadSource -Headers $headers -OutFile $Destination
    } else {
      Invoke-WebRequest -Uri $downloadSource -OutFile $Destination
    }
  } catch {
    if ($Source -like "https://github.com/*") {
      if ($token) {
        throw "failed to download private GitHub release asset from $Source. Verify the token has access to this repository and release assets. Original error: $($_.Exception.Message)"
      }
      throw "failed to download GitHub release asset from $Source. If this repository or release is private, pass -GitHubToken or set GITHUB_TOKEN/GH_TOKEN with release read access. Original error: $($_.Exception.Message)"
    }
    throw
  }
  Assert-DownloadedReleaseFile -Source $Source -Destination $Destination
}

function Verify-ReleaseAsset {
  param(
    [string]$ReleaseBase,
    [string]$AssetName,
    [string]$Destination,
    [object]$ManifestAsset
  )

  if (-not $ManifestAsset -or -not $ManifestAsset.checksum_sha256) {
    if (-not $AllowUnsignedLocalPreview) {
      throw "strict installer verification failed: release asset $AssetName is missing from release manifest"
    }
    Write-Warning "dev-only local preview override: release asset $AssetName is missing from release manifest, continuing with checksum verification"
  }

  $checksumDestination = "$Destination.sha256"
  Copy-ReleaseFile -Source "$ReleaseBase/$AssetName" -Destination $Destination
  Copy-ReleaseFile -Source "$ReleaseBase/$AssetName.sha256" -Destination $checksumDestination
  $expected = Read-ChecksumHash -Path $checksumDestination
  if ($ManifestAsset -and $ManifestAsset.checksum_sha256) {
    $manifestChecksum = $ManifestAsset.checksum_sha256.ToString().ToUpperInvariant()
    if ($manifestChecksum -ne $expected) {
      throw "release manifest checksum does not match $AssetName.sha256"
    }
  }
  $actual = (Get-FileHash -Algorithm SHA256 -Path $Destination).Hash.ToUpperInvariant()
  if ($expected -ne $actual) {
    throw "checksum mismatch for $AssetName"
  }
  return $expected
}

if ($Help) {
  Show-Usage
  exit 0
}

$target = Get-WindowsTarget
$gpu = Find-NvidiaGpu
$profile = if ($gpu) { "windows-x86_64-cuda" } else { "windows-x86_64-generic" }
$assetName = "opengpu-$target.exe"
$agentAssetName = "opengpu-node-agent-$target.exe"
$trayAssetName = "mundusx-tray-$target.exe"
$trayIconAssetName = "mundusx.ico"
$cudaRuntimeRequired = [bool]($gpu -or $InstallCudaRuntime)
$cudaRuntimeAssetName = "llama-runtime-$target-cuda.zip"
$releaseBase = $ReleaseBaseUrl.TrimEnd("/")
$releaseUrl = "$releaseBase/$assetName"
$checksumUrl = "$releaseUrl.sha256"
$manifestUrl = "$releaseBase/release-manifest.json"
$signatureUrl = "$releaseBase/release-manifest.json.sig"
$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("opengpu-install-" + [System.Guid]::NewGuid().ToString("N"))
$tempExe = Join-Path $tempDir $assetName
$tempChecksum = Join-Path $tempDir "$assetName.sha256"
$tempManifest = Join-Path $tempDir "release-manifest.json"
$tempSignature = Join-Path $tempDir "release-manifest.json.sig"
$tempAgent = Join-Path $tempDir $agentAssetName
$tempTray = Join-Path $tempDir $trayAssetName
$tempTrayIcon = Join-Path $tempDir $trayIconAssetName
$tempCudaRuntime = Join-Path $tempDir $cudaRuntimeAssetName
$finalExe = Join-Path $InstallDir "opengpu.exe"
$compatExe = Join-Path $InstallDir "mundusx.exe"
$finalAgent = Join-Path $InstallDir "opengpu-node-agent.exe"
$finalTray = Join-Path $InstallDir "mundusx-tray.exe"
$finalTrayIcon = Join-Path $InstallDir "mundusx.ico"
$runtimeInstallDir = Join-Path (Get-OpenGpuHome) "runtimes\llama"
$finalCudaRuntime = Join-Path $runtimeInstallDir "llama-cli.exe"
$finalCudaServerRuntime = Join-Path $runtimeInstallDir "llama-server.exe"
$manifest = $null
$trustedRuntimePath = $null
$agentExpected = $null
$trayExpected = $null
$trayIconExpected = $null
$runtimeExpected = $null

Write-Output "MundusX Windows installer"
Write-Output "  target: $target"
Write-Output "  profile: $profile"
Write-Output "  gpu: $(if ($gpu) { $gpu.Name } else { 'none detected' })"
Write-Output "  cuda vram: $(if ($gpu -and $gpu.VramMb) { "$($gpu.VramMb) MB" } else { 'none detected' })"
Write-Output "  source: $releaseBase"
Write-Output "  asset: $assetName"
Write-Output "  node agent: $agentAssetName"
Write-Output "  tray companion: $trayAssetName"
Write-Output "  cuda runtime: $(if ($cudaRuntimeRequired) { $cudaRuntimeAssetName } else { 'not required' })"
Write-Output "  install: $InstallDir"
Write-Output "  verification: $(if ($AllowUnsignedLocalPreview) { 'local preview override' } else { 'strict enterprise' })"
Write-Output ""

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
New-Item -ItemType Directory -Force -Path $tempDir | Out-Null

try {
  Write-Output "Fetching opengpu..."
  Copy-ReleaseFile -Source $releaseUrl -Destination $tempExe

  $expected = $null
  try {
    Copy-ReleaseFile -Source $checksumUrl -Destination $tempChecksum
    Write-Output "Verifying checksum..."
    $expected = Read-ChecksumHash -Path $tempChecksum
    $actual = (Get-FileHash -Algorithm SHA256 -Path $tempExe).Hash.ToUpperInvariant()
    if ($expected -ne $actual) {
      throw "checksum mismatch for $assetName"
    }
  } catch {
    if (-not $AllowUnsignedLocalPreview) {
      throw "strict installer verification failed: checksum unavailable or invalid for $assetName ($($_.Exception.Message))"
    }
    Write-Warning "dev-only local preview override: checksum unavailable or invalid for $assetName, continuing without verification"
  }

  try {
    Copy-ReleaseFile -Source $manifestUrl -Destination $tempManifest
    Copy-ReleaseFile -Source $signatureUrl -Destination $tempSignature
    Write-Output "Checking signed release manifest..."
    $manifest = Read-ReleaseManifest -Path $tempManifest
    if ($manifest.artifact_kind -ne "release-binary") {
      throw "release manifest artifact_kind is not release-binary"
    }
    if ($manifest.binary_name -ne $assetName) {
      throw "release manifest binary_name does not match $assetName"
    }
    if (-not $manifest.checksum_sha256) {
      throw "release manifest checksum_sha256 is empty"
    }
    if ($expected -and ($manifest.checksum_sha256.ToString().ToUpperInvariant() -ne $expected)) {
      throw "release manifest checksum does not match $assetName.sha256"
    }
    if (-not (Test-Path -LiteralPath $tempSignature)) {
      throw "release manifest signature is missing"
    }
    if ((Get-Item -LiteralPath $tempSignature).Length -le 0) {
      throw "release manifest signature is empty"
    }
  } catch {
    if (-not $AllowUnsignedLocalPreview) {
      throw "strict installer verification failed: signed release manifest unavailable or invalid ($($_.Exception.Message))"
    }
    Write-Warning "dev-only local preview override: signed release manifest unavailable or invalid, continuing without signature verification"
  }
  if ($cudaRuntimeRequired) {
    $runtimeManifestAsset = Find-ManifestRuntimeAsset -Manifest $manifest -Name $cudaRuntimeAssetName
    Write-Output "Fetching CUDA llama runtime..."
    Write-Output "Verifying CUDA runtime checksum..."
    $runtimeExpected = Verify-ReleaseAsset -ReleaseBase $releaseBase -AssetName $cudaRuntimeAssetName -Destination $tempCudaRuntime -ManifestAsset $runtimeManifestAsset
  }

  $agentManifestAsset = Find-ManifestReleaseAsset -Manifest $manifest -Name $agentAssetName
  Write-Output "Fetching node agent..."
  Write-Output "Verifying node agent checksum..."
  $agentExpected = Verify-ReleaseAsset -ReleaseBase $releaseBase -AssetName $agentAssetName -Destination $tempAgent -ManifestAsset $agentManifestAsset

  $trayManifestAsset = Find-ManifestReleaseAsset -Manifest $manifest -Name $trayAssetName
  Write-Output "Fetching Windows tray companion..."
  Write-Output "Verifying Windows tray companion checksum..."
  $trayExpected = Verify-ReleaseAsset -ReleaseBase $releaseBase -AssetName $trayAssetName -Destination $tempTray -ManifestAsset $trayManifestAsset

  $trayIconManifestAsset = Find-ManifestReleaseAsset -Manifest $manifest -Name $trayIconAssetName
  Write-Output "Fetching Windows tray icon..."
  Write-Output "Verifying Windows tray icon checksum..."
  $trayIconExpected = Verify-ReleaseAsset -ReleaseBase $releaseBase -AssetName $trayIconAssetName -Destination $tempTrayIcon -ManifestAsset $trayIconManifestAsset

  Move-Item -Force -Path $tempExe -Destination $finalExe
  Copy-Item -Force -LiteralPath $finalExe -Destination $compatExe
  Move-Item -Force -Path $tempAgent -Destination $finalAgent
  Move-Item -Force -Path $tempTray -Destination $finalTray
  Move-Item -Force -Path $tempTrayIcon -Destination $finalTrayIcon

  if ($cudaRuntimeRequired) {
    if (Test-Path -LiteralPath $runtimeInstallDir) {
      Remove-Item -Recurse -Force -LiteralPath $runtimeInstallDir
    }
    New-Item -ItemType Directory -Force -Path $runtimeInstallDir | Out-Null
    Expand-Archive -LiteralPath $tempCudaRuntime -DestinationPath $runtimeInstallDir -Force
    if (-not (Test-Path -LiteralPath $finalCudaRuntime)) {
      $foundRuntime = Get-ChildItem -Path $runtimeInstallDir -Recurse -Filter "llama-cli.exe" | Select-Object -First 1
      if (-not $foundRuntime) {
        throw "CUDA runtime bundle did not contain llama-cli.exe"
      }
      Copy-Item -LiteralPath $foundRuntime.FullName -Destination $finalCudaRuntime
    }
    $runtimeExeChecksum = (Get-FileHash -Algorithm SHA256 -Path $finalCudaRuntime).Hash.ToUpperInvariant()
    $trustedRuntimePath = Save-TrustedRuntimePath -RuntimeName "llama_cli" -RuntimePath $finalCudaRuntime -Checksum $runtimeExeChecksum
    $foundServerRuntime = Get-ChildItem -Path $runtimeInstallDir -Recurse -Filter "llama-server.exe" | Select-Object -First 1
    if ($foundServerRuntime) {
      if ($foundServerRuntime.FullName -ne $finalCudaServerRuntime) {
        Copy-Item -LiteralPath $foundServerRuntime.FullName -Destination $finalCudaServerRuntime
      }
      $serverRuntimeExeChecksum = (Get-FileHash -Algorithm SHA256 -Path $finalCudaServerRuntime).Hash.ToUpperInvariant()
      $trustedRuntimePath = Save-TrustedRuntimePath -RuntimeName "llama_server" -RuntimePath $finalCudaServerRuntime -Checksum $serverRuntimeExeChecksum
    }
  }
} finally {
  Remove-Item -Recurse -Force -Path $tempDir -ErrorAction SilentlyContinue
}

Write-Output ""
Write-Output "Installed opengpu to $finalExe"
Write-Output "Installed mundusx compatibility alias to $compatExe"
Write-Output "Installed node agent to $finalAgent"
Write-Output "Installed tray companion to $finalTray"
Write-Output "Installed tray icon to $finalTrayIcon"
Write-Output "Tray icon checksum: $($trayIconExpected.ToLowerInvariant())"
if (-not $SkipTrayAutoStart) {
  $runKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
  New-Item -Path $runKey -Force | Out-Null
  New-ItemProperty -Path $runKey -Name "MundusX" -Value ('"' + $finalTray + '"') -PropertyType String -Force | Out-Null
  Write-Output "Configured MundusX tray to start at sign-in"
  Start-Process -FilePath $finalTray
  Write-Output "Started MundusX tray companion"
}
if ($cudaRuntimeRequired) {
  Write-Output "Installed CUDA llama runtime bundle to $runtimeInstallDir"
  Write-Output "Runtime bundle checksum: $($runtimeExpected.ToLowerInvariant())"
  Write-Output "Pinned trusted runtime path in $trustedRuntimePath"
}

$pathEntries = ($env:PATH -split ";") | ForEach-Object { $_.TrimEnd("\") }
$normalizedInstallDir = (Resolve-Path -Path $InstallDir).Path.TrimEnd("\")
if ($pathEntries -notcontains $normalizedInstallDir) {
  Write-Warning "$InstallDir is not currently on PATH. Add it to PATH or run $finalExe directly."
}

Write-Output "Next: opengpu install"


