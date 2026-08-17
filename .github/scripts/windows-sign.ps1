# Authenticode signing for Windows (SISA-PEKERJAAN §I2).
#
#   pwsh .github/scripts/windows-sign.ps1 -Path dist\Silka-Dashboard-1.4.0.msi
#   pwsh .github/scripts/windows-sign.ps1 -Path dist\*.exe, dist\*.msi
#
# Without a signature SmartScreen shows "Windows protected your PC" and hides
# the Run button behind "More info". Users do not click through that, and the
# ones who do are being trained to click through it everywhere else.
#
# ---------------------------------------------------------------------------
# Two ways to hold the key
# ---------------------------------------------------------------------------
#
# **Azure Trusted Signing** (preferred). The key never exists as a file, so
# there is nothing to leak out of a runner and nothing to rotate by hand:
#
#   AZURE_SIGNING_ENDPOINT      e.g. https://eus.codesigning.azure.net
#   AZURE_SIGNING_ACCOUNT       the Trusted Signing account name
#   AZURE_SIGNING_PROFILE       the certificate profile name
#   AZURE_TENANT_ID / AZURE_CLIENT_ID / AZURE_CLIENT_SECRET
#
# **A .pfx file** (fallback, and the only option for a certificate bought from
# a CA that predates Trusted Signing). Since June 2023 every publicly trusted
# code-signing key must live on hardware, so in practice this path means a
# cloud HSM's exported .pfx or an internal CA:
#
#   WINDOWS_CERT_PFX            base64 of the .pfx
#   WINDOWS_CERT_PASSWORD       its password
#
# In the workflow: ${{ secrets.WINDOWS_CERT_PFX }} and friends. Nothing in this
# file is a credential; every one of them is read from the environment.

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string[]] $Path,

    # RFC 3161 timestamping. A signature with no timestamp stops verifying the
    # day the certificate expires — including on machines where it was installed
    # years earlier. This is the single most commonly skipped flag and the most
    # expensive one to skip.
    [string] $TimestampUrl = 'http://timestamp.digicert.com'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Find-SignTool {
    # signtool.exe lives in a versioned Windows SDK directory. Newest first, so
    # a runner with three SDKs installed uses the one that knows about the
    # newest algorithms rather than the one that sorts first alphabetically.
    $roots = @(
        "${env:ProgramFiles(x86)}\Windows Kits\10\bin",
        "${env:ProgramFiles}\Windows Kits\10\bin"
    ) | Where-Object { $_ -and (Test-Path $_) }

    $candidates = foreach ($root in $roots) {
        Get-ChildItem -Path $root -Recurse -Filter 'signtool.exe' -ErrorAction SilentlyContinue |
            Where-Object { $_.FullName -match '\\x64\\' }
    }

    $tool = $candidates | Sort-Object -Property FullName -Descending | Select-Object -First 1
    if (-not $tool) {
        throw 'signtool.exe not found. Install the Windows SDK signing tools on this runner.'
    }
    return $tool.FullName
}

$files = @()
foreach ($pattern in $Path) {
    $resolved = Get-ChildItem -Path $pattern -File -ErrorAction SilentlyContinue
    if (-not $resolved) {
        throw "nothing to sign at $pattern"
    }
    $files += $resolved.FullName
}

$signtool = Find-SignTool
Write-Host "==> signtool: $signtool"

$pfxPath = $null
try {
    if ($env:AZURE_SIGNING_ENDPOINT) {
        Write-Host '==> signing with Azure Trusted Signing'

        # The dlib is the bridge between signtool and the key that never leaves
        # Azure. It is installed by the workflow step above this one.
        $dlib = Join-Path $env:AZURE_CODE_SIGNING_DLIB 'bin\x64\Azure.CodeSigning.Dlib.dll'
        if (-not (Test-Path $dlib)) {
            throw "Azure.CodeSigning.Dlib.dll not found at $dlib"
        }

        $metadata = Join-Path ([System.IO.Path]::GetTempPath()) 'silka-trusted-signing.json'
        @{
            Endpoint               = $env:AZURE_SIGNING_ENDPOINT
            CodeSigningAccountName = $env:AZURE_SIGNING_ACCOUNT
            CertificateProfileName = $env:AZURE_SIGNING_PROFILE
        } | ConvertTo-Json | Set-Content -Path $metadata -Encoding utf8

        foreach ($file in $files) {
            Write-Host "==> signing $file"
            & $signtool sign `
                /v /fd SHA256 `
                /tr $TimestampUrl /td SHA256 `
                /dlib $dlib /dmdf $metadata `
                $file
            if ($LASTEXITCODE -ne 0) { throw "signtool failed on $file" }
        }
    }
    elseif ($env:WINDOWS_CERT_PFX) {
        Write-Host '==> signing with a .pfx from the environment'
        if (-not $env:WINDOWS_CERT_PASSWORD) {
            throw 'WINDOWS_CERT_PASSWORD must be set alongside WINDOWS_CERT_PFX'
        }

        $pfxPath = Join-Path ([System.IO.Path]::GetTempPath()) 'silka-signing.pfx'
        [System.IO.File]::WriteAllBytes(
            $pfxPath,
            [System.Convert]::FromBase64String($env:WINDOWS_CERT_PFX)
        )

        foreach ($file in $files) {
            Write-Host "==> signing $file"
            & $signtool sign `
                /v /fd SHA256 `
                /tr $TimestampUrl /td SHA256 `
                /f $pfxPath /p $env:WINDOWS_CERT_PASSWORD `
                $file
            if ($LASTEXITCODE -ne 0) { throw "signtool failed on $file" }
        }
    }
    else {
        throw 'no signing credentials: set AZURE_SIGNING_ENDPOINT (+ account, profile) or WINDOWS_CERT_PFX (+ password)'
    }
}
finally {
    # The .pfx exists for the length of one signing run and no longer. A runner
    # that dies mid-run leaves nothing behind because the temp directory goes
    # with the job.
    if ($pfxPath -and (Test-Path $pfxPath)) {
        Remove-Item -Path $pfxPath -Force -ErrorAction SilentlyContinue
    }
}

foreach ($file in $files) {
    Write-Host "==> verifying $file"
    # `/pa` uses the Authenticode policy — the one Windows itself applies. The
    # default policy is the driver policy, which passes files SmartScreen will
    # still block.
    & $signtool verify /pa /v $file
    if ($LASTEXITCODE -ne 0) { throw "signature did not verify on $file" }
}

Write-Host "==> signed and verified $($files.Count) file(s)"
