# User guide

Everyday use of this fork of Claude Code Usage Monitor: the widget, its context
menu, pools, view modes and settings. For installation and provider sign-in,
see the [README](README.md). The instructions use the app's English labels.

## Contents

- [The widget](#the-widget)
- [Switch view modes](#switch-view-modes)
- [Choose providers and pools](#choose-providers-and-pools)
- [Fable and external models](#fable-and-external-models)
- [Pace ticks](#pace-ticks)
- [Settings menu](#settings-menu)
- [Settings window](#settings-window)
- [Show used or remaining allowance](#show-used-or-remaining-allowance)
- [Themes](#themes)
- [Windows VM testing for developers](#windows-vm-testing-for-developers)

## The widget

The widget sits in the Windows taskbar and shows one tile per provider or pool:
a gauge for each limit window and the time until it resets. The fork ships one
theme, Compact Fluent Quad, in a horizontal taskbar layout. Rows are sized from
the text they actually show, so the widget is only as wide as it needs to be.

- **Left-click** a widget to change its view mode.
- **Mouse wheel** over a widget steps its view mode up or down.
- **Right-click** the widget to open the context menu.
- **Hover** a tile to see its other pool or labels, where a tile has them, and
  a tooltip with the latest reading or error.

## Switch view modes

Each widget has three compact view modes that trade bar width against text.

- A click moves to the next mode and bounces at the ends: 1, 2, 3, 2, 1, ...
- The wheel moves one step toward mode 1 (wheel up) or mode 3 (wheel down).
- Only the widget under the cursor changes. Hold **Shift** to change all
  widgets to the same mode.

Each widget's mode is saved and restored after a restart.

## Choose providers and pools

Right-click the widget and open **Providers**. Claude Code and Codex each have
two pools, **Main** and **X**; Antigravity has **Main** and **OpenCode**:

| Menu item | What it shows |
| --- | --- |
| **Claude Code (Main)**, **Claude Code X** | Two Claude Code profiles: the default one and a second one in `~/.claude_x`. Sign each in to the account you want. |
| **Codex (Main)**, **Codex X** | Two Codex profiles: the default one and a second one in `~/.codex_x`. |
| **Antigravity (Main)** | Antigravity pool A, from Antigravity's own sign-in. |
| **Antigravity (OpenCode)** | Pool O, read-only from the Google token that OpenCode stores. It is never refreshed by the monitor; when the token is missing or expired the pool shows no data, and it is hidden when it matches pool A. |
| **OpenCode**, **Qoder** | OpenCode Go and Qoder usage. |
| **Cursor**, **Grok**, **Copilot** | Inherited from the original project; untested in this fork. |

With both pools of a provider enabled, the indented **Pools on one widget**
item appears under it (the bundled theme supports it; a user theme may not). On (the default), both pools share one widget. Off,
each pool gets its own widget with its own view mode.

## Fable and external models

Under each Claude pool, the indented **Fable** item shows that pool's Fable
usage. With one Claude pool, Fable is a third row of the Claude widget. With
two pools combined, their Fable rows share a separate widget; with pools
split, each pool's widget holds its own Fable row.

Under each Antigravity pool, **External models** shows the Claude/GPT usage of
that pool. With pools split, each pool's external models are a row inside its
own widget. With pools combined, the external models of both pools share a
separate widget, which has its own hover.

## Pace ticks

**Settings > Show pace** draws a thin tick on usage gauges at the share of the
limit window that has already passed (with **Remaining** direction, the share
still left). With **Used**, a bar past the tick means you are using the
allowance faster than the window elapses; with **Remaining**, a bar past the
tick means you are using it more slowly. Ticks are hidden when a provider
reports no window start, and in the view mode that shows bars only. Qoder reports only reset
dates, so its pace assumes a 30-day window.

## Settings menu

The context menu's **Settings** submenu has **Start with Windows**, **Lock in
taskbar**, **Show pace**, **Language**, and **Check for updates**. **Update
frequency** sets how often usage is refreshed (every minute, 5 or 15 minutes,
or every hour); **Refresh** fetches usage now. Qoder is probed at most every
15 minutes whatever the frequency.

**Check for updates** looks only at this fork's
[GitHub Releases](https://github.com/fix2it/Claude-Code-Usage-Monitor/releases).

## Settings window

Choose **Open Dashboard** from the context menu, or run:

```powershell
claude-code-usage-monitor --dashboard
```

The window has two pages, **Settings** and **Diagnostics**. Settings covers
providers and accounts, update frequency, language, startup, the taskbar lock,
and usage direction. Diagnostics records polling errors when you enable it; see
[Troubleshooting](README.md#troubleshooting).

## Show used or remaining allowance

1. Open **Settings > Display**.
2. Set **Usage direction** to **Used** or **Remaining**.

| Option | What the percentage means | Example |
| --- | --- | --- |
| **Used** (default) | How much of the allowance you have consumed. | After using 30%, the display reads 30%. |
| **Remaining** | How much allowance is left. | After using 30%, the display reads 70%. |

## Themes

Theme Studio, the menu editor, and the other bundled themes were removed in
this fork. A saved selection of a removed theme falls back to Compact Fluent
Quad. Theme files you made yourself still load from the theme selector, but
pool splitting, per-widget modes, and Fable rows need theme support; themes
without it keep the combined layout.

## Windows VM testing for developers

Run this harness locally on a developer machine with Hyper-V. The scripts live in
`tests/windows-vm/`. The harness provisions two disposable guests and runs desktop
scenarios against known checkpoints. `New-LabVM.ps1` applies an installation image
to a new VHDX, prepares a local account and boots the guest. `Invoke-Lab.ps1` runs
the test matrix from the ordered `tests.json` catalogue. See
[extending the Windows VM tests](docs/windows-vm-testing.md) for test selection,
suites, results, and how to add a test without changing the runner.

### Coverage

| Flow | What the runner exercises | Inputs |
| --- | --- | --- |
| `portable-launch` (default smoke suite) | Launch from a path with spaces, visible widget, one instance, restart, retained language and poll interval | Candidate EXE |
| `first-run-clean-profile` (default smoke suite) | Clean-profile defaults, visible widget, missing-sign-in message, and first dashboard launch | Candidate EXE |
| `portable-dashboard-warp` | Automatic and forced software-renderer fallback, dashboard input and clipboard, resizing, single instance, close, and persisted settings | Candidate EXE; stock Hyper-V graphics |
| `portable-taskbar-tray` | Drag into free taskbar space, add/remove notification icons, assert real tray growth, no false undocking, stable free-space position and restoration; save geometry samples and screenshots | Candidate EXE; visible taskbar |
| `portable-update-helper` | Old EXE launches; its real `--apply-update` helper verifies, replaces and relaunches the candidate; candidate SHA-256 and retained settings checked | Previous and newer candidate EXEs |
| `winget-install` | Public-source installation of a pinned version, package detection, launch, restart, uninstall | Published candidate version |
| `winget-upgrade` | Public-source installation of a pinned old version, CLI upgrade to a pinned candidate, retained settings, launch and uninstall | Two published versions |

The helper scenario does **not** cover the app's update button, GitHub metadata
check, download, or staging. The old EXE must support the current helper arguments
(size and SHA-256). WinGet scenarios test `winget` commands, not the app's WinGet
update action. Unpublished candidates are supported only in portable scenarios.
Neither result should be reported as full updater end-to-end coverage.

The default `smoke` suite runs portable launch and clean-profile first launch on
the baseline taskbar on both OS versions: four scenarios. `-Suite regression`
includes launch, dashboard WARP, and taskbar/tray tests across their supported
modes. Explicit `-Tests` (or the
compatible `-Flows` alias) uses each test's declared taskbar matrix: normally
baseline and auto-hide on both OS versions, plus left and centre alignment on
Windows 11. Unsupported combinations are reported as skipped with a reason.
`left` means Windows 11 icon alignment, not a Windows 10 taskbar edge.
Use selected `-Flows` and `-Taskbars` for quick checks.
Use `-VMNames CCUM-Win10` or `-VMNames CCUM-Win11` to select a single guest.

For tray regressions, use `-Flows portable-taskbar-tray -Taskbars baseline`
(or `left,center` on Windows 11). This flow requires a visible horizontal taskbar.
It uses synthetic drag messages against the real widget window procedure and real
notification icons. By default it uses a 120 x 32 pixel fixture that fits both OS
taskbars at 100% scaling. Use `-TrayTheme classic` to exercise the default theme on
Windows 11; that theme is taller than the stock Windows 10 taskbar. A shell that
keeps every added icon in overflow fails the
tray-growth precondition; that result is not evidence that positioning is correct.
Vertical taskbars, DPI changes and insufficient-space clamp cases remain covered
by Rust tests rather than this desktop scenario.

Assertions inspect process count/path, widget class and visibility, nonempty bounds,
display intersection (except with auto-hide), hashes/versions where applicable,
settings values, and command exit codes. Screenshots support visual review; passing
does not establish freedom from clipping, overlap, rendering defects, or correct
auto-hide animation. Alignment checks verify the registry preference, not icon positions.
Restarts currently terminate the app process; graceful shutdown is not covered.

### Try the harness without VMs

From the repository root in Windows PowerShell 5.1:

```powershell
.\tests\windows-vm\Test-Harness.ps1
.\tests\windows-vm\Invoke-Lab.ps1 -ListTests
.\tests\windows-vm\Invoke-Lab.ps1 -PlanOnly
.\tests\windows-vm\Invoke-Lab.ps1 -Suite regression -PlanOnly
.\tests\windows-vm\Invoke-Lab.ps1 -PlanOnly `
  -Flows portable-launch,portable-update-helper,winget-install,winget-upgrade
```

`-PlanOnly` validates the configuration and emits the scenario matrix as JSON,
without loading Hyper-V, checking binaries, or requiring credentials.
The harness checks validate configuration, matrix generation, VM/checkpoint refusal
paths, PowerShell parsing and C# bridge compilation. They do not simulate Windows UI.

### Provision from installation media

From an elevated Windows PowerShell session, supply a lab-only account credential:

```powershell
$credential = Get-Credential -UserName LabUser -Message 'New disposable VM account'
.\tests\windows-vm\New-LabVM.ps1 -VMName CCUM-Win10 `
  -IsoPath C:\work\ISO\Windows10.iso -ImageName 'Windows 10 Pro' `
  -Credential $credential
.\tests\windows-vm\New-LabVM.ps1 -VMName CCUM-Win11 `
  -IsoPath C:\work\ISO\Windows11.iso -ImageName 'Windows 11 Enterprise Evaluation' `
  -Credential $credential
```

Choose image names actually present in your media. The Windows 11 media supplied
for the first run contains Enterprise Evaluation. The script does not activate
Windows or read product-key files. VM files default to `C:\work\CCUM-Lab`.
Existing VM names and provisioning directories are refused. `-Resume` can continue
after image application if `image-applied.json` matches the selected VM and media.
It never reformats an existing disk when resuming. Interrupted image application
without this marker requires separate inspection, not an automatic retry.

The new account signs in once to run `Initialize-Guest.ps1`. It disables automatic
locking/sleep and automatic Windows updates in the guest for a reproducible lab,
then removes temporary autologon credentials and the answer file. A successful
setup writes `C:\CCUM-Lab\initialized.json` inside the VM. Wait for that marker and
a settled desktop before taking checkpoints. Setup can reboot several times.
`Save-VMConsole.ps1 -VMName CCUM-Win10 -Path C:\work\win10.png` captures the console
through Hyper-V even before the guest bridge is available.

The account password temporarily exists in the guest's unattended installation
file during setup. For unattended host runs, a PSCredential can be saved with
`Export-Clixml` outside the repository; Windows encrypts it for that host user.
The first run used `C:\work\CCUM-Lab\credential.clixml` with a restricted ACL.
Never print the decrypted credential or put it in source control.

### Prepare checkpoints

Suggested initial allocation: 2 vCPUs, 4 GB RAM and a 64 GB dynamically expanding
disk per guest. Run scenarios sequentially. Use x64 Windows 10 22H2 and a current
supported Windows 11 build; record the exact editions, builds and patch dates.
Windows 11 needs a generation 2 VM with Secure Boot and virtual TPM enabled.
Use your own installation media and licences. Activation remains separate.

The host must have Hyper-V and a PowerShell session with Hyper-V administrative
access. On each VM:

1. Use the provisioned local administrator named `LabUser`, with the same lab-only credential
   on both guests. Leave it signed into an **unlocked console desktop** with Explorer
   running. Use the basic VMConnect console; an RDP session that locks/disconnects
   can invalidate desktop tests. The runner will not automatically log in or unlock.
2. Install and initialise App Installer/WinGet for that user. Keep its default
   per-user portable package directory and public `winget` source. The guest needs
   internet access for WinGet. Do not install the monitor or sign into real provider
   accounts; the baseline must have no monitor settings or running monitor process.
   `Initialize-WinGet.ps1` must run in the interactive desktop. `-UseInstalled`
   keeps an existing usable client. Otherwise stage Microsoft's official bundle at
   `C:\CCUM-Lab\downloads\Microsoft.DesktopAppInstaller_8wekyb3d8bbwe.msixbundle`
   and its x64 dependencies under `C:\CCUM-Lab\downloads\x64`. It records the
   actual version and package lookup result; a source-update exit code alone is
   insufficient because WinGet can report zero after a cancelled update.
3. Use a known display resolution/scaling and a bottom, visible taskbar. Disable
   sleep, screen saver/automatic locking, and automatic monitor startup in this
   disposable test profile. Complete pending OS updates/reboots before capture.
4. In the guest, create the explicit opt-in marker:

   ```powershell
   New-Item -ItemType Directory -Path C:\CCUM-Lab -Force
   New-Item -ItemType File -Path C:\CCUM-Lab\guest-enabled -Force
   ```

5. On the host, mark **only the disposable test VMs** and take standard checkpoints
   while their console desktops are signed in and unlocked:

   ```powershell
   foreach ($name in 'CCUM-Win10', 'CCUM-Win11') {
     Set-VM -Name $name -Notes 'CCUM-DISPOSABLE-LAB' `
       -CheckpointType Standard -AutomaticCheckpointsEnabled $false
     Checkpoint-VM -Name $name -SnapshotName 'ccum-clean-desktop'
   }
   ```

Standard checkpoints captured with a running desktop preserve VM memory, which
lets this POC resume the desktop session without storing autologon passwords.
The runner requires a checkpoint with running, saved, or paused state; production
and powered-off checkpoints have no desktop memory and are refused.
Use exactly one checkpoint with that name per VM. Rebuild/checkpoint updated images
deliberately instead of letting the baseline change between scenarios.

Copy `tests/windows-vm/lab.example.json` to `tests/windows-vm/lab.local.json` to customise VM names, checkpoint names
and usernames. Names must start with `CCUM-`. Local config and default evidence are
gitignored. Credentials are supplied as PSCredential objects, never written to the
JSON config or scenario logs.

Keep a pristine checkpoint **without** the Visual C++ Redistributable for detecting
undeclared runtime dependencies. A separate `ccum-runtime-desktop` checkpoint can
include the signed Microsoft x64 redistributable for exercising the rest of the
matrix. Do not treat passing results on that checkpoint as clean-install success.

### Run against the prepared guests

Build a candidate, then run from an elevated Windows PowerShell host session:

```powershell
.\tests\Test-Dependencies.ps1
cargo build --release
$credential = Get-Credential -UserName LabUser -Message 'Disposable VM local administrator'
.\tests\windows-vm\Invoke-Lab.ps1 `
  -CandidateExe .\target\release\claude-code-usage-monitor.exe `
  -Credential $credential
```

Portable helper coverage (the candidate product version must be newer):

```powershell
.\tests\windows-vm\Invoke-Lab.ps1 `
  -Flows portable-update-helper -Taskbars baseline `
  -PreviousExe C:\LabBinaries\previous\claude-code-usage-monitor.exe `
  -CandidateExe .\target\release\claude-code-usage-monitor.exe `
  -Credential $credential
```

WinGet coverage uses **versions actually available in the public source**, supplied
explicitly, rather than silently testing whichever release is latest:

```powershell
# Set these to the published versions you intend to validate.
$previousVersion = '2.14.54'
$candidateVersion = '2.14.55'
.\tests\windows-vm\Invoke-Lab.ps1 `
  -Flows winget-install,winget-upgrade -Taskbars baseline `
  -PreviousVersion $previousVersion -CandidateVersion $candidateVersion `
  -Credential $credential
```

These version numbers are examples, not an availability assertion. Missing versions,
source/network failures and timeouts are reported as failures. WinGet scenarios do
not use `-CandidateExe`. Add `-ConfigPath .\tests\windows-vm\lab.local.json` when
using local configuration.

For each scenario the host restores the baseline, transfers scripts and binaries
through PowerShell Direct, starts a limited-token interactive scheduled task for
`LabUser`, waits for its result, copies evidence, and restores the baseline again
in `finally`. The guest must match the configured OS. A global mutex prevents
concurrent harness runs from restoring each other's VMs. No inbound guest service,
WinRM network listener or Docker service is required.

The runner restores VM state, discarding changes since the checkpoint. It requires
the exact disposable-VM notes marker before touching any VM. A guest marker and
Hyper-V model check prevent accidental guest-script execution on a physical host.
Abrupt host shutdown or killing PowerShell can prevent `finally`; restore the named
checkpoint manually before reuse if a run is forcibly interrupted.

### Results and AI use

`tests/windows-vm/artifacts/<run-id>/summary.json` records every planned scenario,
including skipped and unexecuted scenarios, duration, failed checks, and evidence
paths. `summary.md` provides a readable table with evidence links.
Each scenario's `evidence` subfolder contains its result/checks, OS build,
window bounds, screenshots, executable version/hash, settings, diagnostics,
transcript and WinGet output when used. The host also records the scheduled task's
exit result. Files are collected individually so a locked transcript cannot block
the remaining evidence. Timeout evidence may be partial. An ordinary failure continues to the
next scenario; checkpoint cleanup failure aborts the run. Any failed scenario makes
the command fail with a nonzero process exit when invoked with `powershell.exe -File`.

An AI can invoke the runner, read JSON/logs, inspect PNGs, change code, and rerun a
selected matrix once host access and VM setup are available. The harness determines
pass/fail. There is no AI agent installed in the guests and no automatic repair or
deployment action. Keep provider credentials and personal data out of the lab;
evidence intentionally includes the test user's settings and desktop.

### Follow-up coverage

- A test-only, explicit HTTPS release endpoint and trusted fixture assets for full
  portable update check/download/staging/UI tests, retaining size/digest validation.
- A private WinGet test source for unpublished old/candidate versions; separate
  app-initiated upgrade and public-source availability tests.
- First launch with no settings file, repeated launch, graceful exit, Explorer
  recovery, reboot, failed/interrupted updates and migration assertions.
- Windows 10 taskbar edges, crowded taskbars, light/dark modes, DPI/resolution
  changes and multi-monitor scenarios with explicit geometry assertions.

Keep the fast Rust tests, especially updater safeguards and placement regression
tests. This POC adds real desktop evidence; it does not justify removing them yet.

### Platform references

- [PowerShell Direct requirements and file transfer](https://learn.microsoft.com/en-us/windows-server/virtualization/hyper-v/powershell-direct)
- [Hyper-V standard and production checkpoints](https://learn.microsoft.com/en-us/windows-server/virtualization/hyper-v/checkpoints)
- [Interactive scheduled-task principals](https://learn.microsoft.com/en-us/powershell/module/scheduledtasks/new-scheduledtaskprincipal)
- [Windows 11 VM requirements](https://learn.microsoft.com/en-us/windows/whats-new/windows-11-requirements)
- [WinGet upgrade options](https://learn.microsoft.com/en-us/windows/package-manager/winget/upgrade)
