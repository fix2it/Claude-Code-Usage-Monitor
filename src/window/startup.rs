//! Process startup: single instance, initial state, windows and the message loop.

use super::*;

/// Command-line switches read once at startup.
struct StartupArgs {
    open_dashboard: bool,
    allow_multiple: bool,
    no_poll: bool,
}

impl StartupArgs {
    fn from_env() -> Self {
        let run_args: Vec<String> = std::env::args().collect();
        let has = |flag: &str| run_args.iter().any(|argument| argument == flag);
        Self {
            open_dashboard: has("--dashboard"),
            allow_multiple: has("--allow-multiple"),
            no_poll: has("--no-poll"),
        }
    }
}

/// The active theme chosen at startup and the file it came from.
struct StartupTheme {
    path: Option<PathBuf>,
    theme: Option<ThemeDocument>,
}

pub fn run() {
    let args = StartupArgs::from_env();
    unsafe {
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CURRENT_DPI.store(GetDpiForSystem(), Ordering::Relaxed);
    }
    diagnose::log("window::run started");

    let Some(_mutex) = acquire_single_instance(&args) else {
        return;
    };

    let class_name = native_interop::wide_str("ClaudeCodeUsageMonitor");

    unsafe {
        let hinstance = GetModuleHandleW(PCWSTR::null()).unwrap();
        let (large_icon, small_icon) = tray_icon::load_app_icons();
        register_controller_class(&class_name, hinstance, large_icon, small_icon);

        let mut settings = load_settings();
        let startup_theme = load_startup_theme(&mut settings);
        let language_override = settings.language.as_deref().and_then(LanguageId::from_code);
        let language = localization::resolve_language(language_override);
        let install_channel = updater::current_install_channel();

        refresh_theme_host_geometry();

        // Hidden top-level lifetime owner: receives shell broadcasts and is
        // never parented into Explorer. The public class stays stable for IPC.
        let title = native_interop::wide_str(language.strings().window_title);
        let controller = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            PCWSTR::from_raw(class_name.as_ptr()),
            PCWSTR::from_raw(title.as_ptr()),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(HINSTANCE(hinstance.0)),
            None,
        )
        .expect("create application controller");
        CONTROLLER_HWND.store(SendHwnd::from_hwnd(controller).0, Ordering::Relaxed);
        let hwnd = lifecycle::create_primary_surface();
        if hwnd.is_invalid() {
            let _ = DestroyWindow(controller);
            return;
        }
        set_window_icons(hwnd, large_icon, small_icon);

        diagnose::log(format!("main window created hwnd={:?}", hwnd));
        diagnose::log(format!("controller window created hwnd={controller:?}"));

        *lock_state() = Some(initial_app_state(
            hwnd,
            &settings,
            language_override,
            language,
            install_channel,
            startup_theme,
        ));

        if let Some(state) = lock_state().as_ref() {
            publish_update_status(state);
        }
        if let Err(error) = crate::dashboard::start_request_listener(controller) {
            diagnose::log_error("dashboard request listener failed", error);
        }

        show_initial_widget(hwnd, &args);
        start_timers_and_workers(hwnd, &args);

        // Initial theme check
        check_theme_change();

        run_message_loop();
    }
}

/// Single-instance guard: `None` means this process must exit.
///
/// Uses the local namespace so other users' desktop/RDP sessions remain
/// independent. When relaunched after a UI hang (ENV_RELAUNCH set), waits for
/// the previous instance to release the mutex, then takes over.
fn acquire_single_instance(args: &StartupArgs) -> Option<HANDLE> {
    let is_relaunch = std::env::var(ENV_RELAUNCH).is_ok();
    let mutex_name = native_interop::wide_str(&if args.allow_multiple {
        format!("Local\\ClaudeCodeUsageMonitor-{}", std::process::id())
    } else {
        "Local\\ClaudeCodeUsageMonitor".to_string()
    });
    unsafe {
        let handle = CreateMutexW(None, true, PCWSTR::from_raw(mutex_name.as_ptr()));
        match handle {
            Ok(h) => {
                if GetLastError() == ERROR_ALREADY_EXISTS {
                    if is_relaunch {
                        diagnose::log("relaunch: waiting for previous instance to exit");
                        let wait_result = WaitForSingleObject(h, 10_000);
                        if wait_result != WAIT_OBJECT_0 && wait_result != WAIT_ABANDONED {
                            diagnose::log(format!(
                                "startup aborted: previous instance did not exit cleanly ({wait_result:?})"
                            ));
                            return None;
                        }
                    } else {
                        if args.open_dashboard {
                            if let Err(error) = crate::dashboard::request_from_existing_monitor() {
                                crate::dashboard::report_launch_failure(HWND::default(), &error);
                            }
                        }
                        diagnose::log("startup aborted: another instance is already running");
                        return None;
                    }
                }
                Some(h)
            }
            Err(error) => {
                diagnose::log_error(
                    "startup aborted: unable to create single-instance mutex",
                    error,
                );
                None
            }
        }
    }
}

