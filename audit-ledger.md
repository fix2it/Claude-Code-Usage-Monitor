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
- F6 | S1 | src/window.rs:2240-2418 | `sync_surface_windows_with` (~180) | proposal: split primary recovery / mirror reconcile / desktop-surface reconcile | fixed (refactor/6-window-split; surfaces.rs: recreate_primary_surface / replace_destroyed_mirrors / reconcile_mirror_count / reconcile_desktop_surfaces)
- F7 | S1 | src/window.rs:2597-2988 | `run` (~390) | proposal: split instance setup, state init, loop start | fixed (refactor/6-window-split; startup.rs: acquire_single_instance / load_startup_theme / initial_app_state / show_initial_widget / start_timers_and_workers)
- F8 | S1 | src/window.rs:3011-3219 | `render_layered_impl` (~210) | proposal: frame prep / hover bounds / per-surface present | fixed (refactor/6-window-split; render.rs: frame_input / refresh_tile_rects / render_surface)
- F9 | S1 | src/window.rs:3370-3653 | `do_poll_once` (~280) | proposal: progress publish / success apply / failure handling | fixed (refactor/6-window-split; poll.rs: PollRequest / publish_poll_progress / apply_poll_success / apply_poll_failure)
- F10 | S1 | src/window.rs:4498-4601 | `reload_external_settings` (~100) | proposal: state apply vs refresh actions | fixed (refactor/6-window-split; window.rs: apply_external_settings / reset_hidden_tiles)
- F11 | S1 | src/window/message_loop.rs:32-1377 | `wnd_proc` (~1350 lines) | proposal: per-message handler fns (timers, settings commands, lifecycle) | fixed (refactor/6-window-split; wnd_proc 1140 -> 152 lines, on_* handlers; WM_COMMAND in menu_commands.rs)
- F12 | S1 | src/theme_engine.rs:2227-2435 | `display_metrics` (~210) | proposal: per-provider metric builder | fixed (refactor/7-theme-engine; theme_metrics.rs: measured_lines / LineAdvances::measure / measured_display_metrics / provider_marker_column)
- F13 | S1 | src/theme_engine.rs:2675-3167 | `from_usage_with_runtime` (~490) | proposal: split runtime / provider / pool binding population | fixed (refactor/5-pool-table: pool bindings; refactor/7-theme-engine: runtime/provider population — theme_data_context.rs: insert_environment / insert_i18n / insert_clock / insert_provider_flags / insert_display_flags / insert_antigravity_display / insert_hover_states / insert_usage / insert_split_pools)
- F14 | S1 | src/theme_engine.rs:3238-3558 | `insert_provider` (~320) | proposal: quota-window, reset-time, provider-specific helpers | fixed (refactor/7-theme-engine; theme_data_context.rs: insert_window_labels / quota_windows + insert_window / insert_credits_and_headline)
- F15 | S1 | src/theme_engine/theme_rendering.rs:98-265 | `render_theme_surface_with_metrics_at_scale` (~170) | proposal: setup / background / opacity pass | fixed (refactor/7-theme-engine; theme_rendering.rs: paint_floating_card / root_layer / surface_visibility / fade_pixels)
- F16 | S1 | src/theme_engine/theme_rendering.rs:893-1143 | `resolve_geometry` (~250) | proposal: parent resolve / expression eval / transform | fixed (refactor/7-theme-engine; theme_rendering.rs: layer_renders / root_geometry / place_in_parent / layout_cursor / managed_position / anchored_position / rotate_into_parent)
- F19 | S1 | src/window/window_context_menu.rs:237 | every right-click enumerates, reads, parses and validates the menu directory on the UI thread (reviewer said L4; orchestrator: local small files, click-only → L5) | proposal: cache parsed menus, invalidate on file change | fixed (refactor/2: right-click uses resolve_context_menu_cached, keyed by reference + directory size/mtime fingerprint; errors not cached; known limit: an edit that keeps size and mtime is not seen)
- F20 | S1 | src/theme_engine/theme_rendering.rs:296 | per-object pixel buffer allocated and cleared each frame | proposal: reuse scratch buffers / composite directly | fixed (refactor/3-render-hot-path)
- F21 | S1 | src/theme_engine/theme_rendering.rs:1435-1538 | `render_text_mask` (~100) creates/destroys DC, bitmap, font per text object per frame | proposal: cache fonts and mask resources by params; split setup vs compositing | fixed (refactor/3-render-hot-path)
Duplication:
- F22 | S1 | src/theme_engine.rs:3051 | split-pool binding prep duplicated for Claude and Codex (clear fields, copy namespaces, hover bindings; double map scan) | proposal: one shared pool-binding helper, single pass | fixed (refactor/5-pool-table): bind_account_pools, one retain pass per map
More long functions:
- F23 | S1 | src/window.rs:1779-1882 | `begin_update_check` | proposal: extract result handling / completion notify | fixed (refactor/6-window-split; update_flow.rs: mark_update_check_started / finish_update_check / record_update_check)
- F24 | S1 | src/window/positioning.rs:277-382 | `position_at_taskbar` | proposal: taskbar selection vs placement apply | fixed (refactor/8-long-functions)
- F25 | S1 | src/window/positioning.rs:516-646 | `position_custom_theme_internal` | proposal: placement resolve vs positioning | fixed (refactor/8-long-functions)
- F26 | S1 | src/theme_engine.rs:469-579 | `parse_mouse_actions` | proposal: per-action-kind parsers | fixed (refactor/7-theme-engine; theme_mouse.rs: parse_action_statement + per-kind parse_*_action)
- F27 | S1 | src/theme_engine.rs:3995-4117 | `execute_mouse_actions_with_metrics` | proposal: per-action-kind executors | fixed (refactor/7-theme-engine; theme_mouse.rs: MouseActionExecutionContext::run / set_property / reset_property / adjust_property / toggle_property)
- F28 | S1 | src/theme_engine.rs:4414-4566 | `validate_scene_object` | proposal: common vs content-specific validation | fixed (refactor/7-theme-engine; theme_validation.rs: ObjectValidator (background / content))
- F29 | S1 | src/theme_engine/theme_rendering.rs:1227-1330 | `render_object_content` | proposal: progress / text branches | fixed (refactor/7-theme-engine; theme_rendering.rs: render_progress / render_text)
- F30 | S1 | src/window/window_context_menu.rs:224-365 | `show_context_menu_document` | proposal: build / track popup / dispatch | fixed (refactor/8-long-functions)
- F31 | S1 | src/window/window_context_menu.rs:481-618 | `context_menu_checked_with_origin` | proposal: per-action-family check state | fixed (refactor/8-long-functions)
- F32 | S1 | src/context_menu.rs:369-574 | `classic_context_menu` | proposal: provider and settings menu builders | fixed (refactor/8-long-functions)
- F33 | S1 | src/context_menu.rs:824-953 | test `classic_provider_children_follow_their_parents` (L6: test shape) | proposal: split into focused cases | fixed (L6 test split: 4 focused tests + shared helpers)
UI-thread blocking (round 3):
- F35 | S1/M1 | src/window.rs:1172 | `save_state_settings` loads and writes settings file while holding the global state lock (called from menu actions) | proposal: snapshot under lock, do file I/O after release | fixed (refactor/2: file I/O outside STATE; SETTINGS_IO mutex serializes saves, lock order SETTINGS_IO → STATE)
- F36 | S1/M6 | src/native_interop.rs:435-541 (484,493) | `find_desktop_host` (~105 lines) can block UI thread up to 2×1 s in `SendMessageTimeoutW` when cached host is invalid | proposal: background/bounded rediscovery; split cached-host validation vs discovery | partly fixed (refactor/2: SMTO_ABORTIFHUNG (hung Explorer → no wait) + second WorkerW request skipped after a failed first one (timeout → 1 s instead of 2 s). Still up to ~2 s when the first request succeeds slowly and the second times out; a shared deadline would skip a request that used to succeed. Background rediscovery not done: three callers need the host synchronously, moving it off-thread changes placement timing — visible behavior)
More long functions (round 3):
- F37 | S1 | src/poller/accounts.rs:108-299 | `poll_accounts_with_progress` | proposal: targets / dispatch / aggregate | fixed (refactor/8-long-functions)
- F38 | M2 | src/poller/qoder.rs:318-429 | `usage_data` | proposal: credit pools vs window select | fixed (refactor/8-long-functions)
- F39 | M2 | src/poller/qoder.rs:539-660 | `run_usage` | proposal: pseudo-console setup / launch / collect | fixed (refactor/8-long-functions)
- F40 | S1 | src/poller/claude/limits.rs:19-137 | `parse` | proposal: array parse / legacy buckets / collisions | fixed (refactor/8-long-functions)
- F41 | M3 | src/theme_engine/theme_expression.rs:490-618 | `format_usage_line` (runs during render, allocates lookup keys) | proposal: provider normalize + availability helpers; reuse keys | fixed (refactor/3-render-hot-path)
- F42 | M4 | src/studio_app/studio_core.rs:342-442 | `shell` | proposal: nav vs page content | fixed (refactor/8-long-functions)
- F43 | M4 | src/studio_app/studio_diagnostics.rs:113-218 | `diagnostics_page` | proposal: recording controls vs log view | fixed (refactor/8-long-functions)
- F44 | M4 | src/studio_app/studio_settings.rs:5-203 | `settings_page` | proposal: per-group render helpers | fixed (refactor/8-long-functions)
- F45 | M4 | src/studio_app/studio_settings.rs:206-311 | `account_settings` | proposal: selection vs editing | fixed (refactor/8-long-functions)
Round 4:
- F48 | S1 | src/theme_engine/theme_rendering.rs:117 | each surface render builds the full usage `DataContext` twice (incl. formatted dynamic limit keys) | proposal: build once per frame, patch canvas-size values | fixed (refactor/3-render-hot-path)
- F49 | M2 | src/poller/copilot.rs:331 (+ cursor, grok, opencode) | env-value normalization duplicated in 4 pollers; secret-signature logic duplicated in copilot and cursor | proposal: shared helpers in poller.rs | fixed (refactor/5-pool-table): non_empty_environment and secret_signature in poller.rs
- F50 | M5 | src/updater.rs:48 | `InstallChannel::Winget` dead in production (`allow(dead_code)`), plus its update path | proposal: remove or gate under cfg(test) | fixed (refactor/4-dead-code)
- F51 | M4 | src/studio_app/warp.rs:425-564 | test `warp_renders_partial_textures_at_zoom_and_frees_after_paint` 140 lines (L6: test shape) | proposal: split | fixed (L6 test split: WarpTarget harness + 2 focused tests)
Round 5:
- F53 | S1 | src/window.rs:4328 | hover setters for Claude, Codex, Antigravity, OpenCode, Qoder, agy external repeat state-change + animation + timer logic | proposal: one generic hover-animation helper with per-provider eligibility | fixed (refactor/5-pool-table): set_tile_hover + tile_hover_animates over the Tile table
- F54 | S1 | src/window.rs:1490-1588 | `sync_tray_icon` (~100) mixes state collection, surface filter, render, shell sync | proposal: extract themed-icon build | fixed (refactor/6-window-split; tray_status.rs: TrayFrame (surface_renders, render_icon))
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

