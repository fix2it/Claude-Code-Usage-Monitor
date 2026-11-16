//! Context-menu (WM_COMMAND) actions.

use super::*;

pub(super) unsafe fn on_command(hwnd: HWND, wparam: WPARAM) -> LRESULT {
    let id = wparam.0 as u16;
    match id {
        IDM_DASHBOARD => {
            crate::dashboard::show(controller_for(hwnd));
        }
        1 => {
            {
                let mut state = lock_state();
                if let Some(s) = state.as_mut() {
                    s.force_notify_auth_error = true;
                }
            }
            render_layered();
            request_poll(hwnd);
        }
        IDM_VERSION_ACTION => perform_update_action(hwnd),
        2 => {
            let _ = PostMessageW(Some(controller_for(hwnd)), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        IDM_START_WITH_WINDOWS => {
            set_startup_enabled(!is_startup_enabled());
        }
        IDM_LOCK_TASKBAR => {
            toggle_taskbar_lock(hwnd);
        }
        IDM_SHOW_PACE => {
            {
                let mut state = lock_state();
                if let Some(s) = state.as_mut() {
                    s.show_pace = !s.show_pace;
                    invalidate_metrics(s);
                }
            }
            save_state_settings();
            render_layered();
            sync_tray_icon(hwnd);
        }
        IDM_SHOW_FABLE => {
            {
                let mut state = lock_state();
                if let Some(s) = state.as_mut() {
                    super::toggle_show_fable(s);
                }
            }
            save_state_settings();
            render_layered();
            // The Fable column changes the root width: a right-pinned
            // window moves its left edge, so Win32 sends no
            // WM_MOUSELEAVE for a cursor it moved out from under.
            super::mouse::revalidate_mouse_hover(hwnd);
            sync_tray_icon(hwnd);
        }
        IDM_SHOW_FABLE_X => {
            {
                let mut state = lock_state();
                if let Some(s) = state.as_mut() {
                    super::toggle_show_fable_x(s);
                }
            }
            save_state_settings();
            render_layered();
            // The Fable column changes the root width: a right-pinned
            // window moves its left edge, so Win32 sends no
            // WM_MOUSELEAVE for a cursor it moved out from under.
            super::mouse::revalidate_mouse_hover(hwnd);
            sync_tray_icon(hwnd);
        }
        IDM_CLAUDE_POOLS_ONE_WIDGET
        | IDM_ANTIGRAVITY_POOLS_ONE_WIDGET
        | IDM_CODEX_POOLS_ONE_WIDGET => {
            let provider = match id {
                IDM_CLAUDE_POOLS_ONE_WIDGET => ProviderId::Claude,
                IDM_CODEX_POOLS_ONE_WIDGET => ProviderId::Codex,
                _ => ProviderId::Antigravity,
            };
            {
                let mut state = lock_state();
                if let Some(s) = state.as_mut() {
                    if let Some(one_widget) = s.pools_one_widget_mut(provider) {
                        *one_widget = !*one_widget;
                    }
                    super::invalidate_metrics(s);
                }
            }
            save_state_settings();
            position_at_taskbar();
            render_layered();
            super::mouse::revalidate_mouse_hover(hwnd);
            sync_tray_icon(hwnd);
        }
        IDM_FREQ_1MIN | IDM_FREQ_5MIN | IDM_FREQ_15MIN | IDM_FREQ_1HOUR => {
            let new_interval = match id {
                IDM_FREQ_1MIN => POLL_1_MIN,
                IDM_FREQ_5MIN => POLL_5_MIN,
                IDM_FREQ_15MIN => POLL_15_MIN,
                IDM_FREQ_1HOUR => POLL_1_HOUR,
                _ => POLL_15_MIN,
            };
            {
                let mut state = lock_state();
                if let Some(s) = state.as_mut() {
                    s.poll_interval_ms = new_interval;
                }
            }
            save_state_settings();
            // Reset the poll timer with the new interval
            set_controller_timer(Some(hwnd), TIMER_POLL, new_interval, None);
        }
        IDM_PROVIDER_ANTIGRAVITY_GEMINI | IDM_PROVIDER_ANTIGRAVITY_EXTERNAL => {
            let mut should_reposition = false;
            {
                let mut state = lock_state();
                if let Some(s) = state.as_mut() {
                    should_reposition =
                        super::toggle_antigravity_models(s, id == IDM_PROVIDER_ANTIGRAVITY_GEMINI);
                    s.stop_crossfade_timer_if_idle(hwnd);
                }
            }
            save_state_settings();
            if should_reposition {
                position_at_taskbar();
            }
            render_layered();
            sync_tray_icon(hwnd);
            request_poll(hwnd);
        }
        IDM_PROVIDER_ANTIGRAVITY_EXTERNAL_X => {
            {
                let mut state = lock_state();
                if let Some(s) = state.as_mut() {
                    super::toggle_antigravity_external_x(s);
                }
            }
            save_state_settings();
            // The external-models widget can appear or go away.
            position_at_taskbar();
            render_layered();
            super::mouse::revalidate_mouse_hover(hwnd);
            sync_tray_icon(hwnd);
        }
        IDM_PROVIDER_ANTIGRAVITY_A => {
            let mut should_reposition = false;
            {
                let mut state = lock_state();
                if let Some(s) = state.as_mut() {
                    should_reposition = super::toggle_antigravity_a(s);
                    // Pool A off leaves pool O alone in one row.
                    super::invalidate_hover_tile_rects(s);
                    invalidate_metrics(s);
                }
            }
            save_state_settings();
            if should_reposition {
                position_at_taskbar();
            }
            render_layered();
            super::mouse::revalidate_mouse_hover(hwnd);
            sync_tray_icon(hwnd);
            request_poll(hwnd);
        }
        IDM_PROVIDER_ANTIGRAVITY_X => {
            {
                let mut state = lock_state();
                if let Some(s) = state.as_mut() {
                    super::toggle_antigravity_pool_o(s);
                    // Pool O adds a marker column and a second row.
                    super::invalidate_hover_tile_rects(s);
                    invalidate_metrics(s);
                }
            }
            save_state_settings();
            position_at_taskbar();
            render_layered();
            super::mouse::revalidate_mouse_hover(hwnd);
            sync_tray_icon(hwnd);
            request_poll(hwnd);
        }
        IDM_PROVIDER_CLAUDE_MAIN
        | IDM_PROVIDER_CLAUDE_X
        | IDM_PROVIDER_CODEX_MAIN
        | IDM_PROVIDER_CODEX_X => {
            let (provider, main) = match id {
                IDM_PROVIDER_CLAUDE_MAIN => (ProviderId::Claude, true),
                IDM_PROVIDER_CLAUDE_X => (ProviderId::Claude, false),
                IDM_PROVIDER_CODEX_MAIN => (ProviderId::Codex, true),
                _ => (ProviderId::Codex, false),
            };
            let mut should_reposition = false;
            {
                let mut state = lock_state();
                if let Some(s) = state.as_mut() {
                    should_reposition = super::toggle_account_pool(s, provider, main);
                    s.stop_crossfade_timer_if_idle(hwnd);
                }
            }
            save_state_settings();
            if should_reposition {
                position_at_taskbar();
            }
            render_layered();
            super::mouse::revalidate_mouse_hover(hwnd);
            sync_tray_icon(hwnd);
            request_poll(hwnd);
        }
        id if ProviderId::from_native_menu_command_id(id).is_some() => {
            {
                let mut state = lock_state();
                if let Some(s) = state.as_mut() {
                    let provider = ProviderId::from_native_menu_command_id(id)
                        .expect("provider menu command was matched above");
                    let changed = s.providers.toggle(provider);
                    // Tile widths depend on the provider count.
                    invalidate_metrics(s);
                    for (provider, tile) in [
                        (ProviderId::Claude, Tile::Claude),
                        (ProviderId::Codex, Tile::Codex),
                    ] {
                        if !s.providers.contains(provider) {
                            s.tiles.reset(tile);
                        }
                    }
                    if provider == ProviderId::Antigravity && changed {
                        let enabled = s.providers.contains(ProviderId::Antigravity);
                        // The child External choice survives parent toggles.
                        s.show_antigravity_gemini = enabled;
                        if !enabled {
                            s.tiles.reset(Tile::Antigravity);
                        }
                    }
                    s.stop_crossfade_timer_if_idle(hwnd);
                }
            }
            save_state_settings();
            position_at_taskbar();
            render_layered();
            sync_tray_icon(hwnd);
            request_poll(hwnd);
        }
        id if id == IDM_LANG_SYSTEM || language_from_menu_command_id(id).is_some() => {
            let language_override = if id == IDM_LANG_SYSTEM {
                None
            } else {
                language_from_menu_command_id(id)
            };
            {
                let mut state = lock_state();
                if let Some(s) = state.as_mut() {
                    apply_language_to_state(s, language_override);
                }
            }
            save_state_settings();
            render_layered();
        }
        _ => {}
    }
    LRESULT(0)
}
