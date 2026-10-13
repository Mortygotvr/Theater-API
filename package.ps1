# Builds the release zips, dist\theater-api-<name>-<version>-windows-x64.zip,
# one per plugin:
#   theater-api-<name>\bin\64bit   the plugin and its Qt shim (trust_qt.dll)
#   theater-api-<name>\data        its menu icon
#   theater-api-<name>\version.txt the version, which the Theater Installer reads
# People extract them into C:\ProgramData\obs-studio\plugins (or use the installer).
# Build first: cargo build --release (and Theater Scene's Trust\build_trust_qt.ps1)
param(
    [string]$Version = "",
    [string[]]$Only = @('twitch', 'patreon', 'kofi'),
    [string]$QtShim = "$PSScriptRoot\..\theater scene(rust)\Trust\target\release\trust_qt.dll"
)
$ErrorActionPreference = 'Stop'

$root = $PSScriptRoot
if (!$Version) { $Version = (Select-String -LiteralPath "$root\Cargo.toml" -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value }
if (!(Test-Path -LiteralPath $QtShim)) { throw "$QtShim not found - build Theater Scene's Qt shim first." }
Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem

foreach ($name in $Only) {
    $id = "theater-api-$name"
    $dll = "$root\target\release\theater_api_$name.dll"
    if (!(Test-Path -LiteralPath $dll)) { throw "$dll not found - run cargo build --release first." }
    $work = "$root\dist\stage"
    $stage = "$work\$id"
    if (Test-Path $work) { Remove-Item $work -Recurse -Force }
    New-Item -ItemType Directory -Force "$stage\bin\64bit", "$stage\data" | Out-Null
    Copy-Item -LiteralPath $dll "$stage\bin\64bit\$id.dll"
    Copy-Item -LiteralPath $QtShim "$stage\bin\64bit\trust_qt.dll"
    Copy-Item -LiteralPath "$root\assets\$name.png" "$stage\data\icon.png"
    Copy-Item -LiteralPath "$root\LICENSE" "$stage\LICENSE.txt"
    Set-Content -LiteralPath "$stage\version.txt" -Value $Version -Encoding ascii

    $zip = "$root\dist\$id-$Version-windows-x64.zip"
    if (Test-Path $zip) { Remove-Item $zip }
    # Entries named with / between folders, as zip tools expect
    $archive = [System.IO.Compression.ZipFile]::Open($zip, [System.IO.Compression.ZipArchiveMode]::Create)
    try {
        $base = $work.Length + 1
        foreach ($file in Get-ChildItem -LiteralPath $stage -Recurse -File) {
            $entry = $file.FullName.Substring($base).Replace('\', '/')
            [void][System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($archive, $file.FullName, $entry, [System.IO.Compression.CompressionLevel]::Optimal)
        }
    } finally { $archive.Dispose() }
    Remove-Item $work -Recurse -Force
    Write-Host "Made $zip"
}
