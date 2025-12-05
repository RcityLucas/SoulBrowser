param(
    [switch]$DryRun
)

$root = Split-Path -Path $PSScriptRoot -Parent
$targets = @(
    "soulbrowser-output",
    "tmp",
    "plan.json",
    "plan_test.json"
)

$removedAny = $false
foreach ($rel in $targets) {
    $full = Join-Path $root $rel
    if (Test-Path $full) {
        $removedAny = $true
        if ($DryRun) {
            Write-Host "[dry-run] would remove $full"
        }
        else {
            Remove-Item -Path $full -Recurse -Force -ErrorAction Stop
            Write-Host "Removed $full"
        }
    }
}

if (-not $removedAny) {
    if ($DryRun) {
        Write-Host "[dry-run] nothing to remove"
    }
    else {
        Write-Host "Nothing to remove, workspace already clean."
    }
}