unsafe fn register_controller_class(
    class_name: &[u16],
    hinstance: HMODULE,
    large_icon: HICON,
    small_icon: HICON,
) {
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
        lpfnWndProc: Some(lifecycle::controller_wnd_proc),
        hInstance: HINSTANCE(hinstance.0),
        hIcon: large_icon,
        hIconSm: small_icon,
        hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
        hbrBackground: HBRUSH(std::ptr::null_mut()),
        lpszClassName: PCWSTR::from_raw(class_name.as_ptr()),
        ..Default::default()
    };

    let atom = RegisterClassExW(&wc);
    if atom == 0 {
        diagnose::log("RegisterClassExW returned 0");
    }
}

/// Load the configured theme, migrate v1.4.9 placement/visibility settings
/// into a theme copy, fall back to the starter theme, and persist the choice.
fn load_startup_theme(settings: &mut SettingsFile) -> StartupTheme {
    let starter_theme_path = theme_engine::ensure_starter_theme().ok();
    let mut configured_theme_path = settings.active_theme_path.as_deref().map(PathBuf::from);
    let mut configured_theme = configured_theme_path
        .as_deref()
        .and_then(theme_engine::load_selected_theme)
        .filter(|theme| !theme.is_obsolete_studio_starter());
    let legacy_placement = settings.legacy_placement();
    let legacy_visibility = settings.legacy_widget_visibility();
    if legacy_placement.is_some() || legacy_visibility.is_some() {
        if configured_theme
            .as_ref()
            .is_some_and(|theme| !theme.is_builtin())
        {
            // A user-selected writable theme already owns its presentation.
            // Consume the obsolete settings without replacing that theme.
            settings.consume_legacy_placement();
            settings.consume_legacy_widget_visibility();
            save_settings_or_log(settings, "unable to consume legacy settings");
        } else if legacy_placement.is_some() || legacy_visibility == Some(false) {
            let placement = legacy_placement.map(migrated_theme_placement);
            let migrated =
                ThemeDocument::migrated_from_legacy(placement, legacy_visibility.unwrap_or(true));
            match theme_engine::save_theme(&migrated) {
                Ok(path) => {
                    configured_theme_path = Some(path.clone());
                    configured_theme = Some(migrated);
                    settings.active_theme_path = Some(path.to_string_lossy().into_owned());
                    settings.custom_theme_enabled = true;
                    settings.consume_legacy_placement();
                    settings.consume_legacy_widget_visibility();
                    if let Err(error) = save_settings(settings) {
                        diagnose::log(format!(
                            "migrated theme created but settings cleanup failed: {error}"
                        ));
                    } else {
                        diagnose::log("legacy placement and visibility migrated to Migrated Theme");
                    }
                }
                Err(error) => diagnose::log(format!(
                    "legacy theme migration deferred because the copied theme could not be saved: {error}"
                )),
            }
        } else {
            // An explicitly visible v1.4.9 widget already matches the
            // built-in theme's Render value, so no copy is necessary.
            settings.consume_legacy_widget_visibility();
            save_settings_or_log(settings, "unable to consume legacy visibility");
        }
    }
    let (path, theme) = configured_theme
        .map(|theme| (configured_theme_path, Some(theme)))
        .unwrap_or_else(|| {
            let path = starter_theme_path;
            let theme = path
                .as_deref()
                .and_then(|path| theme_engine::load_theme(path).ok())
                .or_else(|| Some(ThemeDocument::starter()));
            (path, theme)
        });
    if let Some(path) = &path {
        let path = path.to_string_lossy().into_owned();
        if settings.active_theme_path.as_deref() != Some(path.as_str())
            || !settings.custom_theme_enabled
        {
            settings.active_theme_path = Some(path);
            settings.custom_theme_enabled = true;
            save_settings_or_log(settings, "unable to persist active theme");
        }
    }
    StartupTheme { path, theme }
}

unsafe fn set_window_icons(hwnd: HWND, large_icon: HICON, small_icon: HICON) {
    if !large_icon.is_invalid() {
        let _ = SendMessageW(
            hwnd,
            WM_SETICON,
            Some(WPARAM(ICON_BIG as usize)),
            Some(LPARAM(large_icon.0 as isize)),
        );
    }
    if !small_icon.is_invalid() {
        let _ = SendMessageW(
            hwnd,
            WM_SETICON,
            Some(WPARAM(ICON_SMALL as usize)),
            Some(LPARAM(small_icon.0 as isize)),
        );
    }
}

