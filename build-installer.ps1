param([string]$Makensis = 'makensis.exe')

$ErrorActionPreference = 'Stop'
$compiler = (Get-Command $Makensis -CommandType Application).Source
Push-Location -LiteralPath $PSScriptRoot
try {
    $metadata = cargo metadata --offline --locked --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) { throw 'Could not read the Cargo package version.' }
    $version = ($metadata | ConvertFrom-Json).packages[0].version
    cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'Could not build RustDots.' }

    & $compiler /V3 /WX /INPUTCHARSET UTF8 "/DAPP_VERSION=$version" (Join-Path $PSScriptRoot 'installer\RustDots.nsi')
    if ($LASTEXITCODE -ne 0) { throw 'Could not build the Windows installer.' }
} finally {
    Pop-Location
}
