param([string]$Executable)
$ErrorActionPreference = 'Stop'
if (!$Executable) {
    $Executable = Join-Path $PSScriptRoot 'tray-dots-rust.exe'
    if (!(Test-Path -LiteralPath $Executable)) {
        $Executable = Join-Path $PSScriptRoot 'target\release\tray-dots-rust.exe'
    }
}
$source = (Resolve-Path -LiteralPath $Executable).Path
$directory = Join-Path $env:LOCALAPPDATA 'TrayDotsRust'
$installed = Join-Path $directory 'tray-dots-rust.exe'
$legacy = Join-Path $env:LOCALAPPDATA 'TrayDots\TrayDots.exe'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'

function Stop-Instance([string]$Path) {
    $name = [IO.Path]::GetFileNameWithoutExtension($Path)
    $active = @(Get-Process -Name $name -ErrorAction SilentlyContinue | Where-Object Path -eq $Path)
    if (!$active) { return }
    Start-Process -FilePath $Path -ArgumentList '--quit' -WindowStyle Hidden -Wait
    foreach ($process in $active) {
        if (!$process.WaitForExit(8000)) { throw "Process $($process.Id) did not exit." }
    }
}

Stop-Instance $legacy
Stop-Instance $installed
if ($source -ne $installed) { Stop-Instance $source }
if (Get-Process -Name 'tray-dots-rust' -ErrorAction SilentlyContinue) {
    throw 'Another TrayDots Rust copy is running. Close it before installing.'
}
New-Item -ItemType Directory -Path $directory -Force | Out-Null
if ($source -ne $installed) {
    Copy-Item -LiteralPath $source -Destination $installed -Force
}
$main = Start-Process -FilePath $installed -ArgumentList '--background' -WindowStyle Hidden -PassThru
Start-Sleep -Seconds 3
$main.Refresh()
if ($main.HasExited) { throw "TrayDots Rust exited during startup ($($main.ExitCode))." }
$startup = Start-Process -FilePath $installed -ArgumentList '--startup-on' -WindowStyle Hidden -Wait -PassThru
if ($startup.ExitCode -ne 0) { throw 'Could not set autostart.' }

# Retire only the Run value owned by our previous installation. Keep its
# executable and the old approval state intact.
$legacyRun = (Get-Item -LiteralPath $runKey).GetValue('TrayDots')
if ($legacyRun -eq ('"' + $legacy + '"')) {
    $backup = Join-Path $directory 'legacy-autostart.txt'
    if (!(Test-Path -LiteralPath $backup)) { Set-Content -LiteralPath $backup -Value $legacyRun -Encoding utf8 }
    Remove-ItemProperty -LiteralPath $runKey -Name TrayDots
}

$shell = New-Object -ComObject WScript.Shell
$desktop = [Environment]::GetFolderPath('Desktop')
$programs = [Environment]::GetFolderPath('Programs')
$folder = Join-Path $programs 'TrayDots'
New-Item -ItemType Directory -Path $folder -Force | Out-Null
$desktopLink = Join-Path $desktop 'TrayDots.lnk'
if (Test-Path -LiteralPath $desktopLink) {
    $existing = $shell.CreateShortcut($desktopLink)
    if ($existing.TargetPath -notin @($legacy, $installed)) {
        $desktopLink = Join-Path $desktop 'TrayDots Rust.lnk'
    }
}
$links = @(
    @{ Path = $desktopLink; Arguments = '' },
    @{ Path = (Join-Path $folder 'TrayDots.lnk'); Arguments = '' },
    @{ Path = (Join-Path $folder 'Остановить и убрать автозапуск.lnk'); Arguments = '--uninstall' }
)
foreach ($entry in $links) {
    $link = $shell.CreateShortcut($entry.Path)
    $link.TargetPath = $installed
    $link.WorkingDirectory = $directory
    $link.Arguments = $entry.Arguments
    $link.Save()
}
Write-Output "Installed and started: $installed"