---

# Pool-split audit (2026-11-16)

- HEAD: 45cdadbe8f99ee34fc49867530eaab9989754333 (refactor/3-render-hot-path), branch audit/pool-split
- Levels: 1–6. Surface: only the pool-split path (pools on one widget flags for Claude/Codex/agy, saved positions, mirrors, agy pools A/O + external models, runtime toggles, data/width/hover/tooltips per split tile, custom themes).
- Reviewer: Codex CLI `codex_x exec -m gpt-6.1-terra`, read-only, effort high (confirmed from log header, 4 rounds). Orchestrator verified every finding against code; severities adjusted where noted.
- Working tree note: an uncommitted edit in src/window.rs (pixel-buffer recycle) from another session was present; out of scope, not committed.

## Coverage checklist (pool-split path)
- [x] P1 flags: defaults, persistence, legacy load, menu conditions, toggle handler
- [x] P2 ThemeRuntime builders, widget_count family
- [x] P3 per-pool metrics/bindings, agy A/O + external rows
- [x] P4 render of split tiles, hover rects, tooltips
- [~] P5 placement/width after split (split tiles have no own saved positions: placement is per surface)
- [x] P6 mirrors with split
- [x] P7 runtime toggles, reload, poll-driven layout change
- [x] P8 custom themes: capability check, fallback
- [~] P9 tests (gaps only, read selectively)

