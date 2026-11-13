use super::*;
use crate::app_settings::{POLL_15_MIN, POLL_5_MIN};

fn test_app() -> StudioApp {
    let mut settings = SettingsFile::default();
    settings.language = Some("en-US".into());
    StudioApp {
        owner: 0,
        update_status: crate::dashboard::UpdateStatus::Idle,
        diagnostics: studio_diagnostics::DiagnosticsView::new(),
        page: Page::Settings,
        synced_poll_interval_ms: settings.poll_interval_ms,
        synced_taskbar_lock: settings.lock_taskbar,
        poll_interval_editor_generation: 0,
        settings,
        startup_enabled: false,
        theme: ThemeDocument::starter(),
        theme_path: None,
        available_themes: AvailableThemesCache::new(theme_engine::themes_directory()),
        usage: None,
        usage_poll_ok: false,
        usage_has_error: false,
        last_cache_read: Instant::now(),
        settings_error: None,
        theme_error: None,
    }
}

#[test]
fn dashboard_does_not_retry_after_successful_opengl_run() {
    let mut attempts = Vec::new();
    run_dashboard_with_fallback(|renderer, _gl_error| {
        attempts.push(renderer);
        Ok(())
    })
    .unwrap();
    assert_eq!(attempts, [DashboardRenderer::OpenGl]);
}

#[test]
fn dashboard_retries_opengl_painter_and_configuration_failures() {
    for error in [
        eframe::Error::OpenGL("OpenGL 2.0 unavailable".to_owned().into()),
        eframe::Error::NoGlutinConfigs(Default::default(), "No GL configuration".into()),
    ] {
        let mut error = Some(error);
        let mut attempts = Vec::new();
        run_dashboard_with_fallback(|renderer, _gl_error| {
            attempts.push(renderer);
            error.take().map_or(Ok(()), Err)
        })
        .unwrap();
        assert_eq!(
            attempts,
            [DashboardRenderer::OpenGl, DashboardRenderer::Warp]
        );
    }
}

#[test]
fn dashboard_does_not_retry_application_errors() {
    let mut attempts = Vec::new();
    let error = run_dashboard_with_fallback(|renderer, _gl_error| {
        attempts.push(renderer);
        Err(eframe::Error::AppCreation("Application failed".into()))
    })
    .unwrap_err();
    assert_eq!(attempts, [DashboardRenderer::OpenGl]);
    assert!(error.contains("Application failed"));
}

#[test]
fn dashboard_reports_both_failures_without_retrying_again() {
    let mut attempts = Vec::new();
    let error = run_dashboard_with_fallback(|renderer, _gl_error| {
        attempts.push(renderer);
        match renderer {
            DashboardRenderer::OpenGl => Err(eframe::Error::OpenGL(
                "OpenGL unavailable".to_owned().into(),
            )),
            DashboardRenderer::Warp => Err(eframe::Error::AppCreation("Fallback failed".into())),
        }
    })
    .unwrap_err();
    assert_eq!(
        attempts,
        [DashboardRenderer::OpenGl, DashboardRenderer::Warp]
    );
    assert!(error.contains("OpenGL:"));
    assert!(error.contains("OpenGL unavailable"));
    assert!(error.contains("WARP:"));
    assert!(error.contains("Fallback failed"));
}

#[test]
fn warp_launch_receives_the_original_opengl_error() {
    run_dashboard_with_fallback(|renderer, original_error| match renderer {
        DashboardRenderer::OpenGl => {
            assert!(original_error.is_none());
            Err(eframe::Error::OpenGL(
                "missing OpenGL driver".to_owned().into(),
            ))
        }
        DashboardRenderer::Warp => {
            assert!(original_error.unwrap().contains("missing OpenGL driver"));
            Ok(())
        }
    })
    .unwrap();
}

fn run_test_ui(context: &egui::Context, input: egui::RawInput, run_ui: impl FnMut(&mut egui::Ui)) {
    let mut output = context.run_ui(input, run_ui);
    output.textures_delta.clear();
}

