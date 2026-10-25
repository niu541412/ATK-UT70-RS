$ErrorActionPreference = 'Stop'
if ([Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne [Runtime.InteropServices.Architecture]::X64) {
    throw 'This script requires Windows x64.'
}

$repo = Split-Path $PSScriptRoot -Parent
$cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (-not (Test-Path -LiteralPath $cargo)) { throw 'cargo.exe not found.' }
if (-not (Test-Path -LiteralPath $vswhere)) { throw 'Visual Studio Build Tools not found.' }

$installation = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $installation) { throw 'MSVC C++ toolchain not found.' }
$vcvars = Join-Path $installation 'VC\Auxiliary\Build\vcvars64.bat'
if (-not (Test-Path -LiteralPath $vcvars)) { throw 'vcvars64.bat not found.' }

Push-Location $repo
try {
    $build = 'call "' + $vcvars + '" && "' + $cargo + '" build --release --locked'
    & cmd.exe /c $build
    if ($LASTEXITCODE -ne 0) { throw "Build failed with exit code $LASTEXITCODE." }

    $source = Join-Path $repo 'target\release\atk-ut70-rs.exe'
    $destination = [IO.Path]::GetFullPath((Join-Path $repo '..\ATK-UT70-RS-windows-x86_64.exe'))
    Copy-Item -LiteralPath $source -Destination $destination -Force
    Write-Output $destination
}
finally {
    Pop-Location
}