fn initial_app_state(
    hwnd: HWND,
    settings: &SettingsFile,
    language_override: Option<LanguageId>,
    language: LanguageId,
    install_channel: InstallChannel,
    startup_theme: StartupTheme,
) -> AppState {
    let StartupTheme {
        path: active_theme_path,
        theme: active_theme,
    } = startup_theme;
    let providers = settings.enabled_providers();
    let theme_clock_interval = active_theme
        .as_ref()
        .and_then(ThemeDocument::current_time_refresh_interval);
    let tray_theme_uses_current_time = active_theme
        .as_ref()
        .is_some_and(theme_tray_uses_current_time);
    AppState {
        hwnd: SendHwnd::from_hwnd(hwnd),
        taskbar_hwnd: None,
        tray_notify_hwnd: None,
        win_event_hook: None,
        is_dark: theme::is_dark_mode(),
        embedded: false,
        language_override,
        language,
        install_channel,
        providers,
        show_antigravity_gemini: settings.antigravity_gemini_enabled(),
        show_antigravity_external: settings.antigravity_external_enabled(),
        show_antigravity_external_x: settings.antigravity_external_x_enabled(),
        tiles: tiles::Tiles::default(),
        view_mode: settings.view_mode,
        mode_phase: (f64::from(settings.view_mode) - 1.0).clamp(0.0, 2.0),
        widget_modes: widget_modes_from_settings(settings),
        widget_directions: [1; crate::theme_engine::WIDGET_SLOTS],
        widget_animations: [None; crate::theme_engine::WIDGET_SLOTS],
        accounts: settings.accounts.clone(),
        data: None,
        prepared_metrics: None,
        poll_interval_ms: settings.poll_interval_ms,
        retry_count: 0,
        force_notify_auth_error: false,
        auth_error_paused_polling: false,
        auth_watch_mode: poller::CredentialWatchMode::ActiveSource(
            providers.first().unwrap_or_default(),
        ),
        auth_watch_snapshot: Vec::new(),
        last_poll_ok: false,
        last_poll_failure: None,
        last_poll_ok_at: None,
        update_status: UpdateStatus::Idle,
        last_update_check_unix: settings.last_update_check_unix,
        taskbar_index: settings.taskbar_index,
        tray_offset: settings.tray_offset,
        dragging: false,
        pending_drag: false,
        drag_start_cursor: POINT::default(),
        drag_start_origin: POINT::default(),
        drag_start_client_x: 0,
        auto_ejected: false,
        auto_ejected_for_capacity: false,
        auto_ejected_origin: None,
        auto_ejected_host: None,
        lock_taskbar: settings.lock_taskbar,
        is_switching_window_style: false,
        is_snapped: false,
        placement_override: settings.placement_override.clone(),
        floating_card_opacity: settings.floating_card_opacity,
        window_state_timer_active: false,
        custom_theme_enabled: true,
        usage_countdown: settings.usage_countdown,
        use_windows_default_font: settings.use_windows_default_font,
        show_pace: settings.show_pace,
        show_fable: settings.show_fable,
        show_fable_x: settings.show_fable_x.unwrap_or(settings.show_fable),
        claude_pools_one_widget: settings.claude_pools_one_widget,
        codex_pools_one_widget: settings.codex_pools_one_widget,
        antigravity_pools_one_widget: settings.antigravity_pools_one_widget,
        active_theme_path,
        active_theme,
        theme_clock_interval,
        tray_theme_uses_current_time,
        mirror_hwnds: Vec::new(),
        desktop_hwnds: Vec::new(),
        mouse_action_overrides: HashMap::new(),
        hovered_mouse_layer: None,
        pending_mouse_click: None,
        suppress_next_left_up: false,
    }
}

/// Create the surface windows, register the tray icon, position and paint the
/// widget, and open the dashboard when asked.
unsafe fn show_initial_widget(hwnd: HWND, args: &StartupArgs) {
    sync_surface_windows();
    native_interop::make_popup(hwnd, false);

    // Register the persistent application tray icon.
    if !args.no_poll {
        sync_tray_icon(hwnd);
    }

    // Theme surfaces decide whether their windows render.
    position_at_taskbar();
    diagnose::log("window shown");

    // Initial render using the presenter selected by the surface nest.
    schedule_countdown_timer();
    schedule_clock_timer();
    render_layered();

    if args.open_dashboard {
        crate::dashboard::show(controller_for(hwnd));
    }
}

/// Arm the poll and window-state timers, start the watchdog threads, and
/// request the first poll and update check.
unsafe fn start_timers_and_workers(hwnd: HWND, args: &StartupArgs) {
    // Poll timer: 15 minutes
    let initial_poll_ms = {
        let state = lock_state();
        state
            .as_ref()
            .map(|s| s.poll_interval_ms)
            .unwrap_or(POLL_15_MIN)
    };
    set_controller_timer(Some(hwnd), TIMER_POLL, initial_poll_ms, None);
    sync_window_state_timer(hwnd);

    // Sample collision geometry off-thread and ask the stable controller to
    // reconcile shell-hosted windows every two seconds.
    taskbar_collision::spawn_reader();
    spawn_taskbar_watchdog();
    spawn_ui_hang_watchdog();

    if args.no_poll {
        return;
    }
    diagnose::log("initial poll requested");
    request_poll(hwnd);
    schedule_auto_update_check(hwnd);
    let should_check_updates = lock_state()
        .as_ref()
        .is_some_and(|s| auto_update_check_due(s.last_update_check_unix));
    if should_check_updates {
        begin_update_check(hwnd, false);
    }
}

unsafe fn run_message_loop() {
    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}