#[test]
fn dashboard_keeps_dark_visuals_when_system_theme_changes() {
    for initial_theme in [None, Some(egui::Theme::Light), Some(egui::Theme::Dark)] {
        let context = egui::Context::default();
        run_test_ui(
            &context,
            egui::RawInput {
                system_theme: initial_theme,
                ..Default::default()
            },
            |_| {},
        );
        configure_style(&context, LanguageId::English);

        for system_theme in [egui::Theme::Light, egui::Theme::Dark, egui::Theme::Light] {
            run_test_ui(
                &context,
                egui::RawInput {
                    system_theme: Some(system_theme),
                    ..Default::default()
                },
                |ui| {
                    assert!(ui.visuals().dark_mode);
                    assert_eq!(ui.visuals().panel_fill, menu_surface());
                    assert_eq!(
                        ui.visuals().text_color(),
                        egui::Visuals::dark().text_color()
                    );
                    assert_eq!(ui.spacing().item_spacing, egui::vec2(9.0, 8.0));
                },
            );
        }
    }
}

#[test]
fn configured_fonts_render_fallback_text_and_lucide_icons() {
    let context = egui::Context::default();
    configure_style(&context, LanguageId::English);

    run_test_ui(&context, egui::RawInput::default(), |ui| {
        let fallback = ui.ctx().fonts_mut(|fonts| {
            fonts.layout_no_wrap(
                "Fallback — ‘ready’".into(),
                egui::FontId::new(14.0, egui::FontFamily::Name("ui-fallback".into())),
                egui::Color32::WHITE,
            )
        });
        assert!(fallback.size().x > 0.0);

        for language in LanguageId::ALL {
            let native_name = language.native_name();
            assert_eq!(
                language_name(LanguageId::English, language.code()),
                native_name
            );
            let native_label = ui.ctx().fonts_mut(|fonts| {
                fonts.layout_no_wrap(
                    native_name.into(),
                    egui::FontId::new(14.0, egui::FontFamily::Proportional),
                    egui::Color32::WHITE,
                )
            });
            assert!(native_label.size().x > 0.0);
        }

        let icon = ui.ctx().fonts_mut(|fonts| {
            fonts.layout_no_wrap(
                format!(
                    "{}{}",
                    LucideIcon::X.unicode(),
                    LucideIcon::ChevronRight.unicode()
                ),
                egui::FontId::new(14.0, egui::FontFamily::Name("lucide".into())),
                egui::Color32::WHITE,
            )
        });
        assert!(icon.size().x > 0.0);

        let native_menu = ui.ctx().fonts_mut(|fonts| {
            fonts.layout_no_wrap(
                "Native menu".into(),
                egui::FontId::new(12.0, egui::FontFamily::Name("native-menu".into())),
                egui::Color32::BLACK,
            )
        });
        assert!(native_menu.size().x > 0.0);
    });
}

mod monitor_reconnection {
    use super::*;
    use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::*;

    struct MonitorWindow(HWND);

    impl MonitorWindow {
        fn new() -> Self {
            static REGISTERED: std::sync::Once = std::sync::Once::new();
            unsafe extern "system" fn procedure(
                hwnd: HWND,
                message: u32,
                wparam: WPARAM,
                lparam: LPARAM,
            ) -> LRESULT {
                unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
            }
            let class = native_interop::wide_str("ClaudeCodeUsageMonitor");
            unsafe {
                let module = GetModuleHandleW(PCWSTR::null()).unwrap();
                REGISTERED.call_once(|| {
                    let window_class = WNDCLASSW {
                        lpfnWndProc: Some(procedure),
                        hInstance: module.into(),
                        lpszClassName: PCWSTR::from_raw(class.as_ptr()),
                        ..Default::default()
                    };
                    assert_ne!(RegisterClassW(&window_class), 0);
                });
                Self(
                    CreateWindowExW(
                        WINDOW_EX_STYLE::default(),
                        PCWSTR::from_raw(class.as_ptr()),
                        PCWSTR::null(),
                        WINDOW_STYLE::default(),
                        0,
                        0,
                        0,
                        0,
                        Some(HWND_MESSAGE),
                        None,
                        Some(module.into()),
                        None,
                    )
                    .unwrap(),
                )
            }
        }

        fn owner(&self) -> isize {
            self.0 .0 as isize
        }

