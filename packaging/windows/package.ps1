[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidateSet('x86_64', 'aarch64')]
    [string]$Arch,
    [Parameter(Mandatory)]
    [string]$BinaryDir,
    [Parameter(Mandatory)]
    [string]$Wix,
    [string]$OutputDirectory = 'dist'
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$BinaryDir = (Resolve-Path $BinaryDir).Path
$Wix = (Resolve-Path $Wix).Path
$output = (New-Item -ItemType Directory -Force -Path $OutputDirectory).FullName

if ($Arch -eq 'aarch64') {
    $machine = 0xaa64
    $wixArch = 'arm64'
    $upgradeCode = 'A5EF34B4-1A58-4E25-A730-10A93F12774E'
} else {
    $machine = 0x8664
    $wixArch = 'x64'
    $upgradeCode = '57568D70-146F-4CBD-96F8-255A6FF3B14D'
}

$binaries = foreach ($name in 'rynthv.exe', 'rynthv-synth.exe') {
    $path = Join-Path $BinaryDir $name
    $reader = [IO.BinaryReader]::new([IO.File]::OpenRead($path))
    try {
        if ($reader.ReadUInt16() -ne 0x5a4d) { throw "$name has no DOS header" }
        $reader.BaseStream.Position = 0x3c
        $reader.BaseStream.Position = $reader.ReadUInt32()
        if ($reader.ReadUInt32() -ne 0x00004550 -or $reader.ReadUInt16() -ne $machine) {
            throw "$name is not a native $Arch PE image"
        }
    } finally {
        $reader.Dispose()
    }
    $path
}

$artifact = Join-Path $output "rynthv-windows-$Arch"
$documents = (Join-Path $root 'README.md'), (Join-Path $root 'LICENSE')
Compress-Archive -LiteralPath ($binaries + $documents) -DestinationPath "$artifact.zip" -CompressionLevel Optimal -Force

$manifest = Get-Content (Join-Path $root 'Cargo.toml') -Raw
$version = [regex]::Match($manifest, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
if (-not $version) { throw 'Could not read the package version from Cargo.toml' }

$msi = "$artifact.msi"
& $Wix build "$PSScriptRoot/RynthV.wxs" -arch $wixArch -d "ProductVersion=$version" `
    -d "ProductUpgradeCode=$upgradeCode" -d "BinaryDir=$BinaryDir" -pdbtype none -o $msi
if ($LASTEXITCODE -ne 0) { throw 'WiX MSI packaging failed' }

$installer = New-Object -ComObject WindowsInstaller.Installer
$summary = $installer.GetType().InvokeMember('SummaryInformation', 'GetProperty', $null, $installer, @($msi, 0))
$template = $summary.GetType().InvokeMember('Property', 'GetProperty', $null, $summary, @(7))
if (($template -split ';', 2)[0] -ne $wixArch) { throw "Unexpected MSI template '$template' in $msi" }
