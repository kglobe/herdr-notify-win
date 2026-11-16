# Builds herdr-toast.exe, installs the win-toast mod, registers the toast AppID and its click
# handler, starts the exe now and at every logon, and switches Herdr to a clone of the main layout
# whose notifications go to win-toast. Re-run to update.
$ErrorActionPreference = 'Stop'
$app = if ($env:HERDR_APP_HOME) { $env:HERDR_APP_HOME } else { "$env:USERPROFILE\.config\herdr-app" }
$mod = "$app\mods\win-toast"
$local = "$env:LOCALAPPDATA\herdr-win-toast"
$exe = "$local\herdr-toast.exe"
$herdr = "$env:LOCALAPPDATA\Programs\herdr-app\herdr-app.exe"
$aumid = 'Herdr.Notify'                                  # must match src/main.rs
$clsid = '{F284B6D9-7D6B-4119-9170-D8A75506F3FA}'        # must match src/main.rs

cargo build --release --manifest-path "$PSScriptRoot\Cargo.toml"
if ($LASTEXITCODE) { throw 'cargo build failed' }

# Stop the running copy (and the older PowerShell daemon) so the new one takes over.
Get-Process herdr-toast -ErrorAction SilentlyContinue | Stop-Process -Force
Get-CimInstance Win32_Process -Filter "Name='powershell.exe'" |
    Where-Object CommandLine -match 'win-toast\\daemon\.ps1' |
    ForEach-Object { Stop-Process -Id $_.ProcessId -Force }

New-Item -ItemType Directory -Force $mod, "$local\spool" | Out-Null
Remove-Item "$mod\notify.ps1", "$mod\focus.ps1", "$mod\daemon.ps1" -ErrorAction SilentlyContinue  # earlier versions
Remove-Item 'HKCU:\Software\Classes\herdr-notify' -Recurse -ErrorAction SilentlyContinue           # earlier versions
Copy-Item "$PSScriptRoot\mod\*" $mod -Force
Copy-Item "$PSScriptRoot\target\release\herdr-toast.exe" $exe -Force

# Toast header shows DisplayName and this icon; a click calls the COM class below, which the
# running exe serves (or Windows starts the exe to serve it).
# One AppID per agent so the header says which agent it was; all share the click handler.
Add-Type -AssemblyName System.Drawing
[Drawing.Icon]::ExtractAssociatedIcon($herdr).ToBitmap().Save("$local\herdr.png")
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$PSScriptRoot\icons.ps1" -Icons "$app\icons" -Out $local
$headers = @{
    $aumid                 = 'Herdr', 'herdr'
    "$aumid.Claude"        = 'Claude Code', 'claude'
    "$aumid.Codex"         = 'Codex', 'codex'
    "$aumid.Antigravity"   = 'Antigravity', 'antigravity'
}
foreach ($id in $headers.Keys) {
    $key = "HKCU:\Software\Classes\AppUserModelId\$id"
    New-Item -Force $key | Out-Null
    Set-ItemProperty $key DisplayName $headers[$id][0]
    Set-ItemProperty $key IconUri "$local\$($headers[$id][1]).png"
    Set-ItemProperty $key CustomActivator $clsid
    Set-ItemProperty $key IconBackgroundColor '00000000'  # else Windows fills the transparent parts with the accent color
}
New-Item -Force "HKCU:\Software\Classes\CLSID\$clsid\LocalServer32" | Out-Null
Set-ItemProperty "HKCU:\Software\Classes\CLSID\$clsid\LocalServer32" '(default)' "`"$exe`""

$lnk = (New-Object -ComObject WScript.Shell).CreateShortcut("$([Environment]::GetFolderPath('Startup'))\herdr-win-toast.lnk")
$lnk.TargetPath = $exe
$lnk.Arguments = ''
$lnk.Save()
Start-Process $exe

$layout = "$app\layouts\main-wintoast.json"
if (-not (Test-Path $layout)) { & $herdr do layout.clone --name main --as main-wintoast | Out-Null }
$text = [IO.File]::ReadAllText($layout)
if ($text -notmatch '"toast"') {
    # Text edit keeps the file's formatting; the root node is the first "id": "root".
    $text = ([regex]'"id": "root",').Replace($text, '"id": "root", "toast": "win-toast",', 1)
    [IO.File]::WriteAllText($layout, $text)
}
& $herdr do layout.switch --name main-wintoast
