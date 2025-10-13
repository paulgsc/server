---

# PowerShell & WSL Quick Reference Cheat Sheet

This cheat sheet provides useful PowerShell commands and techniques for managing WSL, handling process management, troubleshooting USB issues, and general PowerShell tips.

---

## Table of Contents

- [WSL Management](#wsl-management)
- [Process Management](#process-management)
- [External Drive Troubleshooting](#external-drive-troubleshooting)
- [General PowerShell Commands](#general-powershell-commands)
  
---

## WSL Management

### Shutdown All WSL Instances

Stops all running WSL distributions, freeing up resources.

```powershell
wsl --shutdown
```

### List Running WSL Instances

Displays a list of currently running WSL distributions.

```powershell
wsl --list --running
```

### Start a Specific WSL Distribution

To start a specific distribution (e.g., Ubuntu):

```powershell
wsl -d Ubuntu
```

---

## Process Management

### Killing WSL Processes

If WSL is hanging or needs to be force-stopped, you can terminate the WSL process (usually `vmmem`) with:

```powershell
Stop-Process -Name "vmmem" -Force
```

Or, if you want to kill all processes with "wsl" in their name:

```powershell
Get-Process | Where-Object { $_.Name -like "*wsl*" } | Stop-Process -Force
```

### Kill Process by ID

To kill a specific process by its Process ID (PID):

```powershell
Stop-Process -Id <PID> -Force
```

To find a PID for a process, use:

```powershell
Get-Process | Where-Object { $_.Name -like "<process-name>" }
```

### View Running Processes

To list all currently running processes:

```powershell
Get-Process
```

---

## External Drive Troubleshooting

### Disable USB Selective Suspend

Prevents the USB drive from disconnecting due to power-saving settings.

1. Open `Control Panel > Hardware and Sound > Power Options`.
2. Select `Change plan settings` for your active plan.
3. Click `Change advanced power settings`.
4. Expand `USB settings` > `USB selective suspend setting`.
5. Set to **Disabled**.

### USB Power Management

1. Open `Device Manager`.
2. Expand `Universal Serial Bus controllers`.
3. Right-click each `USB Root Hub` and `Generic USB Hub`.
4. Select `Properties`, then go to the `Power Management` tab.
5. Uncheck `Allow the computer to turn off this device to save power`.

### Assign a Static Drive Letter

1. Right-click on `This PC` and select `Manage`.
2. Open `Disk Management`.
3. Right-click your external drive and choose `Change Drive Letter and Paths`.
4. Assign a consistent drive letter.

---

## General PowerShell Commands

### Check Windows Version

Shows detailed information about your Windows version.

```powershell
Get-ComputerInfo | Select-Object CsName, WindowsVersion, OsArchitecture, WindowsBuildLabEx
```

### List All Installed Software

Displays a list of all installed applications.

```powershell
Get-ItemProperty HKLM:\Software\Wow6432Node\Microsoft\Windows\CurrentVersion\Uninstall\* |
Select-Object DisplayName, DisplayVersion, Publisher, InstallDate | Format-Table -AutoSize
```

### Check Disk Space on All Drives

Displays disk usage information.

```powershell
Get-PSDrive -PSProvider FileSystem | Select-Object Name, @{Name="Used(GB)";Expression={[math]::round($_.Used/1GB,2)}}, @{Name="Free(GB)";Expression={[math]::round($_.Free/1GB,2)}}, @{Name="Total(GB)";Expression={[math]::round($_.Used/1GB + $_.Free/1GB,2)}}
```

### List Environment Variables

Lists all environment variables.

```powershell
Get-ChildItem Env:
```

### Network Configuration Details

Displays detailed network information, including IP addresses, MAC addresses, and DNS servers.

```powershell
Get-NetIPAddress
Get-DnsClientServerAddress
```

---

This reference guide provides some essential commands to make managing WSL, external devices, and system processes smoother. Feel free to expand this with additional commands that suit your specific workflows!
