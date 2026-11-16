# Audit ledger — whole application

- Repository: C:\3_Work\VibeCode\Usage_Monitor (fix2it/Claude-Code-Usage-Monitor), branch audit/app-review
- HEAD: 6dadae35c8c1fb73c8502a764e0acbe86535ddcf (main)
- Date: 2026-11-16
- Levels: 1–6 (all)
- Surfaces: 1 (primary journey), 5 (module slice)
- Extra focus (refactoring): duplication, functions > ~100 lines, dead code, hot paths (per-frame/paint work, allocations, blocking UI thread). Recorded with location and proposal, never dismissed as vague complexity.
- Reviewer: Codex CLI `codex_x exec -m gpt-6.1-terra`, read-only sandbox effort high (confirmed from CLI log header, all 5 rounds)

## Coverage checklist

`[x]` inspected; `[~]` touched in targeted passes (function inventory + selected behavior), not read in full depth. Loop stopped at the 5-pass limit with coverage incomplete.

Surface 1 — primary journey (poll quotas → widget(s) on taskbar → paint → tooltip/menu):
- [x] S1.1 main.rs startup, single instance, settings load
- [~] S1.2 poller.rs dispatch + poller/claude.rs, codex.rs, antigravity.rs, opencode.rs
- [~] S1.3 models.rs / providers.rs pool model
- [~] S1.4 window.rs widget creation, per-provider widgets, pool split ("pools on one widget" flags in app_settings.rs)
- [~] S1.5 window/message_loop.rs, animation_timer.rs
- [~] S1.6 window/positioning.rs, host_geometry.rs, taskbar_collision.rs
- [~] S1.7 theme_engine.rs + theme_rendering.rs paint path, ui/theme.rs, theme.rs
- [~] S1.8 window/mouse.rs, widget_tooltip.rs, window_context_menu.rs, context_menu.rs (nested menu, per-widget modes)

Surface 5 — module slice:
- [~] M1 app_settings.rs, accounts.rs, poller/accounts.rs
- [~] M2 poller/ others (copilot, cursor, grok, qoder, claude_desktop, retry_after, windows_credentials)
- [~] M3 theme_engine/ others (expression, limits, storage, datetime, text_measure), themes/
- [~] M4 studio_app/ + studio_app.rs, dashboard.rs
- [~] M5 updater.rs + updater/
- [~] M6 native_interop*, desktop_compositor.rs, tray_icon.rs, widget_font.rs, winsqlite.rs, known_folders.rs, diagnose.rs, https_test.rs
- [~] M7 localization/, window/lifecycle.rs
- [~] M8 tests (placement, theme_engine/tests.rs, poller/tests.rs) — gaps only

## Findings

### Level 4 — Medium
- F1 | S1 | src/window.rs:1385-1398 | Compact tray summary prints `{:.0}%` bypassing `window_text`, so an unavailable session/weekly window shows as 0% used / 100% left (only Claude session has a guard at 1378) | user sees a fake number instead of "unavailable" | fixed (refactor/1-level4-fixes)

- F46 | M5 | src/updater.rs:287-315 | `replace_target_binary`: if copy fails and restoring the backup also fails, the old exe lives only in the backup, and the next loop iteration (`remove_file(&backup_path)`) deletes it; after 60 failures the install has no exe | user loses the installed app until manual re-download | proposal: never delete backup unless target exists; report restore error | fixed (refactor/1-level4-fixes)
- F47 | S1 | src/window/widget_tooltip.rs:97-103 | usage-summary tooltip: any provider error (`tray_error_lines`) replaces the whole summary, so a healthy widget's tooltip shows only another provider's error (bad with split pools/widgets) | proposal: show healthy usage plus errors, or scope to the widget's provider | fixed (refactor/1-level4-fixes)

- F52 | S1 | src/desktop_compositor.rs:154-185 (caller positioning.rs:432) | failed presenter (device loss / Present error) stays cached in PRESENTERS; caller only logs, so a desktop-nested widget can stay blank/stale until restart | proposal: `remove(hwnd)` on present/resize error, recreate next frame | fixed (refactor/1-level4-fixes)

