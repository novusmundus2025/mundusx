$ErrorActionPreference = 'Stop'
$tokens = $null
$errors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot '../install.ps1'), [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw 'Installer syntax errors' }
$helper = $ast.Find({ param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Wait-InstallerDownloadTask' }, $true)
Invoke-Expression $helper.Extent.Text
$stream = [System.IO.MemoryStream]::new([byte[]](1,2,3,4))
$buffer = [byte[]]::new(4)
$count = Wait-InstallerDownloadTask -Task ($stream.ReadAsync($buffer, 0, 4)) -Label 'Test download'
if ($count -ne 4 -or $buffer[3] -ne 4) { throw 'Async download result was changed' }
$failed = [System.Threading.Tasks.TaskCompletionSource[int]]::new()
$failed.SetException([System.Exception]::new('fixture failure'))
$threw = $false
try { Wait-InstallerDownloadTask -Task $failed.Task -Label 'Test failure' } catch { $threw = $true }
if (-not $threw) { throw 'Task failure was swallowed' }
$stream.Dispose()
Write-Output 'Windows download progress preserves bytes and failures.'