        fn assert_message(&self, message: u32) {
            let mut received = MSG::default();
            assert!(unsafe {
                PeekMessageW(&mut received, Some(self.0), message, message, PM_REMOVE).as_bool()
            });
            assert_eq!(received.hwnd, self.0);
            assert_eq!(received.message, message);
        }
    }

    impl Drop for MonitorWindow {
        fn drop(&mut self) {
            unsafe { DestroyWindow(self.0).unwrap() };
        }
    }

    #[test]
    fn dashboard_reconnects_after_monitor_replacement_and_preserves_edits() {
        let original = MonitorWindow::new();
        let mut app = test_app();
        app.owner = original.owner();
        app.theme.name = "Unsaved theme".into();
        app.settings.poll_interval_ms = POLL_5_MIN;
        drop(original);

        let replacement = MonitorWindow::new();
        crate::dashboard::publish_update_status(
            replacement.0,
            crate::dashboard::UpdateStatus::Available("3.0.0".into()),
        );
        app.last_cache_read = Instant::now() - Duration::from_secs(2);
        app.refresh_usage_cache();

        assert_eq!(app.owner, replacement.owner());
        assert_eq!(
            app.update_status,
            crate::dashboard::UpdateStatus::Available("3.0.0".into())
        );
        assert_eq!(app.theme.name, "Unsaved theme");
        assert_eq!(app.settings.poll_interval_ms, POLL_5_MIN);
        app.request_refresh();
        assert!(app.settings_error.is_none());
        replacement.assert_message(WM_APP_REFRESH_NOW);
    }

    #[test]
    fn dashboard_commands_reconnect_before_periodic_refresh() {
        let original = MonitorWindow::new();
        let mut app = test_app();
        app.owner = original.owner();
        drop(original);
        let replacement = MonitorWindow::new();
        crate::dashboard::publish_update_status(
            replacement.0,
            crate::dashboard::UpdateStatus::Idle,
        );

        app.request_refresh();
        assert!(app.settings_error.is_none());
        replacement.assert_message(WM_APP_REFRESH_NOW);
        app.save_settings();
        assert!(app.settings_error.is_none());
        replacement.assert_message(WM_APP_SETTINGS_UPDATED);
        for message in [
            native_interop::WM_APP_ENABLE_DIAGNOSTICS,
            native_interop::WM_APP_DISABLE_DIAGNOSTICS,
        ] {
            studio_diagnostics::send_owner_message(app.owner, message).unwrap();
            replacement.assert_message(message);
        }
        app.request_update_action();
        assert!(app.theme_error.is_none());
        assert_eq!(app.update_status, crate::dashboard::UpdateStatus::Checking);
        replacement.assert_message(native_interop::WM_APP_CHECK_FOR_UPDATES);
    }

    #[test]
    fn dashboard_rejects_stale_or_unrelated_monitor_snapshots() {
        let original = MonitorWindow::new();
        let dead_owner = original.owner();
        drop(original);
        let replacement = MonitorWindow::new();
        let snapshot_path = app_settings::app_data_directory().join("update-status.json");
        app_settings::write_json_atomic(
            &snapshot_path,
            &serde_json::json!({
                "owner": replacement.owner(),
                "process_id": 0,
                "status": {"Available": "3.0.0"}
            }),
        )
        .unwrap();
        assert_eq!(crate::dashboard::resolve_monitor_owner(dead_owner), None);
        assert_eq!(
            crate::dashboard::read_update_status(replacement.owner()),
            crate::dashboard::UpdateStatus::Idle
        );

        let unrelated_class = native_interop::wide_str("STATIC");
        let unrelated = MonitorWindow(unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                PCWSTR::from_raw(unrelated_class.as_ptr()),
                PCWSTR::null(),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                None,
                None,
            )
            .unwrap()
        });
        crate::dashboard::publish_update_status(
            unrelated.0,
            crate::dashboard::UpdateStatus::Available("3.0.0".into()),
        );
        assert_eq!(crate::dashboard::resolve_monitor_owner(dead_owner), None);
        assert_eq!(
            crate::dashboard::read_update_status(unrelated.owner()),
            crate::dashboard::UpdateStatus::Idle
        );

        crate::dashboard::publish_update_status(
            replacement.0,
            crate::dashboard::UpdateStatus::Idle,
        );
        drop(replacement);
        assert_eq!(crate::dashboard::resolve_monitor_owner(dead_owner), None);
        std::fs::write(snapshot_path, b"incomplete JSON").unwrap();
        assert_eq!(crate::dashboard::resolve_monitor_owner(dead_owner), None);
    }

    #[test]
    fn dashboard_keeps_live_owner_and_standalone_editor_disconnected() {
        let original = MonitorWindow::new();
        let other = MonitorWindow::new();
        crate::dashboard::publish_update_status(other.0, crate::dashboard::UpdateStatus::Idle);
        assert_eq!(
            crate::dashboard::resolve_monitor_owner(original.owner()),
            Some(original.owner())
        );
        assert_eq!(crate::dashboard::resolve_monitor_owner(0), None);
    }
}

