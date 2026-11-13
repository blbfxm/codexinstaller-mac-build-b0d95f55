param(
    [Parameter(Mandatory = $true)]
    [string]$MsiPath
)

$ErrorActionPreference = "Stop"

$resolvedMsiPath = (Resolve-Path -LiteralPath $MsiPath).Path
$installer = New-Object -ComObject WindowsInstaller.Installer
$database = $installer.OpenDatabase($resolvedMsiPath, 1)

function Invoke-MsiQuery([string]$Sql) {
    $view = $database.OpenView($Sql)
    $view.Execute()
    $view.Close()
}

# Do not restore a previous directory into INSTALLDIR; the MSI UI/default
# directory should be authoritative for every install and upgrade.
Invoke-MsiQuery "DELETE FROM AppSearch WHERE Property = 'INSTALLDIR'"
Invoke-MsiQuery "DELETE FROM RegLocator WHERE Signature_ = 'PrevInstallDirNoName'"
Invoke-MsiQuery "DELETE FROM RegLocator WHERE Signature_ = 'PrevInstallDirWithName'"
$database.Commit()

$verificationView = $database.OpenView("SELECT Property FROM AppSearch WHERE Property = 'INSTALLDIR'")
$verificationView.Execute()
$remainingSearch = $verificationView.Fetch()
$verificationView.Close()
if ($null -ne $remainingSearch) {
    throw "Failed to remove the MSI INSTALLDIR registry search."
}

Write-Output "Removed historical INSTALLDIR lookup from: $resolvedMsiPath"