## Findings

### Level 4 — Medium
- F58 | P4 | src/window/widget_tooltip.rs:112, src/window.rs:1337-1400 `usage_summary_lines` | usage-summary tooltip shows only the selected pool per provider (`data.get(provider)`); Claude X, Codex X, agy O quotas are absent, also when they have their own split tile | user hovers a split X/O tile and sees no numbers for it | proposal: add per-account lines for enabled side pools | fixed (fix/pool-split-l4: widget tooltip lists Claude X, Codex X, agy O + its Claude/GPT pool; tray unchanged)
- F62 | P3/P7 | src/theme_engine.rs:2930-2950 (Claude; same pattern for Codex) | when the `claude_x`/`codex_x` account entry is missing (pool just enabled, before its poll) the split namespace is filled from the selected provider reading minus limits | X tile briefly shows the first pool's session/weekly numbers and reset time | proposal: empty/"--" binding for a missing account | fixed (fix/pool-split-l4: missing claude_x/codex_x account binds no reading, tile shows "--")
- F69 | P7 | src/poller.rs:147 `carry_forward_failures` (with `merge_poll_results`) | a provider without account profiles that fails while another provider answers gets its previous reading carried in silently; its error is dropped, so the tray says "updated HH:MM" and no tooltip names the failure (found in round 3 of the tray-tooltip review) | proposal: keep the provider's poll error next to the carried reading and list it with the other failures | fixed (refactor/5-pool-table): AppUsageData::provider_errors kept with the stale reading (also through a whole failed poll); tray lists it
- F71 | P7 | src/window/message_loop.rs `on_left_button_up` (drop off the taskbar) | with "Lock in taskbar" on, a drag from the taskbar to the desktop took the locked branch: re-embed as a taskbar child + `position_at_taskbar` from a topmost popup instead of a floating drop; the widget ended up at the bottom of the z-order (under windows and desktop) and vanished; old bug (builds s8, s9-6-menufix), found by the owner | owner decision: a drop on the desktop releases the lock | fixed (fix/pinned-drag-to-desktop): `release_taskbar_lock_for_desktop_drop` turns `lock_taskbar` off only when the lock applies (an already-floating widget keeps it), the drop goes the floating path, settings saved, menu check follows; test `desktop_drop_releases_taskbar_lock`

