# Allow LANLink through Windows Firewall on PRIVATE networks only.
# Run from an elevated PowerShell after reviewing it. Not run automatically.
param([int]$Port = 47990)
New-NetFirewallRule -DisplayName "LANLink host (UDP $Port, private networks)" `
    -Direction Inbound -Protocol UDP -LocalPort $Port -Action Allow -Profile Private
New-NetFirewallRule -DisplayName "LANLink discovery (mDNS, private networks)" `
    -Direction Inbound -Protocol UDP -LocalPort 5353 -Action Allow -Profile Private
Write-Host "Done. If your Wi-Fi is set to 'Public', change it to 'Private' in Windows Settings > Network."
