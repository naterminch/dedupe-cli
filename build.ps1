#!/usr/bin/env pwsh
# Builds dedupe and copies the resulting binary to the project root
# (the folder that contains this script), next to Cargo.toml:
#
#   .\build.ps1           release build -> .\dedupe.exe  (at project root)
#   .\build.ps1 -Debug    debug build   -> .\dedupe.exe  (same place)
#
# Nothing is written outside the project directory: the stderr log lives in
# ./target and the binary is copied to the project root.
param(
    [switch]$Debug
)

# Project root = the directory containing this script. Everything below is
# anchored to this path; we never write to $HOME or the user profile.
$root = $PSScriptRoot
if (-not $root) {
    throw "could not determine the script directory (project root)"
}
$profile = if ($Debug) { "debug" } else { "release" }

# Resolve cargo: trust PATH first, fall back to the standard rustup location
# (covers shells started before rustup was installed / added to PATH).
$cargoPath = $null
$cargoCmd = Get-Command cargo -ErrorAction SilentlyContinue
if ($cargoCmd) {
    $cargoPath = $cargoCmd.Source
} else {
    $rustupCargo = Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"
    if (Test-Path -LiteralPath $rustupCargo) {
        $cargoPath = $rustupCargo
    }
}
if (-not $cargoPath) {
    throw "cargo not found on PATH (install via https://rustup.rs)"
}

Push-Location $root
try {
    # Redirect cargo's stderr to a log inside the project. cargo prints
    # progress on stderr, which PS 5.1 surfaces as noisy error records; the
    # log is printed only if the build actually fails. Real failures are
    # caught via $LASTEXITCODE.
    $errLog = Join-Path $root "target/build-stderr.log"
    if ($Debug) {
        & $cargoPath build 2> $errLog
    } else {
        & $cargoPath build --release 2> $errLog
    }
    if ($LASTEXITCODE -ne 0) {
        if (Test-Path -LiteralPath $errLog) { Get-Content $errLog }
        throw "cargo build failed with exit code $LASTEXITCODE"
    }
    Remove-Item -LiteralPath $errLog -ErrorAction SilentlyContinue

    # Copy the built binary to the project root (never the user profile root).
    $source = Join-Path $root "target/$profile/dedupe.exe"
    $dest = Join-Path $root "dedupe.exe"
    Copy-Item -LiteralPath $source -Destination $dest -Force

    $rootFull = [System.IO.Path]::GetFullPath($root)
    Write-Host "copied  $source"
    Write-Host "    to  $dest  (project root: $rootFull)"
}
finally {
    Pop-Location
}