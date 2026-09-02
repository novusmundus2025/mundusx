param(
  [string]$ChatUrl = "https://chat.mundusx.ai",
  [string]$ReleaseBaseUrl = "https://github.com/mundusx/releases/releases/latest/download",
  [string]$InstallDir = "$env:USERPROFILE\.mundusx\bin"
)

$ErrorActionPreference = "Stop"
$asset = "mundusx-harness-runner-x86_64-pc-windows-msvc.exe"
$releaseBase = $ReleaseBaseUrl.TrimEnd("/")
$temporary = Join-Path ([System.IO.Path]::GetTempPath()) ("mundusx-harness-" + [Guid]::NewGuid().ToString("N"))
$download = Join-Path $temporary $asset
$checksumFile = "$download.sha256"
$destination = Join-Path $InstallDir "mundusx-harness-runner.exe"

try {
  if (-not (Get-Command git.exe -ErrorAction SilentlyContinue)) {
    throw "Git is required for local project isolation. Install Git for Windows, then run this setup again."
  }
  New-Item -ItemType Directory -Force -Path $temporary | Out-Null
  New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
  Invoke-WebRequest -UseBasicParsing -Uri "$releaseBase/$asset" -OutFile $download
  Invoke-WebRequest -UseBasicParsing -Uri "$releaseBase/$asset.sha256" -OutFile $checksumFile
  $expected = ((Get-Content -LiteralPath $checksumFile -Raw).Trim() -split "\s+")[0].ToLowerInvariant()
  $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $download).Hash.ToLowerInvariant()
  if ($expected -notmatch "^[0-9a-f]{64}$" -or $actual -ne $expected) {
    throw "Harness runner checksum verification failed"
  }
  Move-Item -Force -LiteralPath $download -Destination $destination
  & $destination bootstrap --chat-url $ChatUrl
  if ($LASTEXITCODE -ne 0) { throw "Harness runner connection failed with exit code $LASTEXITCODE" }
} finally {
  Remove-Item -LiteralPath $temporary -Recurse -Force -ErrorAction SilentlyContinue
}
