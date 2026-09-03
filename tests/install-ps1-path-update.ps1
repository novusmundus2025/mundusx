$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$installerPath = Join-Path $repoRoot "install.ps1"
$tokens = $null
$parseErrors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile(
  $installerPath,
  [ref]$tokens,
  [ref]$parseErrors
)
if ($parseErrors.Count -gt 0) {
  throw "install.ps1 contains PowerShell parse errors"
}

$functionAst = $ast.Find({
  param($node)
  $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
    $node.Name -eq "Add-DirectoryToUserPath"
}, $true)
if (-not $functionAst) {
  throw "Add-DirectoryToUserPath was not found"
}

$originalUserPath = [Environment]::GetEnvironmentVariable("Path", "User")
$originalProcessPath = $env:PATH
$probePath = Join-Path ([System.IO.Path]::GetTempPath()) "mundusx-path-probe"
try {
  $function = $functionAst.Body.GetScriptBlock()
  & $function -Directory $probePath | Out-Null

  $persistedEntries = [Environment]::GetEnvironmentVariable("Path", "User") -split ";"
  if (-not ($persistedEntries | Where-Object {
    $_.TrimEnd("\").Equals($probePath.TrimEnd("\"), [System.StringComparison]::OrdinalIgnoreCase)
  })) {
    throw "installer did not persist the install directory in the user PATH"
  }
  if (-not (($env:PATH -split ";") | Where-Object {
    $_.TrimEnd("\").Equals($probePath.TrimEnd("\"), [System.StringComparison]::OrdinalIgnoreCase)
  })) {
    throw "installer did not update PATH for its current process"
  }
} finally {
  [Environment]::SetEnvironmentVariable("Path", $originalUserPath, "User")
  $env:PATH = $originalProcessPath
}

Write-Output "PASS: install.ps1 persists the OpenGPU install directory in the user PATH"