### Level 5 — Minor
- F56 | P4/P6 | src/window/mouse.rs:722 `tile_hover_at`, src/window.rs:3076 rect cache | tile hit-testing and tile rects only for surface 0 | split (and other) tiles on a mirror surface of a custom theme get no hover readings | fixed (refactor/5-pool-table): tile rects cached per surface, hit-test uses the hovered surface
- F63 | P4/P7 | src/window/message_loop.rs:191 `WM_APP_USAGE_UPDATED` | poll that changes the layout (agy O appears/disappears) rerenders without `revalidate_mouse_hover`, unlike menu toggles | stationary cursor keeps the old tile hover until the next mouse move | fixed (fix/pool-split-l4: WM_APP_USAGE_UPDATED posts request_hover_revalidation after render)
- F66 | P6/P8 | src/theme_engine.rs:4125 `supports_pool_splitting` | capability checked on the primary surface only; a mirror surface with the combined layout gets split bindings | custom theme mirror can lose the second pool's display | fixed (claude/audit-pool-split-08fda4): `runtime_with_geometry` narrows split support per surface (a mirror cannot widen the primary's); `DisplayMetrics` records the support it was measured for and `metrics_for_surface` re-measures when a surface differs (render, mouse, tooltip, menu, tray, watchdog, mouse actions); tests `combined_mirror_keeps_pools_together_while_the_primary_splits`, `mirror_narrows_split_support_per_provider`
- F70 | P9 | src/window.rs `tile_hover_arms_a_200ms_animation_without_rendering` | test reads its own source via `include_str!` and splits on `"\n}\n"` and a string literal with an embedded newline | with a CRLF checkout (core.autocrlf=true, no .gitattributes) the split finds nothing and the test panics (found after part 6) | fixed (refactor/6-window-split): source normalized `\r\n` -> `\n`, split marker written as `"\nfn set_tile_hover("`; other source-reading tests (mouse.rs, message_loop.rs) checked: single-line `contains` or CR-tolerant scanner
Duplication (input to part 5):
- F57 | P7 | src/window/message_loop.rs:996-1104 (Claude) and 1105-~1210 (Codex) | pool enable/disable handlers duplicated (~110 lines each: profile creation, last-pool guard, provider state, refresh) | proposal: one `toggle_pool(provider, pool)` with provider defaults | fixed (refactor/5-pool-table): toggle_account_pool
- F59 | P1 | src/context_menu.rs:434-480 | three "pools on one widget" menu entries repeat the same provider+two-pools eligibility | proposal: parameterized entry builder | fixed (refactor/5-pool-table): pools_one_widget menu entry builder
- F64 | P2 | src/theme_engine.rs:1633-1645 | `claude_separate` / `codex_separate` identical predicates (and `with_pool_layouts` + `with_codex_pool_layout` split API at 1439-1448) | proposal: one `pool_separate(provider)`, one builder taking all three flags | fixed (refactor/5-pool-table): pool_separate(provider), with_pool_layouts(claude, codex, agy)
- F67 | P3/P4 | src/window.rs:3082-3118, src/window.rs:182-184, theme_engine.rs:1186-1189 | per-provider tile rects + `split_hovers[4]` + string id list kept as parallel hand-written arrays (ids order must match bounds indices) | proposal: table of (provider, pool, object id) driving rects, hovers, bindings | fixed (refactor/5-pool-table): SplitPool + Tile tables drive rects, hovers, bindings
Long functions on this path (already in ledger, relevant for part 5): F6 `sync_surface_windows_with`, F8 `render_layered_impl`, F11 `wnd_proc`, F12 `display_metrics`, F13 `from_usage_with_runtime` (split namespace copies 2900-3170), F14 `insert_provider`.