### Level 5 — Minor (refactoring focus)
Hot paths:
- F2 | S1 | src/window.rs:3032-3045 | `render_layered_impl` rebuilds `effective_theme_from_state` and clones `state.data` on every animation frame under the state lock | proposal: cache effective theme keyed by settings/theme revision; hold usage data in `Arc` | partly fixed (refactor/3-render-hot-path): effective theme cached for animation frames (floating placement still rebuilt per frame); `state.data` clone per frame left as is
- F3 | S1 | src/theme_engine/theme_rendering.rs:134 | `vec![0u32; w*h]` pixel buffer allocated and zeroed every render | proposal: reuse a size-matched buffer per surface | fixed (refactor/3-render-hot-path)
- F4 | S1 | src/theme_engine/theme_rendering.rs:141 | `crate::theme::is_dark_mode()` (registry read) on every floating-card frame | proposal: cache dark-mode flag, invalidate on WM_SETTINGCHANGE | fixed (refactor/3-render-hot-path)
Long functions (> ~100 lines):
- F5 | S1 | src/window.rs:1266-1404 | `usage_summary_lines` (~140 lines), compact/detailed formatting duplicated (root of F1) | proposal: extract provider-label and quota-window formatter helpers | fixed (refactor/1-level4-fixes)
- F6 | S1 | src/window.rs:2240-2418 | `sync_surface_windows_with` (~180) | proposal: split primary recovery / mirror reconcile / desktop-surface reconcile | open
- F7 | S1 | src/window.rs:2597-2988 | `run` (~390) | proposal: split instance setup, state init, loop start | open
- F8 | S1 | src/window.rs:3011-3219 | `render_layered_impl` (~210) | proposal: frame prep / hover bounds / per-surface present | open
- F9 | S1 | src/window.rs:3370-3653 | `do_poll_once` (~280) | proposal: progress publish / success apply / failure handling | open
- F10 | S1 | src/window.rs:4498-4601 | `reload_external_settings` (~100) | proposal: state apply vs refresh actions | open
- F11 | S1 | src/window/message_loop.rs:32-1377 | `wnd_proc` (~1350 lines) | proposal: per-message handler fns (timers, settings commands, lifecycle) | open
- F12 | S1 | src/theme_engine.rs:2227-2435 | `display_metrics` (~210) | proposal: per-provider metric builder | open
- F13 | S1 | src/theme_engine.rs:2675-3167 | `from_usage_with_runtime` (~490) | proposal: split runtime / provider / pool binding population | open
- F14 | S1 | src/theme_engine.rs:3238-3558 | `insert_provider` (~320) | proposal: quota-window, reset-time, provider-specific helpers | open
- F15 | S1 | src/theme_engine/theme_rendering.rs:98-265 | `render_theme_surface_with_metrics_at_scale` (~170) | proposal: setup / background / opacity pass | open
- F16 | S1 | src/theme_engine/theme_rendering.rs:893-1143 | `resolve_geometry` (~250) | proposal: parent resolve / expression eval / transform | open
- F19 | S1 | src/window/window_context_menu.rs:237 | every right-click enumerates, reads, parses and validates the menu directory on the UI thread (reviewer said L4; orchestrator: local small files, click-only → L5) | proposal: cache parsed menus, invalidate on file change | fixed (refactor/2: right-click uses resolve_context_menu_cached, keyed by reference + directory size/mtime fingerprint; errors not cached; known limit: an edit that keeps size and mtime is not seen)
- F20 | S1 | src/theme_engine/theme_rendering.rs:296 | per-object pixel buffer allocated and cleared each frame | proposal: reuse scratch buffers / composite directly | fixed (refactor/3-render-hot-path)
- F21 | S1 | src/theme_engine/theme_rendering.rs:1435-1538 | `render_text_mask` (~100) creates/destroys DC, bitmap, font per text object per frame | proposal: cache fonts and mask resources by params; split setup vs compositing | fixed (refactor/3-render-hot-path)
Duplication:
- F22 | S1 | src/theme_engine.rs:3051 | split-pool binding prep duplicated for Claude and Codex (clear fields, copy namespaces, hover bindings; double map scan) | proposal: one shared pool-binding helper, single pass | open
More long functions:
- F23 | S1 | src/window.rs:1779-1882 | `begin_update_check` | proposal: extract result handling / completion notify | open
- F24 | S1 | src/window/positioning.rs:277-382 | `position_at_taskbar` | proposal: taskbar selection vs placement apply | open
- F25 | S1 | src/window/positioning.rs:516-646 | `position_custom_theme_internal` | proposal: placement resolve vs positioning | open
- F26 | S1 | src/theme_engine.rs:469-579 | `parse_mouse_actions` | proposal: per-action-kind parsers | open
- F27 | S1 | src/theme_engine.rs:3995-4117 | `execute_mouse_actions_with_metrics` | proposal: per-action-kind executors | open
- F28 | S1 | src/theme_engine.rs:4414-4566 | `validate_scene_object` | proposal: common vs content-specific validation | open
- F29 | S1 | src/theme_engine/theme_rendering.rs:1227-1330 | `render_object_content` | proposal: progress / text branches | open
- F30 | S1 | src/window/window_context_menu.rs:224-365 | `show_context_menu_document` | proposal: build / track popup / dispatch | open
- F31 | S1 | src/window/window_context_menu.rs:481-618 | `context_menu_checked_with_origin` | proposal: per-action-family check state | open
- F32 | S1 | src/context_menu.rs:369-574 | `classic_context_menu` | proposal: provider and settings menu builders | open
- F33 | S1 | src/context_menu.rs:824-953 | test `classic_provider_children_follow_their_parents` (L6: test shape) | proposal: split into focused cases | open
UI-thread blocking (round 3):
- F35 | S1/M1 | src/window.rs:1172 | `save_state_settings` loads and writes settings file while holding the global state lock (called from menu actions) | proposal: snapshot under lock, do file I/O after release | fixed (refactor/2: file I/O outside STATE; SETTINGS_IO mutex serializes saves, lock order SETTINGS_IO → STATE)
- F36 | S1/M6 | src/native_interop.rs:435-541 (484,493) | `find_desktop_host` (~105 lines) can block UI thread up to 2×1 s in `SendMessageTimeoutW` when cached host is invalid | proposal: background/bounded rediscovery; split cached-host validation vs discovery | partly fixed (refactor/2: SMTO_ABORTIFHUNG (hung Explorer → no wait) + second WorkerW request skipped after a failed first one (timeout → 1 s instead of 2 s). Still up to ~2 s when the first request succeeds slowly and the second times out; a shared deadline would skip a request that used to succeed. Background rediscovery not done: three callers need the host synchronously, moving it off-thread changes placement timing — visible behavior)
More long functions (round 3):
- F37 | S1 | src/poller/accounts.rs:108-299 | `poll_accounts_with_progress` | proposal: targets / dispatch / aggregate | open
- F38 | M2 | src/poller/qoder.rs:318-429 | `usage_data` | proposal: credit pools vs window select | open
- F39 | M2 | src/poller/qoder.rs:539-660 | `run_usage` | proposal: pseudo-console setup / launch / collect | open
- F40 | S1 | src/poller/claude/limits.rs:19-137 | `parse` | proposal: array parse / legacy buckets / collisions | open
- F41 | M3 | src/theme_engine/theme_expression.rs:490-618 | `format_usage_line` (runs during render, allocates lookup keys) | proposal: provider normalize + availability helpers; reuse keys | fixed (refactor/3-render-hot-path)
- F42 | M4 | src/studio_app/studio_core.rs:342-442 | `shell` | proposal: nav vs page content | open
- F43 | M4 | src/studio_app/studio_diagnostics.rs:113-218 | `diagnostics_page` | proposal: recording controls vs log view | open
- F44 | M4 | src/studio_app/studio_settings.rs:5-203 | `settings_page` | proposal: per-group render helpers | open
- F45 | M4 | src/studio_app/studio_settings.rs:206-311 | `account_settings` | proposal: selection vs editing | open
Round 4:
- F48 | S1 | src/theme_engine/theme_rendering.rs:117 | each surface render builds the full usage `DataContext` twice (incl. formatted dynamic limit keys) | proposal: build once per frame, patch canvas-size values | fixed (refactor/3-render-hot-path)
- F49 | M2 | src/poller/copilot.rs:331 (+ cursor, grok, opencode) | env-value normalization duplicated in 4 pollers; secret-signature logic duplicated in copilot and cursor | proposal: shared helpers in poller.rs | open
- F50 | M5 | src/updater.rs:48 | `InstallChannel::Winget` dead in production (`allow(dead_code)`), plus its update path | proposal: remove or gate under cfg(test) | fixed (refactor/4-dead-code)
- F51 | M4 | src/studio_app/warp.rs:425-564 | test `warp_renders_partial_textures_at_zoom_and_frees_after_paint` 140 lines (L6: test shape) | proposal: split | open
Round 5:
- F53 | S1 | src/window.rs:4328 | hover setters for Claude, Codex, Antigravity, OpenCode, Qoder, agy external repeat state-change + animation + timer logic | proposal: one generic hover-animation helper with per-provider eligibility | open
- F54 | S1 | src/window.rs:1490-1588 | `sync_tray_icon` (~100) mixes state collection, surface filter, render, shell sync | proposal: extract themed-icon build | open
- F55 | S1/M6 | src/desktop_compositor.rs:130 | `Present(1, …)` (vsync wait) on the UI thread for every desktop-surface paint | proposal: measure; `Present(0, …)` or DXGI_PRESENT_DO_NOT_WAIT / waitable swap chain | fixed (refactor/2: Present(0); composition flip-model swap chain does not tear, DWM syncs; not measured with GPU tools)
Dead code:
- F34 | S1 | src/poller/antigravity.rs:514,791 | `fetch_antigravity_model_quota` (`#[allow(dead_code)]`) and its only helper `is_antigravity_display_model` unused | proposal: delete | fixed (refactor/4-dead-code)
- F17 | S5 | src/app_settings.rs:338 | `set_antigravity_external_enabled` only called from tests (1089) | proposal: `#[cfg(test)]` or remove | fixed (refactor/4-dead-code)
- F18 | S1 | src/window.rs:3771,3813 | `Easing::Linear` / `PhaseAnimation::linear` used only by tests | proposal: gate behind `#[cfg(test)]` | fixed (refactor/4-dead-code)

