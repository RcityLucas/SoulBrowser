Param(
    [string]$Root = (Split-Path -Parent $MyInvocation.MyCommand.Path)
)

$repo = Join-Path $Root ".."
Set-Location $repo

$profiles = Get-ChildItem -Directory -Filter ".soulbrowser-profile-*" -ErrorAction SilentlyContinue
$count = 0
foreach ($dir in $profiles) {
    Remove-Item $dir.FullName -Recurse -Force
    $count++
}
Write-Output "Removed $count temporary Chrome profile directories."
