param(
  [string]$Tag = "b9856",
  [string]$OutputPath = "llama-runtime-x86_64-pc-windows-msvc-vulkan.zip"
)

$ErrorActionPreference = "Stop"
$repo = "ggml-org/llama.cpp"
$headers = @{ "User-Agent" = "mundusx-release-runtime-packager" }
if ($env:GITHUB_TOKEN) {
  $headers["Authorization"] = "Bearer $env:GITHUB_TOKEN"
  $headers["X-GitHub-Api-Version"] = "2022-11-28"
}

$release = Invoke-RestMethod -Uri "https://api.github.com/repos/$repo/releases/tags/$Tag" -Headers $headers
$assetName = "llama-$Tag-bin-win-vulkan-x64.zip"
$asset = $release.assets | Where-Object { $_.name -eq $assetName } | Select-Object -First 1
if (-not $asset) { throw "llama.cpp $Tag is missing $assetName" }

$temp = Join-Path ([System.IO.Path]::GetTempPath()) ("mundusx-llama-vulkan-" + [Guid]::NewGuid().ToString("N"))
$expanded = Join-Path $temp "expanded"
$bundle = Join-Path $temp "bundle"
New-Item -ItemType Directory -Force -Path $expanded,$bundle | Out-Null
try {
  $zip = Join-Path $temp $assetName
  Invoke-WebRequest -Uri $asset.browser_download_url -Headers $headers -OutFile $zip
  Expand-Archive -LiteralPath $zip -DestinationPath $expanded -Force

  $llamaCli = Get-ChildItem -Path $expanded -Recurse -Filter "llama-cli.exe" | Select-Object -First 1
  $llamaServer = Get-ChildItem -Path $expanded -Recurse -Filter "llama-server.exe" | Select-Object -First 1
  if (-not $llamaCli) { throw "$assetName did not contain llama-cli.exe" }
  if (-not $llamaServer) { throw "$assetName did not contain llama-server.exe" }

  Get-ChildItem -LiteralPath $llamaCli.DirectoryName -File | Copy-Item -Destination $bundle -Force
  Get-ChildItem -Path $expanded -Recurse -File -Filter "*.dll" | Copy-Item -Destination $bundle -Force
  if (-not (Get-ChildItem -LiteralPath $bundle -Filter "vulkan-1.dll" | Select-Object -First 1)) {
    Write-Warning "bundle does not include vulkan-1.dll; the installed Intel/AMD graphics driver must provide the Vulkan loader"
  }

  $resolvedOutput = [System.IO.Path]::GetFullPath($OutputPath)
  if (Test-Path -LiteralPath $resolvedOutput) { Remove-Item -Force -LiteralPath $resolvedOutput }
  $tar = Get-Command tar.exe -ErrorAction SilentlyContinue
  if (-not $tar) { throw "tar.exe is required to create the Windows runtime release archive" }
  & $tar.Source -a -c -f $resolvedOutput -C $bundle .
  if ($LASTEXITCODE -ne 0) { throw "tar.exe failed with exit code $LASTEXITCODE" }

  $checksum = (Get-FileHash -Algorithm SHA256 -LiteralPath $resolvedOutput).Hash.ToLowerInvariant()
  Set-Content -LiteralPath "$resolvedOutput.sha256" -Value "$checksum  $([System.IO.Path]::GetFileName($resolvedOutput))`n" -NoNewline -Encoding ASCII
  Write-Output "Packaged pinned llama.cpp Vulkan runtime $Tag"
  Write-Output "Runtime: $resolvedOutput"
  Write-Output "SHA-256: $checksum"
} finally {
  Remove-Item -Recurse -Force -LiteralPath $temp -ErrorAction SilentlyContinue
}
