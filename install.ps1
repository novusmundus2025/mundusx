param(
  [string]$InstallDir = "$env:USERPROFILE\.opengpu\bin",
  [string]$ReleaseBaseUrl = "https://github.com/mundusx/releases/releases/download/opengpu-prod",
  [string]$GitHubToken = "",
  [switch]$AllowUnsignedLocalPreview,
  [switch]$InstallCudaRuntime,
  [switch]$InstallVulkanRuntime,
  [switch]$SkipModelRuntime,
  [switch]$SkipTrayAutoStart,
  [switch]$SkipPathUpdate,
  [switch]$SkipContributorSetup,
  [string]$Workspace = "",
  [switch]$SkipChatConnect,
  [ValidateSet("developer", "contributor", "both")]
  [string]$SetupMode,
  [ValidateSet("hermes", "native", "none")]
  [string]$AgentMode = "none",
  [switch]$Help
)

$agentModeWasProvided = $PSBoundParameters.ContainsKey("AgentMode")
$setupModeWasProvided = $PSBoundParameters.ContainsKey("SetupMode")
$ErrorActionPreference = "Stop"
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
Add-Type -AssemblyName System.Net.Http

$installerLogDir = Join-Path (Split-Path -Parent $InstallDir) "logs"
$installerLogPath = Join-Path $installerLogDir "installer.log"
$script:installerTranscriptStarted = $false
New-Item -ItemType Directory -Force -Path $installerLogDir | Out-Null
try {
  Start-Transcript -Path $installerLogPath -Force | Out-Null
  $script:installerTranscriptStarted = $true
} catch {
  Write-Warning "Installer logging could not be started: $($_.Exception.Message)"
}
trap {
  [Console]::Error.WriteLine($_.Exception.Message)
  if ($script:installerTranscriptStarted) {
    Stop-Transcript | Out-Null
  }
  exit 1
}