### Level 6 — Preferences
- F60 | P7 | src/window/message_loop.rs:884-905 | flag selection by `if id == …` chain | proposal: map command → `&mut bool` accessor | fixed (refactor/5-pool-table): menu command -> provider -> pools_one_widget_mut
- F61 | P9 | src/window/widget_tooltip.rs:320 tests | no tooltip test with distinct main/side pool values (would catch F58) | fixed (fix/pool-split-l4: widget_summary_lists_side_pools_with_their_own_numbers, unpolled_second_pool_never_shows_first_pool_reading)
- F65 | P9 | src/theme_engine/tests.rs:6599 (~150 lines), 6751 (~135 lines) | long split tests | proposal: split into focused tests | fixed (L6 test split: 10 focused tests + shared fixtures)
- F68 | P8 | src/theme_engine.rs:4127 | split capability requires all four providers' split objects; a custom theme with only Claude split cannot split anything (documented design) | proposal: per-provider capability, plus tests for partial/mixed themes (placement_regression_tests.rs:120) | fixed (claude/audit-pool-split-08fda4, owner approved the rule change): `PoolSplitSupport` per provider — Claude `claude_x-provider`, Codex `codex_x-provider`, Antigravity both `antigravity_o-provider` and `agy_external_o-provider`; new keys `display.{claude,codex,antigravity}_split_supported`, legacy `display.pool_split_supported` = all providers; "Pools on one widget" menu item per provider; tests `partial_theme_splits_only_the_providers_it_draws`, `mixed_theme_needs_both_antigravity_tiles`

