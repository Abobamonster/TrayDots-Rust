param(
    [string]$Executable = "$PSScriptRoot\target\release\rust-dots.exe",
    [int]$IdleSeconds = 120,
    [int]$Cycles = 60,
    [string]$Report = "$PSScriptRoot\verification.json",
    [switch]$LeaveRunning
)
$ErrorActionPreference = 'Stop'
$Executable = (Resolve-Path -LiteralPath $Executable).Path
$ProcessName = [IO.Path]::GetFileNameWithoutExtension($Executable)
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class TrayDotsProbe {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int L,T,R,B; }
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X,Y; }
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string c, string n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string c, string n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr GetProp(IntPtr w, string name);
    public static IntPtr Top(string c) { return FindWindow(c, null); }
    public static IntPtr Child(IntPtr parent, string c) { return FindWindowEx(parent, IntPtr.Zero, c, null); }
    public static bool HasTrayDotsTaskbarAccent() {
        if (GetProp(Top("Shell_TrayWnd"), "TrayDotsRust.TaskbarAccent.1") != IntPtr.Zero) return true;
        IntPtr secondary = IntPtr.Zero;
        while ((secondary = FindWindowEx(IntPtr.Zero, secondary, "Shell_SecondaryTrayWnd", null)) != IntPtr.Zero) {
            if (GetProp(secondary, "TrayDotsRust.TaskbarAccent.1") != IntPtr.Zero) return true;
        }
        return false;
    }
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr w);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr w, out Rect r);
    [DllImport("user32.dll")] public static extern bool GetLayeredWindowAttributes(IntPtr w, out uint key, out byte alpha, out uint flags);
    public static bool IsMasked(IntPtr w) {
        uint key, flags; byte alpha;
        return GetLayeredWindowAttributes(w, out key, out alpha, out flags) && (flags & 2) != 0 && alpha == 1;
    }
    [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point p);
    [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr w, IntPtr after, int x, int y, int cx, int cy, uint flags);
    public static bool RaiseWindow(IntPtr w) {
        const uint NoSize = 0x0001, NoMove = 0x0002, NoActivate = 0x0010;
        return SetWindowPos(w, IntPtr.Zero, 0, 0, 0, 0, NoSize | NoMove | NoActivate);
    }
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr w, uint m, IntPtr wp, IntPtr lp);
    public static bool OpenWithoutOpaqueFrame(IntPtr button, IntPtr popup) {
        if (!PostMessage(button, 0x0202, IntPtr.Zero, IntPtr.Zero)) return false;
        var watch = System.Diagnostics.Stopwatch.StartNew();
        while (watch.ElapsedMilliseconds < 5000) {
            if (IsWindowVisible(popup) && !IsMasked(popup)) return false;
            if (IsWindowVisible(Top("TrayDotsRust.OverflowGlass.1"))) return true;
            System.Threading.Thread.Yield();
        }
        return false;
    }
    [DllImport("user32.dll")] public static extern IntPtr SendMessageTimeout(IntPtr w, uint m, IntPtr wp, IntPtr lp, uint flags, uint ms, out IntPtr result);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr w, out uint pid);
    public static int Count(IntPtr w) {
        if (w == IntPtr.Zero) return 0;
        IntPtr result;
        if (SendMessageTimeout(w, 0x418, IntPtr.Zero, IntPtr.Zero, 2, 400, out result) == IntPtr.Zero) return -1;
        return result.ToInt32();
    }
}
'@
function Assert-True([bool]$Value, [string]$Message) { if (!$Value) { throw $Message } }
function Wait-Until([scriptblock]$Condition, [string]$Message) {
    $until = [DateTime]::UtcNow.AddSeconds(5)
    do {
        if (& $Condition) { return }
        Start-Sleep -Milliseconds 30
    } while ([DateTime]::UtcNow -lt $until)
    throw $Message
}
function Get-AutoTray {
    Get-ItemPropertyValue 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer' -Name EnableAutoTray -ErrorAction SilentlyContinue
}
function Get-Samples {
    $procs = Get-Process -Name $ProcessName | Where-Object Path -eq $Executable
    Assert-True (@($procs).Count -eq 2) 'Expected main and recovery processes.'
    $rows = @($procs | ForEach-Object {
        [ordered]@{
            id = $_.Id
            private_bytes = $_.PrivateMemorySize64
            working_set_bytes = $_.WorkingSet64
            cpu_ms = [math]::Round($_.TotalProcessorTime.TotalMilliseconds, 3)
            handles = $_.HandleCount
        }
    })
    [ordered]@{
        at_utc = [DateTime]::UtcNow.ToString('o')
        private_bytes = ($rows.private_bytes | Measure-Object -Sum).Sum
        working_set_bytes = ($rows.working_set_bytes | Measure-Object -Sum).Sum
        cpu_ms = ($rows.cpu_ms | Measure-Object -Sum).Sum
        handles = ($rows.handles | Measure-Object -Sum).Sum
        processes = $rows
    }
}
function Is-OverflowVisible {
    [TrayDotsProbe]::IsWindowVisible([TrayDotsProbe]::Top('NotifyIconOverflowWindow'))
}
function Is-GlassVisible {
    [TrayDotsProbe]::IsWindowVisible([TrayDotsProbe]::Top('TrayDotsRust.OverflowGlass.1'))
}
function Start-Tray {
    $proc = Start-Process -FilePath $Executable -ArgumentList '--background' -WindowStyle Hidden -PassThru
    Wait-Until { (Get-AutoTray) -eq 1 -and [TrayDotsProbe]::IsWindowVisible((Find-Button)) } 'Tray button did not appear.'
    $proc
}
function Find-Button {
    [TrayDotsProbe]::Child($trayWnd, 'TrayDotsRust.Ellipsis.1')
}
$root = [TrayDotsProbe]::Top('TrayDotsRust.Controller.1')
Assert-True ($root -eq [IntPtr]::Zero) 'Stop RustDots before running verification.'
$previous = Get-AutoTray
$shellWnd = [TrayDotsProbe]::Top('Shell_TrayWnd')
$trayWnd = [TrayDotsProbe]::Child($shellWnd, 'TrayNotifyWnd')
$pager = [TrayDotsProbe]::Child($trayWnd, 'SysPager')
$mainToolbar = [TrayDotsProbe]::Child($pager, 'ToolbarWindow32')
$beforeCount = [TrayDotsProbe]::Count($mainToolbar)
$restoreFile = Join-Path $env:LOCALAPPDATA 'TrayDotsRust\restore-v1.txt'
$samples = [Collections.Generic.List[object]]::new()
try {
    $proc = Start-Tray
    $button = Find-Button
    $rect = [TrayDotsProbe+Rect]::new()
    [void][TrayDotsProbe]::GetWindowRect($button, [ref]$rect)
    $center = [TrayDotsProbe+Point]::new()
    $center.X = [int](($rect.L + $rect.R)/2)
    $center.Y = [int](($rect.T + $rect.B)/2)
    Assert-True ([TrayDotsProbe]::WindowFromPoint($center) -eq $button) 'Ellipsis is not the click target.'
    $arrow = [TrayDotsProbe]::Child($trayWnd, 'Button')
    Assert-True ([TrayDotsProbe]::RaiseWindow($arrow)) 'Could not reproduce the native arrow covering the ellipsis.'
    Wait-Until { [TrayDotsProbe]::WindowFromPoint($center) -eq $button } 'The ellipsis did not regain the click target when the native arrow was raised.'
    $collapsedCount = [TrayDotsProbe]::Count($mainToolbar)
    $overflow = [TrayDotsProbe]::Top('NotifyIconOverflowWindow')
    $overflowToolbar = [TrayDotsProbe]::Child($overflow, 'ToolbarWindow32')
    $hiddenCount = [TrayDotsProbe]::Count($overflowToolbar)
    Assert-True ($hiddenCount -gt 0) 'No buttons in native overflow.'
    Assert-True (!(Is-OverflowVisible)) 'Overflow must initially be closed.'
    $samples.Add((Get-Samples))
    for ($i=0; $i -lt $Cycles; $i++) {
        # Send mouse-up to our own button, testing its actual window procedure.
        Assert-True ([TrayDotsProbe]::OpenWithoutOpaqueFrame($button, $overflow)) "Transparent opening failed at cycle $i."
        Assert-True ([TrayDotsProbe]::IsMasked($overflow)) 'Native overflow is covering the glass.'
        if ($i -eq 0) {
            $barRect = [TrayDotsProbe+Rect]::new()
            [void][TrayDotsProbe]::GetWindowRect($overflowToolbar, [ref]$barRect)
            $tilePoint = [TrayDotsProbe+Point]::new()
            $tilePoint.X = $barRect.L + 20
            $tilePoint.Y = $barRect.T + 20
            Assert-True ([TrayDotsProbe]::WindowFromPoint($tilePoint) -eq $overflowToolbar) 'The glass intercepted the native toolbar hit target.'
        }
        [void][TrayDotsProbe]::PostMessage($button, 0x0202, [IntPtr]::Zero, [IntPtr]::Zero)
        Wait-Until { !(Is-OverflowVisible) -and !(Is-GlassVisible) -and ![TrayDotsProbe]::IsMasked($overflow) } "Closing or restoring overflow failed at cycle $i."
        if ($i % 10 -eq 9) { $samples.Add((Get-Samples)) }
    }
    $idleStart = Get-Samples
    $samples.Add($idleStart)
    $deadline = [DateTime]::UtcNow.AddSeconds($IdleSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        Start-Sleep -Seconds 3
        $samples.Add((Get-Samples))
    }
    $idleEnd = $samples[$samples.Count-1]
    $cpuDelta = $idleEnd.cpu_ms - $idleStart.cpu_ms
    $privateDelta = $idleEnd.private_bytes - $idleStart.private_bytes
    Assert-True ($privateDelta -lt 2MB) 'Private memory grew by at least 2 MiB while idle.'
    Assert-True (($idleEnd.handles - $idleStart.handles) -lt 10) 'Handle count grew while idle.'
    Start-Process -FilePath $Executable -ArgumentList '--quit' -WindowStyle Hidden -Wait
    Wait-Until { $proc.Refresh(); $proc.HasExited -and (Get-AutoTray) -eq $previous -and !(Test-Path -LiteralPath $restoreFile) -and ![TrayDotsProbe]::HasTrayDotsTaskbarAccent() } 'Graceful exit did not restore tray or taskbar settings.'
    $proc = Start-Tray
    [void][TrayDotsProbe]::PostMessage((Find-Button), 0x0202, [IntPtr]::Zero, [IntPtr]::Zero)
    Wait-Until { (Is-GlassVisible) -and [TrayDotsProbe]::IsMasked($overflow) } 'Glass not ready for the recovery test.'
    Stop-Process -Id $proc.Id -Force
    Wait-Until { (Get-AutoTray) -eq $previous -and !(Test-Path -LiteralPath $restoreFile) -and ![TrayDotsProbe]::IsMasked($overflow) -and ![TrayDotsProbe]::HasTrayDotsTaskbarAccent() } 'Recovery process did not restore tray or taskbar settings.'
    $restoredCount = [TrayDotsProbe]::Count($mainToolbar)
    Assert-True ($restoredCount -eq $beforeCount) 'The initial toolbar button count was not restored.'
    $proc = Start-Tray
    $button = Find-Button
    if (Is-OverflowVisible) {
        [void][TrayDotsProbe]::PostMessage($button, 0x0202, [IntPtr]::Zero, [IntPtr]::Zero)
        Wait-Until { !(Is-OverflowVisible) -and !(Is-GlassVisible) } 'Could not close the overflow left open by the forced-stop test.'
    }
    Assert-True ([TrayDotsProbe]::OpenWithoutOpaqueFrame($button, $overflow)) 'Transparency was lost after recovery and restart.'
    [void][TrayDotsProbe]::PostMessage($button, 0x0202, [IntPtr]::Zero, [IntPtr]::Zero)
    Wait-Until { !(Is-OverflowVisible) -and !(Is-GlassVisible) } 'Could not close the post-recovery overflow.'
    [void][TrayDotsProbe]::PostMessage($arrow, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)
    Wait-Until { (Is-GlassVisible) -and [TrayDotsProbe]::IsMasked($overflow) } 'An overflow opened by Explorer stayed opaque.'
    [void][TrayDotsProbe]::PostMessage($button, 0x0202, [IntPtr]::Zero, [IntPtr]::Zero)
    Wait-Until { !(Is-OverflowVisible) -and !(Is-GlassVisible) } 'Could not close the overflow opened by Explorer.'
    [ordered]@{
        os_build = (Get-CimInstance Win32_OperatingSystem).BuildNumber
        exe_bytes = (Get-Item -LiteralPath $Executable).Length
        cycles = $Cycles
        original_visible = $beforeCount
        collapsed_visible = $collapsedCount
        overflow_buttons = $hiddenCount
        restored_visible = $restoredCount
        idle_seconds = $IdleSeconds
        idle_cpu_ms_both = $cpuDelta
        idle_private_delta_bytes_both = $privateDelta
        peak_private_bytes_both = ($samples.private_bytes | Measure-Object -Maximum).Maximum
        peak_working_set_bytes_both = ($samples.working_set_bytes | Measure-Object -Maximum).Maximum
        normal_exit_restored = $true
        force_stop_restored = $true
        transparent_after_recovery_and_restart = $true
        overflow_opened_by_explorer_decorated = $true
        click_target_restored_after_reorder = $true
        glass_opened = $true
        native_masked_before_first_visible_frame = $true
        native_toolbar_hit_test = $true
        samples = $samples
    } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $Report -Encoding utf8
    Get-Content -LiteralPath $Report -TotalCount 15
} finally {
    $running = [TrayDotsProbe]::Top('TrayDotsRust.Controller.1') -ne [IntPtr]::Zero
    if ($LeaveRunning -and !$running) {
        $null = Start-Tray
    } elseif (!$LeaveRunning -and $running) {
        Start-Process -FilePath $Executable -ArgumentList '--quit' -WindowStyle Hidden -Wait
    }
}
