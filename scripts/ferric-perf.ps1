# Ferric OS-Level Performance Monitor
# Writes CPU, RAM, Handles, and Threads every 10 seconds to ferric-perf.csv
$out = "ferric-perf.csv"
"Time,Process,PID,CPU_Sec,RAM_MB,Handles,Threads" | Out-File $out -Encoding utf8
Write-Host "Monitoring ferric, msedgewebview2, and python processes... (Output: $out)" -ForegroundColor Cyan
while ($true) {
    $ts = Get-Date -Format "HH:mm:ss"
    Get-Process -ErrorAction SilentlyContinue |
        Where-Object { $_.ProcessName -match "ferric|msedgewebview2|python" } |
        ForEach-Object {
            "$ts,$($_.ProcessName),$($_.Id),$([math]::Round($_.CPU,2)),$([math]::Round($_.WorkingSet64/1MB,1)),$($_.HandleCount),$($_.Threads.Count)"
        } | Out-File $out -Append -Encoding utf8
    Start-Sleep -Seconds 10
}
