param(
    [string]$Repo = "",
    [switch]$KeepKeyFiles
)

$ErrorActionPreference = "Stop"

function Fail($Message) {
    Write-Error $Message
    exit 1
}

function Require-Command($Name) {
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        Fail "Required command not found: $Name"
    }
}

function Resolve-OpenSsl {
    $command = Get-Command openssl -ErrorAction SilentlyContinue
    if ($command) {
        return $command.Source
    }

    $candidates = @(
        "C:\Program Files\Git\mingw64\bin\openssl.exe",
        "C:\Program Files\Git\usr\bin\openssl.exe"
    )

    foreach ($candidate in $candidates) {
        if (Test-Path -LiteralPath $candidate) {
            return $candidate
        }
    }

    Fail "Required command not found: openssl"
}

function To-Base64File($InputPath, $OutputPath) {
    $bytes = [System.IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $InputPath))
    [System.IO.File]::WriteAllText($OutputPath, [Convert]::ToBase64String($bytes))
}

Require-Command gh
$OpenSsl = Resolve-OpenSsl

if (-not $Repo) {
    $Repo = gh repo view --json nameWithOwner --jq ".nameWithOwner"
}

if (-not $Repo) {
    Fail "Could not resolve GitHub repository. Pass -Repo owner/name."
}

Write-Host "releaseSigningRepo: $Repo"
gh auth status | Out-Host

$tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("opengpu-release-signing-" + [guid]::NewGuid().ToString("N"))
$privateKey = Join-Path $tempRoot "release-signing-private.pem"
$publicKey = Join-Path $tempRoot "release-signing-public.pem"
$privateB64 = Join-Path $tempRoot "release-signing-private.pem.b64"
$publicB64 = Join-Path $tempRoot "release-signing-public.pem.b64"

try {
    New-Item -ItemType Directory -Path $tempRoot | Out-Null

    Write-Host "releaseSigning: generating RSA-2048 keypair"
    & $OpenSsl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out $privateKey | Out-Null
    & $OpenSsl pkey -in $privateKey -pubout -out $publicKey | Out-Null

    To-Base64File $privateKey $privateB64
    To-Base64File $publicKey $publicB64

    Write-Host "releaseSigning: uploading GitHub secrets"
    Get-Content -Raw $privateB64 | gh secret set OPENGPU_RELEASE_SIGNING_PRIVATE_KEY_PEM_B64 --repo $Repo
    if ($LASTEXITCODE -ne 0) {
        Fail "failed to upload private signing key secret"
    }
    Get-Content -Raw $publicB64 | gh secret set OPENGPU_RELEASE_SIGNING_PUBLIC_KEY_PEM_B64 --repo $Repo
    if ($LASTEXITCODE -ne 0) {
        Fail "failed to upload public signing key secret"
    }

    Write-Host "releaseSigning: verifying secret names"
    $secretList = gh secret list --repo $Repo
    if ($LASTEXITCODE -ne 0) {
        Fail "failed to list repository secrets"
    }
    $matches = $secretList | Select-String -Pattern "OPENGPU_RELEASE_SIGNING_PRIVATE_KEY_PEM_B64|OPENGPU_RELEASE_SIGNING_PUBLIC_KEY_PEM_B64"
    $matches
    if (($matches | Measure-Object).Count -lt 2) {
        Fail "release signing secret verification failed"
    }

    Write-Host "releaseSigning: configured"
}
finally {
    if ($KeepKeyFiles) {
        Write-Host "releaseSigningKeyDir: $tempRoot"
    } elseif (Test-Path -LiteralPath $tempRoot) {
        Remove-Item -LiteralPath $tempRoot -Recurse -Force
        Write-Host "releaseSigning: local key files deleted"
    }
}