## Validation (pool-split audit)
- `cargo clippy --all-targets` — exit 0, 7 warnings (bin) + 13 (tests).
- `cargo test` — 737 passed, 0 failed, 12 ignored (75.8 s).

## Score table (pool-split audit)

| Раунд | Охват | Модель и усилие | Оценка | Новые / в ledger | Самая серьёзная новая | Почему дальше / стоп |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | весь путь P1–P9 | gpt-6.1-terra, high | 7.5 | 2 / 2 | мелкая (нет наведения на зеркале) | новые находки |
| 2 | подсказки, ширина, переключения | gpt-6.1-terra, high | 7 | 4 / 6 | средняя (F58 подсказка без второго пула) | новые находки |
| 3 | данные пулов, позиции, тесты | gpt-6.1-terra, high | 6.5 | 4 / 10 | средняя (F62 чужие цифры на плитке X) | новые находки |
| 4 | позиции, зеркала, кастомные темы | gpt-6.1-terra, high | 7 | 2 / 12 (+F67 оркестратор) | мелкая после пересмотра (F66) | стоп: находки ещё идут, но мельчают; не принято |

## Result (pool-split audit)
Stopped incomplete after 4 passes (still new findings, P5/P9 read selectively). 13 new (F56–F68): L4 ×2 (F58, F62), L5 ×7 (F56, F57, F59, F63, F64, F66, F67), L6 ×4 (F60, F61, F65, F68). No L1–L3. Duplicates dropped: 1 (mirror hover in round 2).

## Flaky tests (2026-11-17, main e0e34df)

- F72 | L4 | src/window/host_geometry.rs:12 (THEME_HOST_GEOMETRY) | test-only race: `shell_query_can_reenter_state_and_read_previous_geometry` writes fake 3840×92 geometry into the process-wide host cache while parallel layout tests read it; `window::placement_regression_tests::absent_saved_monitor_preserves_the_choice_while_using_host_fallbacks` computes two placements and got host 292×104 (empty cache) vs 3840×92. Reproduced 1 of 15 runs with `--test-threads=64` plus a parallel build; 0 of 20 plain runs | fixed (claude/fix-flaky-tests-ed41c1): cache + refresh guard behind `host_geometry_cache()`; release build keeps one static, test builds get one per test thread (same pattern as `app_data_directory`). App behavior unchanged.
- Watch list (not reproduced, timing-based asserts): `poller::tests::concurrent_polling_is_bounded_and_preserves_results` and `poller::accounts` peak test (`peak > 1` relies on 20 ms sleeps overlapping); `app_settings::replace_retries_while_the_destination_is_briefly_held` (40 ms hold vs 375 ms retry budget). Open.

## OpenCode console cookie (2026-11-17, main 2804373)

- F73 | L3 | src/poller/opencode.rs `fetch_go_status` | a bare `authCookie` value was always wrapped as `auth=<value>`; the console moved to `__Host-console_session=st_<uuid>` (CodeZeno/Claude-Code-Usage-Monitor#195). Observed: `__Host-console_session=st_…` 200, `auth=; __Host-console_session=st_…` 200, `auth=st_…` 401, bare `st_…` 401; old `Fe26.2**…` silently 401 | OpenCode widget shows sign-in error with a valid session value | fixed (claude/fix-opencode-cookie-346b3d): bare value tried as `__Host-console_session` first, `auth` on 401, working form kept in memory (one request per steady poll); named cookies sent as-is; Bearer follows the sent cookie (`auth=` only); each 401 logs the cookie form (name only) plus a `Fe26.2**` hint; README/USER_GUIDE name `__Host-console_session=st_…`; stub-server tests for every form

## Pool split per surface (2026-11-18, F66/F68 follow-up)

- F74 | L4 | src/theme_engine/theme_metrics.rs `display_metrics` | metrics always measure the primary surface's text (`theme.surfaces.first()`); mirrors and tray surfaces reuse them, so a custom mirror with different text or fonts gets the primary's column widths (pre-existing design; surfaced by Codex round 3 of the F66/F68 fix) | proposal: measure the target surface and key the cache by it, plus a test with a differently styled mirror | open
