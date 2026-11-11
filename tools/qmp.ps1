# Debug helper: boot the ISO and inject mouse/keyboard input via QMP.
# Coordinates are screen pixels; the script converts them to relative deltas
# from the kernel's initial cursor position (centre of the 1024x768 screen).
param(
    [string]$Clicks = "",           # semicolon-separated, e.g. "48,151;300,280"
    [string]$Drag = "",             # "x1,y1,x2,y2" - press at 1, move to 2, release
    [string]$Keys = "",             # space-separated HMP sendkey names
    [int]$BootWait = 8,
    [string]$Shot = "",
    [int]$Port = 4557,
    [string]$Qemu = '',             # override QEMU binary path
    [string]$Disk = ''              # override FAT32 image path (skipped when missing)
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
        '-qmp', "tcp:127.0.0.1:$Port,server,nowait"
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
    $reader = New-Object System.IO.StreamReader($stream)
    $writer = New-Object System.IO.StreamWriter($stream)
    $writer.AutoFlush = $true
    $null = $reader.ReadLine()          # QMP greeting
    $writer.WriteLine('{"execute":"qmp_capabilities"}')
    $null = $reader.ReadLine()

    $cursorX = 513
    $cursorY = 387

    function Send-Events([object[]]$events) {
        $payload = @{ execute = 'input-send-event'; arguments = @{ events = $events } } |
            ConvertTo-Json -Depth 8 -Compress
        $writer.WriteLine($payload)
        $reply = $reader.ReadLine()
        if ($reply -and $reply -notmatch '"return"') { Write-Host "QMP: $reply" }
    }

    function Rel-Axis([string]$axis, [int]$delta) {
        while ($delta -ne 0) {
            $step = [math]::Sign($delta) * [math]::Min(32, [math]::Abs($delta))
            Send-Events @(@{ type = 'rel'; data = @{ axis = $axis; value = $step } })
            $delta -= $step
            Start-Sleep -Milliseconds 40
        }
    }

    function Flush-Mouse {
        Send-Events @(@{ type = 'rel'; data = @{ axis = 'x'; value = 1 } })
        Start-Sleep -Milliseconds 40
        Send-Events @(@{ type = 'rel'; data = @{ axis = 'x'; value = -1 } })
        Start-Sleep -Milliseconds 40
        Send-Events @(@{ type = 'rel'; data = @{ axis = 'y'; value = 1 } })
        Start-Sleep -Milliseconds 40
        Send-Events @(@{ type = 'rel'; data = @{ axis = 'y'; value = -1 } })
        Start-Sleep -Milliseconds 60
    }

    function Move-Cursor([int]$x, [int]$y) {
        $dx = $x - $script:cursorX
        $dy = $y - $script:cursorY
        Write-Host "move ($script:cursorX,$script:cursorY) -> ($x,$y)"
        Rel-Axis 'x' $dx
        Rel-Axis 'y' $dy
        Start-Sleep -Milliseconds 250
        $script:cursorX = $x
        $script:cursorY = $y
    }

    function Press-Button([bool]$down) {
        Send-Events @(@{ type = 'btn'; data = @{ down = $down; button = 'left' } })
        Start-Sleep -Milliseconds 100
    }

    foreach ($spec in $Clicks.Split(';')) {
        if (-not $spec) { continue }
        Flush-Mouse
        $parts = $spec.Split(',')
        $x = [int]$parts[0]
        $y = [int]$parts[1]
        $double = $parts.Count -gt 2 -and $parts[2] -eq 'double'
        Move-Cursor $x $y
        Press-Button $true
        Press-Button $false
        if ($double) {
            Start-Sleep -Milliseconds 120
            Press-Button $true
            Press-Button $false
        }
        Start-Sleep -Milliseconds 400
    }

    if ($Drag) {
        $parts = $Drag.Split(',')
        $x1 = [int]$parts[0]; $y1 = [int]$parts[1]
        $x2 = [int]$parts[2]; $y2 = [int]$parts[3]
        Flush-Mouse
        Move-Cursor $x1 $y1
        Press-Button $true
        Start-Sleep -Milliseconds 200
        Move-Cursor $x2 $y2
        Press-Button $false
        Start-Sleep -Milliseconds 400
    }

    if ($Keys) {
        foreach ($key in $Keys.Split(' ')) {
            $payload = @{ execute = 'human-monitor-command'; arguments = @{ 'command-line' = "sendkey $key" } } |
                ConvertTo-Json -Depth 4 -Compress
            $writer.WriteLine($payload)
            $null = $reader.ReadLine()
            Start-Sleep -Milliseconds 130
        }
        Start-Sleep -Seconds 1
    }

    if ($Shot) {
        $writer.WriteLine('{"execute":"screendump","arguments":{"filename":"' + $Shot + '"}}')
        $null = $reader.ReadLine()
        Start-Sleep -Seconds 1
    }
    $writer.WriteLine('{"execute":"quit"}')
    Start-Sleep -Milliseconds 500
    $client.Close()
    if (!$proc.HasExited) { $proc.WaitForExit(5000) | Out-Null }
    if (!$proc.HasExited) { $proc.Kill() }
} finally {
    Pop-Location
}
