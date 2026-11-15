# Claude Code Usage Monitor

![Windows](https://img.shields.io/badge/platform-Windows-blue)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

> **This is a standalone fork** of
> [CodeZeno/Claude-Code-Usage-Monitor](https://github.com/CodeZeno/Claude-Code-Usage-Monitor).
> Many thanks to Code Zeno, the original author, for creating the monitor and
> releasing it under the MIT License. The fork is developed separately at
> [fix2it/Claude-Code-Usage-Monitor](https://github.com/fix2it/Claude-Code-Usage-Monitor); please report problems with
> fork builds here, not to the original project.

A Windows taskbar widget for people who juggle several AI quota pools. Where
the original shows one tile per provider, this fork gives each Claude, Codex,
and Antigravity pool its own widget (or combines them with one toggle), shows
Fable usage per Claude pool, reads a second Antigravity pool from the OpenCode
token, and adds Qoder. Each widget keeps its own compact view mode, and pace
ticks show whether you are ahead of or behind the reset window. To keep the
app lean, it ships a single theme, Compact Fluent Quad, in a horizontal
taskbar layout only; Theme Studio and the other themes, including the vertical
one, are removed.

OpenCode Go is supported too. Cursor, Grok Build, and GitHub Copilot support is
inherited from the original project and is untested in this fork. See the
[user guide](USER_GUIDE.md) for everyday use and the
[changelog](CHANGELOG.md) for version history and notable changes.

![Claude Code Usage Monitor running in the Windows taskbar](.github/animation.gif)

## What the fork changes

Compared with the original project, this fork:

- Removes Theme Studio, the context-menu editor, the Assets page, and the extra
  bundled themes. The settings window keeps Settings and Diagnostics; Compact
  Fluent Quad is the only bundled theme. User themes still load.
- Adds three compact view modes for the widget, switched with a click or the
  mouse wheel. Each widget keeps its own mode; hold Shift to change all widgets.
- Shows separate pool widgets for Claude, Codex, and Antigravity (agy), with a
  **Pools on one widget** menu toggle to combine or split them.
- Adds a second Antigravity pool, **O**, read-only from the OpenCode token, next
  to pool **A**; external models get their own widget state.
- Shows Fable usage per Claude pool, with its own toggle in each pool's menu.
- Nests Fable and External models under their providers in a multi-select
  provider menu.
- Draws pace ticks on usage gauges (**Show pace**) and animates hover
  crossfades between pools and labels.
- Sizes compact rows from the measured text and markers, so widths adapt to
  the content.
- Reworks the OpenCode Go widget: it keeps the 7-day window and the billing
  period start, adds dual-mode gauges and markers with a hover swap between
  the weekly and monthly pools, and sends the `Authorization: Bearer` header
  that the OpenCode console API requires.
- Adds a Qoder provider read from the `qodercli` `/usage` panel (Qoder has no
  plain usage API; probes run at most every 15 minutes). The widget shows the
  plan on the top row and the extra credit pool in use below, with pace ticks;
  when both extra pools are in use, hover swaps between them. Pace assumes a 30-day window because Qoder reports only
  end dates. See [Qoder provider](docs/qoder-provider.md).
- Updates only from this fork's own GitHub Releases through the portable
  updater; the original project's WinGet package is not used.

See the [changelog](CHANGELOG.md) for details.

## Features

- Displays current usage and time remaining until each limit resets
- Counts usage up from zero or down from the full allowance, whichever you prefer
- Supports Claude Code, Codex, Google Antigravity, OpenCode Go, Qoder, Cursor, Grok Build, and GitHub Copilot
- Supports multiple accounts for Claude Code and Codex
- Lives in the Windows taskbar with quick controls in the system tray
- Supports multiple monitors and Windows startup
- Includes configurable refresh intervals, providers, languages, and updates
- Ships the Compact Fluent Quad theme and loads user themes
- Collects no analytics or telemetry

## Requirements

- Windows 10 or Windows 11
- At least one supported provider installed and signed in

Claude Code credentials can be detected from the CLI, Claude desktop app, or WSL. Other providers are optional and can be enabled independently from the dashboard.

> **Cursor, Grok Build, and GitHub Copilot** support is inherited unchanged from
> the original project and has not been tested in this fork.

## Installation

Download `claude-code-usage-monitor.exe` from this fork's
[GitHub Releases](https://github.com/fix2it/Claude-Code-Usage-Monitor/releases) and run it; no installer is needed.
The original project's WinGet package installs the upstream app, not this fork.

Fork versions encode the upstream base in the patch number: `2.23.601` is
fork build 01 on top of upstream `2.23.6`.

See [updater verification](docs/updater.md) for the portable updater's integrity checks and trust boundary.

## Usage

Start the monitor:

```powershell
claude-code-usage-monitor
```

Open the settings dashboard directly:

```powershell
claude-code-usage-monitor --dashboard
```

Use the dashboard to select providers, change the refresh interval, choose a display, enable startup, or customize the widget. **Settings > Display > Usage direction** switches the default theme and other themes that support this setting between showing what has been used and what is left, with Used as the default. Selecting Remaining makes a fresh limit read 100% and drain as you work.

Theme authors can opt in with `.display` bindings, including `{claude.session.display:usage_line}` and `{claude.session.display:usage_badge}`. Existing `.percentage`, `.remaining`, and unsuffixed usage summaries keep their meaning; warning thresholds should continue to use `.percentage`.

In the default theme, left-click a provider tray icon to show or hide the widget and right-click it to open the menu.

## Provider setup

| Provider | Setup |
| --- | --- |
| Claude Code | Sign in with the Claude Code CLI or desktop app. Windows and WSL credentials are detected automatically. |
| Codex | Install and sign in to the Codex CLI, then enable Codex in **Providers**. |
| Google Antigravity | Sign in to Antigravity, then enable it in **Providers**. |
| OpenCode Go | Connect an OpenCode Go account, configure the credentials described below, then enable OpenCode in **Providers**. |
| Cursor | Sign in to Cursor, then enable it in **Providers**. The local session is detected automatically. |
| Grok Build | Run `grok login` in the Grok Build CLI, then enable Grok in **Providers**. The signed-in session is detected automatically. |
| GitHub Copilot | Sign in with the Copilot CLI (`copilot`, then `/login`) or the GitHub CLI (`gh auth login`), then enable Copilot in **Providers**. The stored login is detected automatically. |

For OpenCode Go, set `OPENCODE_GO_WORKSPACE_ID` and `OPENCODE_GO_AUTH_COOKIE`, or create `%APPDATA%\opencode-go\config.json`:

```json
{
  "workspaceId": "wrk_01...",
  "authCookie": "__Host-console_session=your-session-cookie-value"
}
```

The workspace ID is part of the OpenCode Go console URL: `https://opencode.ai/console/<workspaceId>/go`. Copy the `__Host-console_session` cookie from an authenticated `opencode.ai` browser session, including its name as shown above. A full Cookie header containing `__Host-console_session` or the legacy `auth` cookie is also accepted unchanged; no empty `auth=;` prefix is needed. Bare legacy `auth` cookie values remain supported. These formats work for both `authCookie` and `OPENCODE_GO_AUTH_COOKIE`. Set `OPENCODE_GO_CONFIG_FILE` to use a different config path. The monitor reads usage from the console JSON API using this workspace ID and cookie.

For Cursor, `CURSOR_SESSION_TOKEN` can override the automatically detected local session.

Grok Build usage comes from the session the CLI stores in `%USERPROFILE%\.grok\auth.json`, read through the same billing endpoint as the CLI's own `/usage` panel. Set `GROK_HOME` if the CLI keeps its home directory elsewhere. Only xAI sign-in entries are used; corporate identity-provider tokens and stored API keys are excluded. A bare `XAI_API_KEY` is not enough: the shared weekly allowance is only readable with a signed-in session. Grok reports one pool per billing period rather than a five-hour window, so the monitor shows it on the long-window row alongside the other providers, leaving the short-window row empty. On-demand spending replaces the pool on that row once any is used, as it already does for Claude Code and Codex.

GitHub Copilot usage comes from `api.github.com/copilot_internal/user`, the endpoint Copilot's own editors and CLI read. The monitor uses the first GitHub login that endpoint accepts, in this order: the `COPILOT_GITHUB_TOKEN`, `GH_TOKEN`, or `GITHUB_TOKEN` environment variable, the Copilot CLI login, then the GitHub CLI login, both read from Windows Credential Manager. Copilot spends one premium-request pool per calendar month, so the monitor shows it on the long-window row and leaves the short-window row empty. Metered chat and completion allowances, as on Copilot Free, are available to themes through `copilot.limits.chat` and `copilot.limits.completions`.

## Data and privacy

The monitor reads local sign-in credentials for enabled providers and sends usage requests directly to their official services. It has no backend service, collects no telemetry, and does not upload credentials or project files.

Credentials are read without modifying the provider files that contain them. When Grok Build rejects a stored token, the monitor asks the Grok CLI to refresh its own session rather than rewriting `auth.json` itself. OpenCode Go credentials saved in a JSON configuration file are plain text and should be protected like a browser session cookie.

## Troubleshooting

Hover over the tray icon for the latest failure reason, or check the account
status under **Settings > Providers > Accounts**. Missing credentials, expired
or rejected logins, network failures, HTTP errors, and unexpected usage responses
have distinct messages. Sign in again using the Claude desktop app or the CLI
that owns the affected account, then refresh the monitor.

Open **Diagnostics** in the dashboard and enable recording to inspect or copy
polling errors. Logging is optional; enable it before reproducing the problem.

Run diagnostics with:

```powershell
claude-code-usage-monitor --diagnose
```

The diagnostic log is written to `%TEMP%\claude-code-usage-monitor.log`. Application settings are stored in `%APPDATA%\ClaudeCodeUsageMonitor\settings.json`.

If the app unexpectedly closes because of a Rust panic, it automatically appends
the panic message, source location, and thread details to the same log, even when
diagnostic recording is off. Copy the log before starting a new `--diagnose`
session, which clears it. Review and redact credentials, account identifiers,
and personal information before sharing relevant excerpts in a crash report.
Report suspected vulnerabilities privately using [SECURITY.md](SECURITY.md).

## Build from source

Install [Rust via rustup](https://www.rust-lang.org/tools/install), then run:

```powershell
cargo build --release
```

The repository pins Rust 1.95.0 in `rust-toolchain.toml`; rustup automatically
selects this version. CI uses the same version. When upgrading Rust, update
`rust-toolchain.toml`, `.github/workflows/release.yml`, and the `rust-version`
in `Cargo.toml` together.

The executable will be created at `target\release\claude-code-usage-monitor.exe`.

Windows MSVC builds statically link the C runtime through `.cargo/config.toml`,
including release builds in CI. The executable does not require a separate
Microsoft Visual C++ Redistributable installation; it still uses built-in Windows
system libraries.

See [dependency security](docs/dependency-security.md) for automated dependency
updates, CI security checks, and the commands to run those checks locally.

## Contributing and community

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, reporting bugs,
and submitting pull requests, and follow our [Code of Conduct](CODE_OF_CONDUCT.md).
For suspected vulnerabilities or credential exposure, use the private reporting
instructions in our [security policy](SECURITY.md).

## License

Licensed under the [MIT License](LICENSE).
