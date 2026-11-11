# Boot the AtrodraX Rust ISO in QEMU (Windows / PowerShell).
#
#   .\run_qemu.ps1                 # graphical window
#   .\run_qemu.ps1 -Headless       # no display, serial output in this console
#
# Extra arguments after -- are passed through to QEMU.

param(
    [string]$Iso = '',
    [string]$Disk = '',
    [string]$Memory = '512M',
    [string]$Qemu = '',
    [switch]$Headless,
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$Extra = @()
)

$ErrorActionPreference = 'Stop'

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not $Iso) { $Iso = Join-Path $scriptDir 'build\atrodrax.iso' }
if (-not $Disk) { $Disk = Join-Path $scriptDir 'disk.img' }

if (-not $Qemu) {
    $Qemu = (Get-Command qemu-system-x86_64 -ErrorAction SilentlyContinue).Source
    if (-not $Qemu) {
        $candidate = 'C:\Program Files\QEMU\qemu-system-x86_64.exe'
        if (Test-Path $candidate) { $Qemu = $candidate }
    }
}
if (-not $Qemu -or -not (Test-Path $Qemu)) {
    throw 'qemu-system-x86_64 not found; install QEMU or pass -Qemu <path>'
}
if (-not (Test-Path $Iso)) {
    throw "$Iso not found; run .\build.sh (in WSL) first"
}

$qemuArgs = @(
    '-cdrom', $Iso,
    '-m', $Memory,
    '-no-reboot',
    '-boot', 'order=d',
    '-nic', 'user,model=e1000'
)

if (Test-Path $Disk) {
    $Disk = (Resolve-Path $Disk).Path
    $qemuArgs += @(
        '-drive', "file=$Disk,format=raw,if=none,id=disk0",
        '-device', 'virtio-blk-pci,drive=disk0,disable-legacy=off,disable-modern=on'
    )
} else {
    Write-Host "note: $Disk not found; booting without a persistent disk"
}

if ($Headless) {
    $qemuArgs += @('-display', 'none', '-serial', 'mon:stdio')
} else {
    $qemuArgs += @('-serial', 'mon:stdio')
}
$qemuArgs += $Extra

Write-Host "booting $Iso with $Qemu (memory $Memory, headless=$($Headless.IsPresent))"
& $Qemu @qemuArgs
