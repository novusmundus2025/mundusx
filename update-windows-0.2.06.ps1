$ErrorActionPreference = "Stop"
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$installDir = Join-Path $env:USERPROFILE ".opengpu\bin"
$logDir = Join-Path $env:USERPROFILE ".opengpu\logs"
$logPath = Join-Path $logDir "update-0.2.06.log"
$releaseBase = "https://github.com/mundusx/releases/releases/download/cli-windows-v0.2.06"
$assets = @(
  @{ Name = "mundusx.exe"; Download = "mundusx-x86_64-pc-windows-msvc.exe"; Hash = "cdc5019af7c74688571aaf84a03eb03b8894b8e9d69ddbe4c3ab0a2f7651da88" },
  @{ Name = "opengpu.exe"; Download = "opengpu-x86_64-pc-windows-msvc.exe"; Hash = "cb34262e3cc7db9bb6170179616ccd21c6b44c0dbb4ca4c9a7c5a47e0fd49240" },
  @{ Name = "opengpu-node-agent.exe"; Download = "opengpu-node-agent-x86_64-pc-windows-msvc.exe"; Hash = "d190a46de10e5ef78ce44dd135a18ec8787d521d2ce04dc8aca02de1fa1549a1" },
  @{ Name = "mundusx-agent-server.exe"; Download = "mundusx-agent-server-x86_64-pc-windows-msvc.exe"; Hash = "6834fe05b72dede9908df719608a0562419af30930fbf3064038ace93bf48479" }
)

New-Item -ItemType Directory -Path $installDir -Force | Out-Null
New-Item -ItemType Directory -Path $logDir -Force | Out-Null
Start-Transcript -Path $logPath -Force | Out-Null
$stagingDir = Join-Path ([IO.Path]::GetTempPath()) ("mundusx-update-" + [guid]::NewGuid().ToString("N"))
$backupDir = Join-Path (Split-Path $installDir -Parent) ("before-0.2.06-" + (Get-Date -Format "yyyyMMdd-HHmmss"))

try {
  New-Item -ItemType Directory -Path $stagingDir -Force | Out-Null
  New-Item -ItemType Directory -Path $backupDir -Force | Out-Null
  foreach ($asset in $assets) {
    $downloadPath = Join-Path $stagingDir $asset.Download
    Invoke-WebRequest -Uri "$releaseBase/$($asset.Download)" -OutFile $downloadPath -UseBasicParsing
    $actual = (Get-FileHash -LiteralPath $downloadPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $asset.Hash) { throw "Checksum verification failed for $($asset.Download)." }
  }

  Get-CimInstance Win32_Process | Where-Object {
    $_.ExecutablePath -and [IO.Path]::GetFullPath($_.ExecutablePath).StartsWith([IO.Path]::GetFullPath($installDir), [StringComparison]::OrdinalIgnoreCase) -and
    $_.Name -in @("mundusx.exe", "opengpu.exe", "opengpu-node-agent.exe", "mundusx-agent-server.exe", "mundusx-tray.exe")
  } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
  Start-Sleep -Milliseconds 500

  foreach ($asset in $assets) {
    $target = Join-Path $installDir $asset.Name
    if (Test-Path -LiteralPath $target) { Copy-Item -LiteralPath $target -Destination (Join-Path $backupDir $asset.Name) -Force }
    Copy-Item -LiteralPath (Join-Path $stagingDir $asset.Download) -Destination $target -Force
  }
  $installedVersion = (& (Join-Path $installDir "mundusx.exe") --version 2>&1 | Out-String).Trim()
  if ($installedVersion -ne "mundusx 0.2.06") { throw "Installed version check failed: $installedVersion" }
  $tray = Join-Path $installDir "mundusx-tray.exe"
  if (Test-Path -LiteralPath $tray) { Start-Process -FilePath $tray }
  Write-Host "MundusX 0.2.06 installed. Backup: $backupDir"
} finally {
  Remove-Item -LiteralPath $stagingDir -Recurse -Force -ErrorAction SilentlyContinue
  Stop-Transcript | Out-Null
}
