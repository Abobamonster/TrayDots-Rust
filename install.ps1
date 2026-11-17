param([string]$Executable)
$ErrorActionPreference = 'Stop'
if (!$Executable) {
    $Executable = Join-Path $PSScriptRoot 'rust-dots.exe'
    if (!(Test-Path -LiteralPath $Executable)) {
        $Executable = Join-Path $PSScriptRoot 'target\release\rust-dots.exe'
    }
}
$source = (Resolve-Path -LiteralPath $Executable).Path
$directory = Join-Path $env:LOCALAPPDATA 'TrayDotsRust'
$installed = Join-Path $directory 'RustDots.exe'
$previous = Join-Path $directory 'tray-dots-rust.exe'
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

foreach ($path in (@($legacy, $previous, $installed, $source) | Select-Object -Unique)) {
    Stop-Instance $path
}
if (Get-Process -Name 'RustDots','rust-dots','tray-dots-rust' -ErrorAction SilentlyContinue) {
    throw 'Another RustDots copy is running. Close it before installing.'
}
New-Item -ItemType Directory -Path $directory -Force | Out-Null
if ($source -ne $installed) {
    Copy-Item -LiteralPath $source -Destination $installed -Force
}
$main = Start-Process -FilePath $installed -ArgumentList '--background' -WindowStyle Hidden -PassThru
Start-Sleep -Seconds 3
$main.Refresh()
if ($main.HasExited) { throw "RustDots exited during startup ($($main.ExitCode))." }
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
$programs = [Environment]::GetFolderPath('Programs')
$link = $shell.CreateShortcut((Join-Path $programs 'RustDots.lnk'))
$link.TargetPath = $installed
$link.WorkingDirectory = $directory
$link.Arguments = ''
$link.Save()
$previousLink = Join-Path $programs 'TrayDots\TrayDots.lnk'
if ((Test-Path -LiteralPath $previousLink) -and $shell.CreateShortcut($previousLink).TargetPath -eq $previous) {
    Remove-Item -LiteralPath $previousLink
}
Write-Output "Installed and started: $installed"