#[test]
fn selector_excludes_retired_managed_theme_and_user_theme_persists() {
    let managed = theme_engine::themes_directory();
    std::fs::create_dir_all(&managed).unwrap();
    let retired = managed.join("classic-vertical.json");
    let user_directory = tempfile::tempdir().unwrap();
    let user_theme = user_directory.path().join("classic-vertical.json");
    let mut theme = ThemeDocument::starter();
    theme.id = "user-theme".into();
    theme.name = "User theme".into();
    app_settings::write_json_atomic(&retired, &theme).unwrap();
    app_settings::write_json_atomic(&user_theme, &theme).unwrap();

    let mut app = test_app();
    app.theme_path = Some(retired.clone());
    assert!(!app
        .available_themes()
        .iter()
        .any(|entry| entry.path == retired));
    app.activate_theme(user_theme.clone());
    assert!(app.theme_error.is_none());
    assert!(app
        .available_themes()
        .iter()
        .any(|entry| entry.path == user_theme));

    let mut reloaded = StudioApp::new_with_context(&egui::Context::default(), 0, Page::Settings);
    assert_eq!(reloaded.theme_path.as_deref(), Some(user_theme.as_path()));
    assert_eq!(reloaded.theme.name, "User theme");
    let themes = reloaded.available_themes();
    assert!(themes.iter().any(|entry| entry.path == user_theme));
    assert!(!themes.iter().any(|entry| entry.path == retired));
    assert!(retired.exists());
}

#[test]
fn available_themes_keeps_current_external_theme_fresh_without_stale_fallbacks() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = test_app();
    app.available_themes = AvailableThemesCache::new(directory.path().join("library"));
    let first_path = directory.path().join("first.json");
    app.theme_path = Some(first_path.clone());
    app.theme.name = "First".into();
    let themes = app.available_themes();
    assert_eq!(themes.len(), 1);
    assert_eq!(themes[0].path, first_path);
    app.theme.name = "Renamed".into();
    assert_eq!(app.available_themes()[0].name, "Renamed");
    let second_path = directory.path().join("second.json");
    app.theme_path = Some(second_path.clone());
    let themes = app.available_themes();
    assert_eq!(themes.len(), 1);
    assert_eq!(themes[0].path, second_path);
    app.theme_path = None;
    assert!(app.available_themes().is_empty());
}

#[test]
fn diagnostics_page_has_logging_controls_and_menu_version() {
    let context = egui::Context::default();
    configure_style(&context, LanguageId::English);
    let mut app = test_app();
    app.page = Page::Diagnostics;
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100.0, 850.0),
            )),
            ..Default::default()
        },
        |ui| app.shell(ui),
    );
    fn collect_text(shape: &egui::epaint::Shape, text: &mut String) {
        match shape {
            egui::epaint::Shape::Text(shape) => {
                text.push_str(&shape.galley.job.text);
                text.push('\n');
            }
            egui::epaint::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect_text(shape, text);
                }
            }
            _ => {}
        }
    }
    let mut text = String::new();
    for shape in &output.shapes {
        collect_text(&shape.shape, &mut text);
    }
    output.textures_delta.clear();
    assert!(text.find("Settings").unwrap() < text.find("Diagnostics").unwrap());
    assert!(!text.contains("Theme Studio"));
    assert!(!text.contains("Context Menus"));
    assert!(!text.contains("Assets"));
    assert!(text.contains(&format!("v{}", env!("CARGO_PKG_VERSION"))));
    assert!(text.contains("Logging"));
    assert!(text.contains("Write diagnostic events to"));
    assert!(text.contains("Disabled"));
    assert!(text.contains("Refresh usage"));
    assert!(text.contains("Follow latest event"));
    assert!(text.contains("Copy log"));
    assert!(!text.contains("Monitor disconnected"));
    assert!(!text.contains("Dashboard process"));
    assert!(!text.contains("Record diagnostics"));
    assert!(!text.contains("Pause output"));
    assert!(
        !crate::diagnose::is_enabled(),
        "visiting the page must not start recording"
    );
}

