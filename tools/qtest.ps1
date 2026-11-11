# Debug helper: boot the ISO, type a shell command, save a screendump.
param(
    [string]$Keys = "l s spc d i s k ret",
    [string]$Shot = "build/shot_test.ppm",
    [int]$BootWait = 8,
    [int]$WaitAfter = 2,
    [int]$Port = 4555,
    [string]$Qemu = '',        # override QEMU binary path
    [string]$Disk = ''         # override FAT32 image path (skipped when missing)
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

if (!$Qemu) {
    $candidates = @()
    if ($env:QEMU) { $candidates += $env:QEMU }
    $candidates += 'C:\Program Files\QEMU\qemu-system-x86_64.exe'
    $candidates += 'C:\Program Files (x86)\QEMU\qemu-system-x86_64.exe'
    $Qemu = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (!$Qemu) { $Qemu = 'qemu-system-x86_64' }
}
if (!$Disk) {
    $Disk = Join-Path $root 'disk.img'
}

Push-Location $root
try {
    if (Test-Path build/serial.log) { Remove-Item build/serial.log }
    $args = @(
        '-cdrom', 'build/atrodrax.iso',
        '-m', '512M',
        '-display', 'none',
        '-no-reboot',
        '-nic', 'user,model=e1000',
        '-boot', 'order=d',
        '-serial', 'file:build/serial.log',
        '-monitor', "tcp:127.0.0.1:$Port,server,nowait"
    )
    if (Test-Path $Disk) {
        $args += @(
            '-drive', "file=$((Resolve-Path $Disk).Path),format=raw,if=none,id=disk0",
            '-device', 'virtio-blk-pci,drive=disk0,disable-legacy=off,disable-modern=on'
        )
    } else {
        Write-Warning "disk image not found ($Disk) - booting without virtio-blk"
    }
    $proc = Start-Process -FilePath $Qemu -ArgumentList $args -PassThru -WindowStyle Hidden
    Start-Sleep -Seconds $BootWait

    $client = New-Object System.Net.Sockets.TcpClient('127.0.0.1', $Port)
    $stream = $client.GetStream()
    $writer = New-Object System.IO.StreamWriter($stream)
    $writer.AutoFlush = $true
    Start-Sleep -Milliseconds 300
    foreach ($key in $Keys.Split(' ')) {
        $writer.WriteLine("sendkey $key")
        Start-Sleep -Milliseconds 120
    }
    Start-Sleep -Seconds $WaitAfter
    $writer.WriteLine("screendump $Shot")
    Start-Sleep -Seconds 1
    $writer.WriteLine('quit')
    Start-Sleep -Milliseconds 500
    $client.Close()
    if (!$proc.HasExited) { $proc.WaitForExit(5000) | Out-Null }
    if (!$proc.HasExited) { $proc.Kill() }
} finally {
    Pop-Location
}
