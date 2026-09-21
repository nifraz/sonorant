<#
.SYNOPSIS
    Builds the Windows download: sonorant.exe and its documents in one zip.

.DESCRIPTION
    Sonorant needs no installer. It writes its settings to %APPDATA%\Sonorant and
    nothing else, so the download is the program, its licence and its readme in a zip
    that can be unpacked anywhere. The release workflow runs this on windows-latest and
    attaches the zip and its checksum to the release; winget points at the same file.

    Signing is optional and off by default, because the project has no code-signing
    certificate yet. Without one SmartScreen warns the first few hundred people who run
    it. Pass -SignWith to sign, on a machine that has signtool.exe on PATH.

.EXAMPLE
    packaging\windows\build.ps1
    packaging\windows\build.ps1 -NoBuild -Out C:\tmp
    packaging\windows\build.ps1 -SignWith cert.pfx -SignPassword $env:CERT_PASSWORD
#>
[CmdletBinding()]
param(
    # Defaults to the version in Cargo.toml, which the release workflow stamps from the tag.
    [string]$Version,
    [string]$Out,
    [switch]$NoBuild,
    [string]$SignWith,
    [string]$SignPassword
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
if (-not $Out) { $Out = Join-Path $root 'target\zip' }

if (-not $Version) {
    $manifest = Get-Content (Join-Path $root 'Cargo.toml') -Raw
    $match = [regex]::Match($manifest, '(?ms)^\[workspace\.package\].*?^version = "([^"]+)"')
    if (-not $match.Success) { throw 'cannot read the version from Cargo.toml' }
    $Version = $match.Groups[1].Value
}

if (-not $NoBuild) {
    & cargo build --release --locked -p sonorant
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }
}

$exe = Join-Path $root 'target\release\sonorant.exe'
if (-not (Test-Path $exe)) { throw "no binary at $exe" }

if ($SignWith) {
    # Not $args: that name belongs to PowerShell itself inside an advanced script.
    $signArgs = @('sign', '/fd', 'SHA256', '/f', $SignWith,
                  '/tr', 'http://timestamp.digicert.com', '/td', 'SHA256')
    if ($SignPassword) { $signArgs += @('/p', $SignPassword) }
    & signtool.exe @signArgs $exe
    if ($LASTEXITCODE -ne 0) { throw "signtool failed ($LASTEXITCODE)" }
}

$name = "sonorant-$Version-windows-x64"
$stage = Join-Path ([System.IO.Path]::GetTempPath()) $name
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item $stage -ItemType Directory | Out-Null

Copy-Item $exe (Join-Path $stage 'sonorant.exe')
Copy-Item (Join-Path $root 'README.md') $stage
Copy-Item (Join-Path $root 'LICENSE') (Join-Path $stage 'LICENSE.txt')
# The icon, so a shortcut to the unpacked program has one: an exe with no resource
# section carries no icon of its own, and building one in would need a resource
# compiler that the GNU toolchain route hasn't got.
Copy-Item (Join-Path $root 'packaging\icons\sonorant.ico') $stage

New-Item $Out -ItemType Directory -Force | Out-Null
$zip = Join-Path $Out "$name.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip
Remove-Item $stage -Recurse -Force

# The checksum the release notes and the winget manifest both quote.
$hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLowerInvariant()
"$hash  $name.zip" | Set-Content -Path "$zip.sha256" -Encoding ascii -NoNewline

Write-Output $zip
Write-Output $hash