#[test]
fn context_menu_frequency_replaces_custom_dashboard_value_and_edit_buffer() {
    let mut app = test_app();
    app.settings.poll_interval_ms = 2 * POLL_1_MIN;
    app.synced_poll_interval_ms = app.settings.poll_interval_ms;

    app.sync_poll_interval(POLL_5_MIN);
    assert_eq!(app.settings.poll_interval_ms, POLL_5_MIN);
    assert_eq!(app.poll_interval_editor_generation, 1);

    // An unrelated settings edit must not restore the previous two minutes.
    app.settings.usage_countdown = true;
    app.sync_poll_interval(POLL_5_MIN);
    assert_eq!(app.settings.poll_interval_ms, POLL_5_MIN);
    assert_eq!(app.poll_interval_editor_generation, 1);
    assert!(app.settings.usage_countdown);
}

#[test]
fn local_frequency_edits_survive_sync_before_save() {
    let mut app = test_app();
    app.sync_poll_interval(POLL_5_MIN);
    let generation = app.poll_interval_editor_generation;
    app.settings.poll_interval_ms = 2 * POLL_1_MIN;

    // The on-disk preset is still five until the local edit has been saved.
    app.sync_poll_interval(POLL_5_MIN);
    assert_eq!(app.settings.poll_interval_ms, 2 * POLL_1_MIN);
    assert_eq!(app.poll_interval_editor_generation, generation);

    // After saving, a later context-menu selection becomes authoritative again.
    app.synced_poll_interval_ms = app.settings.poll_interval_ms;
    app.sync_poll_interval(POLL_15_MIN);
    assert_eq!(app.settings.poll_interval_ms, POLL_15_MIN);
    assert_eq!(app.poll_interval_editor_generation, generation + 1);
}

#[test]
fn menu_taskbar_lock_survives_unrelated_dashboard_edits() {
    let mut app = test_app();
    let mut persisted = SettingsFile::default();
    persisted.lock_taskbar = true;
    app_settings::save_settings(&persisted).unwrap();
    // Simulate a menu toggle while the dashboard still holds its startup snapshot.
    assert!(!app.settings.lock_taskbar);
    app.settings.usage_countdown = true;
    app.save_settings();
    assert!(app.settings.lock_taskbar);
    let saved = app_settings::load_settings();
    assert!(saved.lock_taskbar && saved.usage_countdown);
    persisted.lock_taskbar = false;
    app_settings::save_settings(&persisted).unwrap();
    app.sync_taskbar_lock(false);
    assert!(!app.settings.lock_taskbar);
}

#[test]
fn local_taskbar_lock_edits_survive_sync_before_save() {
    let mut app = test_app();
    for saved in [false, true] {
        app.settings.lock_taskbar = saved;
        app.synced_taskbar_lock = saved;
        app.settings.lock_taskbar = !saved;
        app.sync_taskbar_lock(saved);
        assert_eq!(app.settings.lock_taskbar, !saved);
        // A subsequent menu toggle becomes authoritative after saving.
        app.synced_taskbar_lock = app.settings.lock_taskbar;
        app.sync_taskbar_lock(saved);
        assert_eq!(app.settings.lock_taskbar, saved);
    }
}

