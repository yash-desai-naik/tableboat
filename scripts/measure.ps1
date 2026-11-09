# Measures Tableboat's real memory and idle CPU: the app process plus every WebView2
# process it started. "Private" is the private working set (Task Manager's "Memory" column).
#
#   powershell -ExecutionPolicy Bypass -File scripts\measure.ps1 [-Exe path] [-SettleSeconds 8] [-IdleSeconds 30]
#
# With -Exe it launches the app, waits for it to settle, measures, then closes it.
# Without -Exe it measures an already running Tableboat.

param(
    [string]$Exe,
    [int]$SettleSeconds = 8,
    [int]$IdleSeconds = 30
)

$ErrorActionPreference = "Stop"

function Get-Tree([int]$RootId, $AllProcesses) {
    $ids = [System.Collections.Generic.List[int]]::new()
    $ids.Add($RootId)
    $i = 0
    while ($i -lt $ids.Count) {
        $parent = $ids[$i]
        foreach ($p in $AllProcesses) {
            $id = [int]$p.ProcessId
            if ([int]$p.ParentProcessId -eq $parent -and -not $ids.Contains($id)) { $ids.Add($id) }
        }
        $i++
    }
    return $ids
}

function Get-Kind([string]$CommandLine) {
    if (-not $CommandLine) { return "?" }
    if ($CommandLine -notmatch "msedgewebview2") { return "app" }
    if ($CommandLine -match "--utility-sub-type=([\w.]+)") { return "utility:" + ($Matches[1] -replace "^.*\.", "") }
    if ($CommandLine -match "--type=([\w-]+)") { return $Matches[1] }
    return "browser"
}

function Get-CpuMs($Ids) {
    $map = @{}
    foreach ($id in $Ids) {
        $p = Get-Process -Id $id -ErrorAction SilentlyContinue
        if ($p) { $map[$id] = $p.TotalProcessorTime.TotalMilliseconds }
    }
    return $map
}

$started = $null
if ($Exe) {
    $started = Start-Process -FilePath $Exe -PassThru
    Start-Sleep -Seconds $SettleSeconds
    $rootId = $started.Id
} else {
    $proc = Get-Process tableboat -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $proc) { throw "Tableboat is not running." }
    $rootId = $proc.Id
}

try {
    $all = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, CommandLine
    $ids = Get-Tree $rootId $all

    $cpuBefore = Get-CpuMs $ids
    Start-Sleep -Seconds $IdleSeconds
    $cpuAfter = Get-CpuMs $ids

    $perf = Get-CimInstance Win32_PerfFormattedData_PerfProc_Process | Where-Object { $ids.Contains([int]$_.IDProcess) }

    $rows = foreach ($p in $perf) {
        $id = [int]$p.IDProcess
        $cmd = ($all | Where-Object { [int]$_.ProcessId -eq $id } | Select-Object -First 1).CommandLine
        $cpu = if ($cpuBefore.ContainsKey($id) -and $cpuAfter.ContainsKey($id)) { [math]::Round($cpuAfter[$id] - $cpuBefore[$id]) } else { 0 }
        [pscustomobject]@{
            Kind      = Get-Kind $cmd
            Pid       = $id
            PrivateMB = [math]::Round($p.WorkingSetPrivate / 1MB, 1)
            IdleCpuMs = $cpu
        }
    }
    $rows | Sort-Object PrivateMB -Descending | Format-Table -AutoSize | Out-String | Write-Host

    $totalPrivate = [math]::Round(($rows | Measure-Object PrivateMB -Sum).Sum, 1)
    $totalCpu = ($rows | Measure-Object IdleCpuMs -Sum).Sum
    $cores = [Environment]::ProcessorCount
    $pct = [math]::Round(($totalCpu / ($IdleSeconds * 1000 * $cores)) * 100, 3)
    Write-Host "Processes: $($rows.Count)    Total private memory: $totalPrivate MB"
    Write-Host "Idle CPU over $IdleSeconds s: $totalCpu ms of CPU time ($pct % of all $cores cores)"
}
finally {
    if ($started) { Stop-Process -Id $started.Id -ErrorAction SilentlyContinue }
}