## Borderline

Not applicable (selection ends at level 6).

## Safety overrides

none

## Validation

Required: `cargo clippy --all-targets` (build/lint gate) — run by orchestrator: exit 0, 13 warnings, all test-only style (e.g. bool_assert_comparison, manual_dangling_ptr in taskbar_collision.rs:514 test).
Required: `cargo test` — run by orchestrator: 738 passed, 0 failed, 12 ignored (67 s).

## Score table

| Раунд | Охват | Модель и усилие | Оценка | Новые / в ledger | Самая серьёзная новая | Почему дальше / стоп |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | ур. 1–6, пов. 1+5 | gpt-6.1-terra, high | 4.5 | 18 / 18 | средняя (F1) | новые находки, охват неполный |
| 2 | пов. 1 (S1.2–S1.8) | gpt-6.1-terra, high | 4 | 16 / 34 | мелкая (рецензент дал 4, понижено) | новые находки, S1 прочитан частично |
| 3 | S1 поведение + M1–M8 | gpt-6.1-terra, high | 4 | 11 / 45 | мелкая (блокировка UI) | новые находки, M5 и часть модулей не прочитаны |
| 4 | S1.4 путь пулов + M2–M8 | gpt-6.1-terra, high | 6.5 | 6 / 51 | средняя (F46 обновление, F47 подсказка) | новые находки, путь разбивки пулов не дочитан |
| 5 | S1.4/S1.5/S1.2 глубже + M6–M7 | gpt-6.1-terra, high | 6.5 | 4 / 55 | средняя (F52 пустой виджет на рабочем столе) | стоп: лимит 5 проходов, охват неполный → не принято |

## Result

Stopped incomplete: 5-pass limit reached, still new findings in pass 5, coverage items only partly read in depth. 55 open findings: L4 ×4 (F1, F46, F47, F52), L5 ×49 (F19 demoted from L4 by orchestrator), L6 ×2 (F33, F51, listed in the L5 block). No L1–L3 findings. Duplicates dropped: none reported.