#[test]
fn version_text_and_trailing_icon_share_one_update_button() {
    fn footer_button(app: &mut StudioApp, ui: &mut egui::Ui) -> egui::Response {
        ui.allocate_ui_with_layout(
            egui::vec2(94.0, CONTROL_HEIGHT),
            egui::Layout::right_to_left(egui::Align::Max),
            |ui| app.version_button(ui),
        )
        .inner
    }
    for status in [
        crate::dashboard::UpdateStatus::Idle,
        crate::dashboard::UpdateStatus::Available("9.8.7".into()),
    ] {
        for click_icon in [false, true] {
            let context = egui::Context::default();
            configure_style(&context, LanguageId::English);
            let mut app = test_app();
            app.update_status = status.clone();
            let mut rect = egui::Rect::NOTHING;
            run_test_ui(&context, egui::RawInput::default(), |ui| {
                rect = footer_button(&mut app, ui).rect;
            });
            let footer_space = DEFAULT_MENU_WIDTH - 16.0 - 36.0 * 98.0 / 96.0 - 4.0;
            assert!(
                rect.width() <= footer_space,
                "version button overflows footer: {rect:?}"
            );
            let position = egui::pos2(
                if click_icon {
                    rect.right() - 8.0
                } else {
                    rect.left() + 12.0
                },
                rect.center().y,
            );
            // Settle pointer movement, then allow the tooltip to appear.
            run_test_ui(
                &context,
                egui::RawInput {
                    time: Some(1.0),
                    events: vec![egui::Event::PointerMoved(position)],
                    ..Default::default()
                },
                |ui| {
                    footer_button(&mut app, ui);
                },
            );
            run_test_ui(
                &context,
                egui::RawInput {
                    time: Some(2.0),
                    ..Default::default()
                },
                |ui| {
                    footer_button(&mut app, ui);
                },
            );
            let mut output = context.run_ui(
                egui::RawInput {
                    time: Some(3.0),
                    ..Default::default()
                },
                |ui| {
                    rect = footer_button(&mut app, ui).rect;
                },
            );
            let mut text = String::new();
            let mut outlined = false;
            let mut menu_hover_fill = false;
            for shape in &output.shapes {
                match &shape.shape {
                    egui::epaint::Shape::Text(shape) => {
                        text.push_str(&shape.galley.job.text);
                        if shape.galley.job.text == format!("v{}", env!("CARGO_PKG_VERSION")) {
                            let ink_center = shape.pos.y + shape.galley.mesh_bounds.center().y;
                            assert!((ink_center - rect.center().y).abs() <= 1.0,
                                "version text is not visually centred: ink={ink_center}, button={rect:?}");
                        }
                    }
                    egui::epaint::Shape::Rect(shape) if shape.rect.contains_rect(rect) => {
                        outlined |= shape.stroke.width > 0.0;
                        menu_hover_fill |= shape.fill == crate::ui::theme::menu_hover();
                    }
                    _ => {}
                }
            }
            output.textures_delta.clear();
            let (icon, tooltip) = if matches!(status, crate::dashboard::UpdateStatus::Available(_))
            {
                (LucideIcon::Download, "Click to update to v9.8.7")
            } else {
                (LucideIcon::RefreshCw, "Check for updates")
            };
            assert!(text.contains(icon.unicode()), "missing update icon: {text}");
            assert!(text.contains(tooltip), "missing update tooltip: {text}");
            assert!(!outlined, "version button hover should have no border");
            assert!(
                menu_hover_fill,
                "version button should use the menu hover fill"
            );
            let input = egui::RawInput {
                events: vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            };
            run_test_ui(&context, input, |ui| {
                footer_button(&mut app, ui);
            });
            assert!(app
                .theme_error
                .as_deref()
                .unwrap()
                .contains("not connected"));
        }
    }
}

#[test]
fn update_button_ignores_repeated_clicks_while_busy() {
    for status in [
        crate::dashboard::UpdateStatus::Checking,
        crate::dashboard::UpdateStatus::Applying,
    ] {
        let mut app = test_app();
        app.update_status = status;
        app.request_update_action();
        assert!(app.theme_error.is_none());
    }
}

#[test]
fn disconnected_update_button_reports_error_without_staying_busy() {
    let mut app = test_app();
    app.request_update_action();
    assert!(app
        .theme_error
        .as_deref()
        .unwrap()
        .contains("not connected"));
    assert!(!app.update_status.is_busy());
}