function Show-Usage {
  @"
Usage:
  powershell -ExecutionPolicy Bypass -File .\install.ps1

Options:
  -InstallDir <path>       Directory where opengpu.exe will be installed.
  -ReleaseBaseUrl <url>    Release download base URL.
  -GitHubToken <token>     Optional token for private GitHub release assets.
  -InstallCudaRuntime      Force CUDA llama runtime installation and pinning.
  -InstallVulkanRuntime    Force Vulkan llama runtime installation instead of CUDA.
                          By default, the installer detects NVIDIA/CUDA and
                          otherwise installs the Vulkan runtime for Windows.
  -SkipModelRuntime        Agent-only install: do not download llama.cpp or a GPU runtime.
  -SkipTrayAutoStart       Install the tray companion without starting it at sign-in.
  -SkipPathUpdate          Test-only: do not add the install directory to the user PATH.
  -SkipContributorSetup    Do not open a fresh PowerShell window for `opengpu install`.
  -Workspace <path>        Local project root. Defaults to <home>\MundusX\Projects.
  -SkipChatConnect         Install the selected agent without connecting it to Chat.
  -SetupMode <mode>        Guided role: developer, contributor, or both.
  -AgentMode <mode>        Agent harness: hermes, native, or none.
                          When omitted, asks interactively and defaults to none.
  -AllowUnsignedLocalPreview
                          Dev-only: allow missing checksum or signed manifest
                          when testing a local release preview.
  -Help                    Print this help and exit.

After this bootstrapper installs the binary, a fresh PowerShell window opens
and runs `opengpu install` automatically unless -SkipContributorSetup is set.
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

function Select-AgentMode {
  Write-Host ""
  Write-Host "Would you like to configure a local coding agent now?"
  Write-Host "  1. Hermes Agent (full third-party coding harness)"
  Write-Host "  2. MundusX Agent (built in)"
  Write-Host "  3. Not now (select an agent later)"

  while ($true) {
    try {
      $choice = Read-Host "Choose 1, 2, or 3 [default: 3]"
    } catch {
      Write-Warning "Interactive input is unavailable; no agent will be configured now."
      return "none"
    }

    $normalizedChoice = if ($null -eq $choice) { "" } else { $choice.Trim().ToLowerInvariant() }
    switch ($normalizedChoice) {
      "1" { return "hermes" }
      "hermes" { return "hermes" }
      "2" { return "native" }
      "native" { return "native" }
      "mundusx" { return "native" }
      "" { return "none" }
      "3" { return "none" }
      "none" { return "none" }
      "later" { return "none" }
      default { Write-Warning "Please enter 1, 2, or 3." }
    }
  }
}

function Select-SetupMode {
  Write-Host ""
  Write-Host "How will you use MundusX?"
  Write-Host "  1. Develop with AI on my local projects (recommended)"
  Write-Host "  2. Contribute compute to the MundusX network"
  Write-Host "  3. Both development and compute contribution"
  while ($true) {
    $choice = Read-Host "Choose 1, 2, or 3 [default: 1]"
    switch ($choice.Trim().ToLowerInvariant()) {
      "" { return "developer" }
      "1" { return "developer" }
      "developer" { return "developer" }
      "2" { return "contributor" }
      "contributor" { return "contributor" }
      "3" { return "both" }
      "both" { return "both" }
      default { Write-Warning "Please enter 1, 2, or 3." }
    }
  }
}

function Select-Workspace {
  $defaultWorkspace = Join-Path $env:USERPROFILE "MundusX\Projects"
  $choice = Read-Host "Local project folder [default: $defaultWorkspace]"
  if ([string]::IsNullOrWhiteSpace($choice)) { return $defaultWorkspace }
  return [IO.Path]::GetFullPath($choice.Trim().Trim('"'))
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
    [string]$Destination,
    [string]$Label = ""
  )

  if (-not $Label) {
    $Label = Split-Path -Leaf $Destination
  }

  $uri = $null
  if ([System.Uri]::TryCreate($Source, [System.UriKind]::Absolute, [ref]$uri) -and $uri.IsFile) {
    Copy-Item -LiteralPath $uri.LocalPath -Destination $Destination
    Write-Output "Copied $Label."
    return
  }

  if (Test-Path -LiteralPath $Source) {
    Copy-Item -LiteralPath $Source -Destination $Destination
    Write-Output "Copied $Label."
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

  $handler = $null
  $client = $null
  $request = $null
  $response = $null
  $inputStream = $null
  $outputStream = $null
  try {
    $handler = New-Object System.Net.Http.HttpClientHandler
    $handler.AllowAutoRedirect = $true
    $client = New-Object System.Net.Http.HttpClient($handler)
    $client.Timeout = [TimeSpan]::FromHours(2)
    $client.DefaultRequestHeaders.UserAgent.ParseAdd("MundusX-Installer/1.0")

    $request = New-Object System.Net.Http.HttpRequestMessage([System.Net.Http.HttpMethod]::Get, $downloadSource)
    foreach ($name in $headers.Keys) {
      [void]$request.Headers.TryAddWithoutValidation($name, [string]$headers[$name])
    }

    $response = $client.SendAsync(
      $request,
      [System.Net.Http.HttpCompletionOption]::ResponseHeadersRead
    ).GetAwaiter().GetResult()
    [void]$response.EnsureSuccessStatusCode()

    $totalBytes = $response.Content.Headers.ContentLength
    $inputStream = $response.Content.ReadAsStreamAsync().GetAwaiter().GetResult()
    $outputStream = [System.IO.File]::Open(
      $Destination,
      [System.IO.FileMode]::Create,
      [System.IO.FileAccess]::Write,
      [System.IO.FileShare]::None
    )
    $buffer = New-Object byte[] (1024 * 1024)
    $receivedBytes = [long]0
    $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    $lastUpdateMs = [long]-1000

    while (($read = $inputStream.Read($buffer, 0, $buffer.Length)) -gt 0) {
      $outputStream.Write($buffer, 0, $read)
      $receivedBytes += $read
      if (($stopwatch.ElapsedMilliseconds - $lastUpdateMs) -lt 250 -and
          (-not $totalBytes -or $receivedBytes -lt $totalBytes)) {
        continue
      }
      $lastUpdateMs = $stopwatch.ElapsedMilliseconds
      $elapsedSeconds = [Math]::Max($stopwatch.Elapsed.TotalSeconds, 0.001)
      $megabytes = $receivedBytes / 1MB
      $speedMbps = ($receivedBytes / 1MB) / $elapsedSeconds

      if ($totalBytes -and $totalBytes -gt 0) {
        $percent = [Math]::Min(100, [int][Math]::Floor(($receivedBytes * 100.0) / $totalBytes))
        $totalMegabytes = $totalBytes / 1MB
        $remainingSeconds = if ($speedMbps -gt 0) {
          [Math]::Max(0, (($totalBytes - $receivedBytes) / 1MB) / $speedMbps)
        } else {
          0
        }
        $status = "${percent}% of 100% - $($megabytes.ToString('N1'))/$($totalMegabytes.ToString('N1')) MB - $($speedMbps.ToString('N1')) MB/s - about $([Math]::Ceiling($remainingSeconds))s remaining"
        Write-Progress -Id 1 -Activity "Downloading $Label" -Status $status -PercentComplete $percent
      } else {
        $status = "$($megabytes.ToString('N1')) MB downloaded - $($speedMbps.ToString('N1')) MB/s - total size unavailable"
        Write-Progress -Id 1 -Activity "Downloading $Label" -Status $status -PercentComplete -1
      }
    }

    $stopwatch.Stop()
    Write-Progress -Id 1 -Activity "Downloading $Label" -Completed
    $finalMegabytes = $receivedBytes / 1MB
    Write-Output "Downloaded ${Label}: 100% - $($finalMegabytes.ToString('N1')) MB in $([Math]::Ceiling($stopwatch.Elapsed.TotalSeconds))s."
  } catch {
    Write-Progress -Id 1 -Activity "Downloading $Label" -Completed
    if (Test-Path -LiteralPath $Destination) {
      Remove-Item -Force -LiteralPath $Destination -ErrorAction SilentlyContinue
    }
    if ($Source -like "https://github.com/*") {
      if ($token) {
        throw "failed to download private GitHub release asset from $Source. Verify the token has access to this repository and release assets. Original error: $($_.Exception.Message)"
      }
      throw "failed to download GitHub release asset from $Source. If this repository or release is private, pass -GitHubToken or set GITHUB_TOKEN/GH_TOKEN with release read access. Original error: $($_.Exception.Message)"
    }
    throw
  } finally {
    if ($outputStream) { $outputStream.Dispose() }
    if ($inputStream) { $inputStream.Dispose() }
    if ($response) { $response.Dispose() }
    if ($request) { $request.Dispose() }
    if ($client) { $client.Dispose() }
    if ($handler) { $handler.Dispose() }
  }
  Assert-DownloadedReleaseFile -Source $Source -Destination $Destination
}

function Add-DirectoryToUserPath {
  param([string]$Directory)

  $normalizedDirectory = [System.IO.Path]::GetFullPath($Directory).TrimEnd("\")
  $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
  $userEntries = @($userPath -split ";" | Where-Object { $_ } | ForEach-Object { $_.TrimEnd("\") })
  $alreadyPersisted = [bool]($userEntries | Where-Object {
    $_.Equals($normalizedDirectory, [System.StringComparison]::OrdinalIgnoreCase)
  })

  if (-not $alreadyPersisted) {
    $updatedUserPath = if ([string]::IsNullOrWhiteSpace($userPath)) {
      $normalizedDirectory
    } else {
      $userPath.TrimEnd(";") + ";" + $normalizedDirectory
    }
    [Environment]::SetEnvironmentVariable("Path", $updatedUserPath, "User")
    Write-Output "Added $normalizedDirectory to the persistent user PATH."
  } else {
    Write-Output "$normalizedDirectory is already present in the persistent user PATH."
  }

  $processEntries = @($env:PATH -split ";" | Where-Object { $_ } | ForEach-Object { $_.TrimEnd("\") })
  $alreadyInProcess = [bool]($processEntries | Where-Object {
    $_.Equals($normalizedDirectory, [System.StringComparison]::OrdinalIgnoreCase)
  })
  if (-not $alreadyInProcess) {
    $env:PATH = $env:PATH.TrimEnd(";") + ";" + $normalizedDirectory
  }
}

function Start-ContributorSetup {
  param([string]$CliPath)

  if (-not (Test-Path -LiteralPath $CliPath -PathType Leaf)) {
    throw "installed opengpu was not found at $CliPath"
  }

  $escapedCliPath = $CliPath.Replace("'", "''")
  $setupCommand = @"
& '$escapedCliPath' install
`$setupExit = `$LASTEXITCODE
Write-Host ''
if (`$setupExit -eq 0) {
  Write-Host 'Contributor setup finished. Run opengpu start when you are ready to contribute.' -ForegroundColor Green
} else {
  Write-Host 'Contributor setup failed. Review the error above.' -ForegroundColor Red
}
"@
  $encodedCommand = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($setupCommand))
  Start-Process -FilePath "powershell.exe" -ArgumentList @(
    "-NoLogo",
    "-NoProfile",
    "-NoExit",
    "-ExecutionPolicy", "Bypass",
    "-EncodedCommand", $encodedCommand
  )
  Write-Output "Opened a fresh PowerShell window and started opengpu install."
}

function Stop-InstalledOpenGpuProcesses {
  param([string]$OpenGpuHome)

  $normalizedHome = [IO.Path]::GetFullPath($OpenGpuHome).TrimEnd("\") + "\"
  $installedProcesses = @(Get-CimInstance Win32_Process -ErrorAction Stop | Where-Object {
    $_.ExecutablePath -and
    $_.ExecutablePath.StartsWith($normalizedHome, [StringComparison]::OrdinalIgnoreCase)
  })
  foreach ($process in $installedProcesses) {
    Write-Output "Stopping installed $($process.Name) before replacement..."
    Stop-Process -Id $process.ProcessId -Force -ErrorAction Stop
    Wait-Process -Id $process.ProcessId -Timeout 10 -ErrorAction SilentlyContinue
  }
}

function Write-InstallerPhase {
  param(
    [int]$Current,
    [int]$Total,
    [string]$Message
  )

  $overallPercent = [int][Math]::Floor((($Current - 1) * 100.0) / $Total)
  Write-Output ""
  Write-Output "[$Current/$Total - overall $overallPercent%] $Message"
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
  Copy-ReleaseFile -Source "$ReleaseBase/$AssetName" -Destination $Destination -Label $AssetName
  Copy-ReleaseFile -Source "$ReleaseBase/$AssetName.sha256" -Destination $checksumDestination -Label "$AssetName checksum"
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

function Start-ChatConnection {
  param(
    [string]$CliPath,
    [string]$WorkspacePath
  )

  $resolvedWorkspace = if ($WorkspacePath.Trim()) {
    [IO.Path]::GetFullPath($WorkspacePath)
  } else {
    Join-Path $env:USERPROFILE "MundusX\Projects"
  }
  New-Item -ItemType Directory -Force -Path $resolvedWorkspace | Out-Null
  $escapedCliPath = $CliPath.Replace("'", "''")
  $escapedWorkspace = $resolvedWorkspace.Replace("'", "''")
  $connectCommand = "& '$escapedCliPath' connect --workspace '$escapedWorkspace'"
  $encodedCommand = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($connectCommand))
  Start-Process -FilePath "powershell.exe" -ArgumentList @(
    "-NoLogo", "-NoProfile", "-ExecutionPolicy", "Bypass",
    "-EncodedCommand", $encodedCommand
  ) -WindowStyle Hidden
  Write-Output "Started the Chat connection in the background for workspace $resolvedWorkspace"
}

if (-not $setupModeWasProvided) {
  $SetupMode = Select-SetupMode
}
switch ($SetupMode) {
  "developer" {
    $SkipModelRuntime = $true
    $SkipContributorSetup = $true
    if (-not $Workspace.Trim()) { $Workspace = Select-Workspace }
  }
  "contributor" {
    $AgentMode = "none"
    $agentModeWasProvided = $true
    $SkipChatConnect = $true
  }
  "both" {
    if (-not $Workspace.Trim()) { $Workspace = Select-Workspace }
  }
}
if (-not $agentModeWasProvided -and $SetupMode -ne "contributor") {
  $AgentMode = Select-AgentMode
}

if ($InstallCudaRuntime -and $InstallVulkanRuntime) {
  throw "choose only one runtime override: -InstallCudaRuntime or -InstallVulkanRuntime"
}
if ($SkipModelRuntime -and ($InstallCudaRuntime -or $InstallVulkanRuntime)) {
  throw "-SkipModelRuntime cannot be combined with a GPU runtime override"
}

$target = Get-WindowsTarget
$gpu = Find-NvidiaGpu
$cudaRuntimeRequired = [bool](-not $SkipModelRuntime -and ($gpu -or $InstallCudaRuntime) -and -not $InstallVulkanRuntime)
$vulkanRuntimeRequired = [bool](-not $SkipModelRuntime -and -not $cudaRuntimeRequired)
$runtimeSelectionReason = if ($SkipModelRuntime) {
  "skipped for agent-only installation"
} elseif ($InstallCudaRuntime) {
  "forced CUDA"
} elseif ($InstallVulkanRuntime) {
  "forced Vulkan"
} elseif ($gpu) {
  "auto-detected NVIDIA/CUDA"
} else {
  "auto-selected Vulkan fallback"
}
$profile = if ($SkipModelRuntime) { "windows-x86_64-agent-only" } elseif ($cudaRuntimeRequired) { "windows-x86_64-cuda" } else { "windows-x86_64-vulkan" }
$assetName = "opengpu-$target.exe"
$agentAssetName = "opengpu-node-agent-$target.exe"
$mundusxAssetName = "mundusx-$target.exe"
$agentServerAssetName = "mundusx-agent-server-$target.exe"
$trayAssetName = "mundusx-tray-$target.exe"
$trayIconAssetName = "mundusx.ico"
$cudaRuntimeAssetName = "llama-runtime-$target-cuda.zip"
$vulkanRuntimeAssetName = "llama-runtime-$target-vulkan.zip"
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
$tempMundusx = Join-Path $tempDir $mundusxAssetName
$tempAgentServer = Join-Path $tempDir $agentServerAssetName
$tempTray = Join-Path $tempDir $trayAssetName
$tempTrayIcon = Join-Path $tempDir $trayIconAssetName
$tempCudaRuntime = Join-Path $tempDir $cudaRuntimeAssetName
$tempVulkanRuntime = Join-Path $tempDir $vulkanRuntimeAssetName
$finalExe = Join-Path $InstallDir "opengpu.exe"
$finalMundusx = Join-Path $InstallDir "mundusx.exe"
$finalAgentServer = Join-Path $InstallDir "mundusx-agent-server.exe"
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
Write-Output "  runtime: $(if ($SkipModelRuntime) { 'not included' } elseif ($cudaRuntimeRequired) { $cudaRuntimeAssetName } else { $vulkanRuntimeAssetName })"
Write-Output "  runtime selection: $runtimeSelectionReason"
Write-Output "  install: $InstallDir"
Write-Output "  verification: $(if ($AllowUnsignedLocalPreview) { 'local preview override' } else { 'strict enterprise' })"
Write-Output ""

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
New-Item -ItemType Directory -Force -Path $tempDir | Out-Null

try {
  Write-InstallerPhase -Current 1 -Total 6 -Message "Downloading OpenGPU CLI"
  Copy-ReleaseFile -Source $releaseUrl -Destination $tempExe -Label "OpenGPU CLI"

  $expected = $null
  try {
    Write-InstallerPhase -Current 2 -Total 6 -Message "Verifying the signed release"
    Copy-ReleaseFile -Source $checksumUrl -Destination $tempChecksum -Label "OpenGPU CLI checksum"
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
    Copy-ReleaseFile -Source $manifestUrl -Destination $tempManifest -Label "release manifest"
    Copy-ReleaseFile -Source $signatureUrl -Destination $tempSignature -Label "release signature"
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
  Write-InstallerPhase -Current 3 -Total 6 -Message $(if ($SkipModelRuntime) { "Skipping the local model runtime" } else { "Downloading the GPU runtime" })
  if ($SkipModelRuntime) {
    Write-Output "Agent-only profile selected; no llama.cpp server, local model, CUDA, or Vulkan runtime will be installed."
  } elseif ($cudaRuntimeRequired) {
    $runtimeManifestAsset = Find-ManifestRuntimeAsset -Manifest $manifest -Name $cudaRuntimeAssetName
    Write-Output "Fetching CUDA llama runtime..."
    Write-Output "Verifying CUDA runtime checksum..."
    $runtimeExpected = Verify-ReleaseAsset -ReleaseBase $releaseBase -AssetName $cudaRuntimeAssetName -Destination $tempCudaRuntime -ManifestAsset $runtimeManifestAsset
  } else {
    $runtimeManifestAsset = Find-ManifestRuntimeAsset -Manifest $manifest -Name $vulkanRuntimeAssetName
    Write-Output "Fetching Vulkan llama runtime..."
    Write-Output "Verifying Vulkan runtime checksum..."
    $runtimeExpected = Verify-ReleaseAsset -ReleaseBase $releaseBase -AssetName $vulkanRuntimeAssetName -Destination $tempVulkanRuntime -ManifestAsset $runtimeManifestAsset
  }

  Write-InstallerPhase -Current 4 -Total 6 -Message "Downloading the OpenGPU node agent"
  $agentManifestAsset = Find-ManifestReleaseAsset -Manifest $manifest -Name $agentAssetName
  Write-Output "Fetching node agent..."
  Write-Output "Verifying node agent checksum..."
  $agentExpected = Verify-ReleaseAsset -ReleaseBase $releaseBase -AssetName $agentAssetName -Destination $tempAgent -ManifestAsset $agentManifestAsset

  Write-Output "Fetching MundusX agent..."
  Verify-ReleaseAsset -ReleaseBase $releaseBase -AssetName $mundusxAssetName -Destination $tempMundusx -ManifestAsset (Find-ManifestReleaseAsset -Manifest $manifest -Name $mundusxAssetName) | Out-Null
  Verify-ReleaseAsset -ReleaseBase $releaseBase -AssetName $agentServerAssetName -Destination $tempAgentServer -ManifestAsset (Find-ManifestReleaseAsset -Manifest $manifest -Name $agentServerAssetName) | Out-Null

  Write-InstallerPhase -Current 5 -Total 6 -Message "Downloading the Windows tray application"
  $trayManifestAsset = Find-ManifestReleaseAsset -Manifest $manifest -Name $trayAssetName
  Write-Output "Fetching Windows tray companion..."
  Write-Output "Verifying Windows tray companion checksum..."
  $trayExpected = Verify-ReleaseAsset -ReleaseBase $releaseBase -AssetName $trayAssetName -Destination $tempTray -ManifestAsset $trayManifestAsset

  $trayIconManifestAsset = Find-ManifestReleaseAsset -Manifest $manifest -Name $trayIconAssetName
  if ($trayIconManifestAsset) {
    Write-Output "Fetching Windows tray icon..."
    Write-Output "Verifying Windows tray icon checksum..."
    $trayIconExpected = Verify-ReleaseAsset -ReleaseBase $releaseBase -AssetName $trayIconAssetName -Destination $tempTrayIcon -ManifestAsset $trayIconManifestAsset
  } else {
    Write-Warning "release manifest does not include $trayIconAssetName; tray will use the embedded/system icon fallback"
  }

  Write-InstallerPhase -Current 6 -Total 6 -Message "Installing verified components"
  Stop-InstalledOpenGpuProcesses -OpenGpuHome (Get-OpenGpuHome)
  Copy-Item -Force -LiteralPath $tempExe -Destination $finalExe
  Copy-Item -Force -LiteralPath $tempMundusx -Destination $finalMundusx
  Copy-Item -Force -LiteralPath $tempAgentServer -Destination $finalAgentServer
  Copy-Item -Force -LiteralPath $tempAgent -Destination $finalAgent
  Copy-Item -Force -LiteralPath $tempTray -Destination $finalTray
  if (Test-Path -LiteralPath $tempTrayIcon) {
    Copy-Item -Force -LiteralPath $tempTrayIcon -Destination $finalTrayIcon
  }

  if ($cudaRuntimeRequired -or $vulkanRuntimeRequired) {
    if (Test-Path -LiteralPath $runtimeInstallDir) {
      Remove-Item -Recurse -Force -LiteralPath $runtimeInstallDir
    }
    New-Item -ItemType Directory -Force -Path $runtimeInstallDir | Out-Null
    $selectedRuntimeArchive = if ($cudaRuntimeRequired) { $tempCudaRuntime } else { $tempVulkanRuntime }
    $selectedRuntimeLabel = if ($cudaRuntimeRequired) { "CUDA" } else { "Vulkan" }
    Expand-Archive -LiteralPath $selectedRuntimeArchive -DestinationPath $runtimeInstallDir -Force
    if (-not (Test-Path -LiteralPath $finalCudaRuntime)) {
      $foundRuntime = Get-ChildItem -Path $runtimeInstallDir -Recurse -Filter "llama-cli.exe" | Select-Object -First 1
      if (-not $foundRuntime) {
        throw "$selectedRuntimeLabel runtime bundle did not contain llama-cli.exe"
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
Write-Output "Installed MundusX agent to $finalMundusx"
Write-Output "Installed MundusX agent server to $finalAgentServer"
Write-Output "Installed node agent to $finalAgent"
Write-Output "Installed tray companion to $finalTray"
if ($trayIconExpected) {
  Write-Output "Installed tray icon to $finalTrayIcon"
  Write-Output "Tray icon checksum: $($trayIconExpected.ToLowerInvariant())"
}
if (-not $SkipTrayAutoStart) {
  $runKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
  New-Item -Path $runKey -Force | Out-Null
  New-ItemProperty -Path $runKey -Name "MundusX" -Value ('"' + $finalTray + '"') -PropertyType String -Force | Out-Null
  Write-Output "Configured MundusX tray to start at sign-in"
  Start-Process -FilePath $finalTray
  Write-Output "Started MundusX tray companion"
}
if ($cudaRuntimeRequired -or $vulkanRuntimeRequired) {
  $installedRuntimeLabel = if ($cudaRuntimeRequired) { "CUDA" } else { "Vulkan" }
  Write-Output "Installed $installedRuntimeLabel llama runtime bundle to $runtimeInstallDir"
  Write-Output "Runtime bundle checksum: $($runtimeExpected.ToLowerInvariant())"
  Write-Output "Pinned trusted runtime path in $trustedRuntimePath"
}

if (-not $SkipPathUpdate) {
  Add-DirectoryToUserPath -Directory $InstallDir
}

Write-Output "Configuring agent harness: $AgentMode"
if ($AgentMode -eq "hermes") {
  if (-not $env:HERMES_HOME) {
    $env:HERMES_HOME = Join-Path $env:USERPROFILE ".hermes"
  }
  Write-Output "Hermes dependency setup can take several minutes. A visible activity update will be printed every 5 seconds."
  & $finalMundusx agent install hermes
} else {
  & $finalMundusx agent use $AgentMode
}
if ($LASTEXITCODE -ne 0) {
  throw "agent harness setup failed for $AgentMode"
}

if ($SkipContributorSetup) {
  Write-Output "Next: opengpu install"
} else {
  Write-Output "Next: contributor setup will open in a fresh PowerShell window"
}
if ($AgentMode -eq "none") {
  Write-Output "Agent setup was deferred. Later, run:"
  Write-Output "  mundusx agent install hermes"
  Write-Output "  or: mundusx agent use native"
}
Write-Output ""
Write-Output "[6/6 - overall 100%] Installation complete"
if ($script:installerTranscriptStarted) {
  Stop-Transcript | Out-Null
  $script:installerTranscriptStarted = $false
}

if ($AgentMode -ne "none" -and -not $SkipChatConnect) {
  Start-ChatConnection -CliPath $finalMundusx -WorkspacePath $Workspace
}

if (-not $SkipContributorSetup) {
  Start-ContributorSetup -CliPath $finalExe
}


