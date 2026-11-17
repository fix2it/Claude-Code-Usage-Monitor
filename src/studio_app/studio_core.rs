use super::*;

impl StudioApp {
    pub(super) fn language(&self) -> LanguageId {
        localization::resolve_language(
            self.settings
                .language
                .as_deref()
                .and_then(LanguageId::from_code),
        )
    }

    pub(super) fn new(
        context: &eframe::CreationContext<'_>,
        owner: isize,
        initial_page: Page,
    ) -> Self {
        style_native_titlebar(context);
        Self::new_with_context(&context.egui_ctx, owner, initial_page)
    }

    pub(super) fn new_with_context(
        context: &egui::Context,
        owner: isize,
        initial_page: Page,
    ) -> Self {
        crate::diagnose::log_lazy(|| format!("dashboard started owner={owner}"));
        let settings = app_settings::load_settings();
        let language = localization::resolve_language(
            settings.language.as_deref().and_then(LanguageId::from_code),
        );
        configure_style(context, language);
        let starter_theme_path = theme_engine::ensure_starter_theme().ok();
        let configured_path = settings.active_theme_path.as_ref().map(PathBuf::from);
        let configured_theme = configured_path
            .as_deref()
            .and_then(theme_engine::load_selected_theme)
            .filter(|theme| !theme.is_obsolete_studio_starter());
        let (theme_path, mut theme) = configured_theme
            .map(|theme| (configured_path, theme))
            .unwrap_or_else(|| {
                let path = starter_theme_path;
                let theme = path
                    .as_deref()
                    .and_then(|path| theme_engine::load_theme(path).ok())
                    .unwrap_or_else(ThemeDocument::starter);
                (path, theme)
            });
        theme.prepare_runtime();
        let usage_cache = app_settings::load_usage_cache().map(|mut cache| {
            cache.data.select_accounts(&settings.accounts);
            cache
        });
        let usage_poll_ok = usage_cache
            .as_ref()
            .is_some_and(|cache| cache.poll_ok && !cache.data.is_empty());
        let usage_has_error = usage_cache.as_ref().is_some_and(|cache| {
            !cache.poll_ok
                || (cache.data.is_empty()
                    && cache
                        .data
                        .accounts
                        .iter()
                        .any(|account| account.error.is_some()))
        });
        let usage = usage_cache.map(|cache| cache.data);
        Self {
            owner,
            update_status: crate::dashboard::read_update_status(owner),
            diagnostics: studio_diagnostics::DiagnosticsView::new(),
            page: initial_page,
            synced_poll_interval_ms: settings.poll_interval_ms,
            synced_taskbar_lock: settings.lock_taskbar,
            synced_settings: settings.clone(),
            poll_interval_editor_generation: 0,
            settings,
            startup_enabled: crate::window::is_startup_enabled(),
            theme,
            theme_path,
            available_themes: AvailableThemesCache::new(theme_engine::themes_directory()),
            usage,
            usage_poll_ok,
            usage_has_error,
            last_cache_read: Instant::now(),
            settings_error: None,
            monitor_save_error: None,
            monitor_save_error_shown: None,
            theme_error: None,
        }
    }

    pub(super) fn notify_owner(&self) {
        let _ = studio_diagnostics::send_owner_message(self.owner, WM_APP_SETTINGS_UPDATED);
    }

    pub(super) fn request_refresh(&mut self) {
        crate::diagnose::log("Refresh now clicked in dashboard");
        match studio_diagnostics::send_owner_message(self.owner, WM_APP_REFRESH_NOW) {
            Ok(()) => {
                crate::diagnose::log("refresh request delivered to monitor message queue");
                self.settings_error = None;
            }
            Err(error) => {
                crate::diagnose::log(&error);
                self.settings_error = Some(error);
            }
        }
    }

    pub(super) fn sync_poll_interval(&mut self, persisted_interval: u32) {
        // A local frequency edit wins; other dashboard edits must preserve a
        // newer interval selected from the widget's context menu.
        if self.settings.poll_interval_ms == self.synced_poll_interval_ms
            && self.settings.poll_interval_ms != persisted_interval
        {
            self.settings.poll_interval_ms = persisted_interval;
            // Drop the numeric control's old text buffer so losing focus cannot
            // commit the previous custom value over the menu selection.
            self.poll_interval_editor_generation =
                self.poll_interval_editor_generation.wrapping_add(1);
        }
        self.synced_poll_interval_ms = persisted_interval;
    }

    fn monitor_save_error_text(&self, error: Option<u32>) -> String {
        let text = self.language().text("The monitor could not save settings");
        match error {
            Some(code) => format!("{text} (Windows error {code})"),
            None => text.to_string(),
        }
    }

    /// Show the monitor's settings save failure, and clear it once a later
    /// save succeeds. A Studio-side error stays until Studio replaces it.
    pub(super) fn sync_monitor_save_error(&mut self, error: Option<Option<u32>>) {
        if error == self.monitor_save_error {
            // Another Studio action may have replaced or cleared the warning;
            // bring it back while the monitor still reports the failure.
            if let (Some(code), None) = (error, &self.settings_error) {
                self.monitor_save_error_shown = Some(self.monitor_save_error_text(code));
                self.settings_error = self.monitor_save_error_shown.clone();
            }
            return;
        }
        // Compare with the text actually shown: the language may have changed.
        if self.monitor_save_error_shown.is_some()
            && self.settings_error == self.monitor_save_error_shown
        {
            self.settings_error = None;
        }
        self.monitor_save_error_shown = error.map(|code| self.monitor_save_error_text(code));
        if self.monitor_save_error_shown.is_some() {
            self.settings_error = self.monitor_save_error_shown.clone();
        }
        self.monitor_save_error = error;
    }

    /// Adopt every other value the monitor saved since the last sync, unless
    /// Studio has a local edit of it. Compared per top-level settings key.
    pub(super) fn sync_other_settings(&mut self, persisted: &SettingsFile) {
        use serde_json::Value;
        let (Ok(Value::Object(mut local)), Ok(Value::Object(synced)), Ok(Value::Object(disk))) = (
            serde_json::to_value(&self.settings),
            serde_json::to_value(&self.synced_settings),
            serde_json::to_value(persisted),
        ) else {
            return;
        };
        // The Settings page shows an unset language as "system"; both mean the
        // same thing and must not count as a local edit.
        if local.get("language").and_then(Value::as_str) == Some("system") {
            local.remove("language");
        }
        let mut changed = false;
        // A key can be missing on disk, e.g. language "System" is omitted.
        let keys: std::collections::BTreeSet<String> = local
            .keys()
            .chain(synced.keys())
            .chain(disk.keys())
            .cloned()
            .collect();
        for key in keys {
            // Handled with their own editor state above.
            if key == "poll_interval_ms" || key == "lock_taskbar" {
                continue;
            }
            let local_value = local.get(&key);
            let disk_value = disk.get(&key);
            if local_value == synced.get(&key) && local_value != disk_value {
                match disk_value {
                    Some(value) => local.insert(key, value.clone()),
                    None => local.remove(&key),
                };
                changed = true;
            }
        }
        if changed {
            if let Ok(mut merged) = serde_json::from_value::<SettingsFile>(Value::Object(local)) {
                // Fields that never serialize keep their in-memory values.
                merged.tray_offset = self.settings.tray_offset;
                merged.taskbar_index = self.settings.taskbar_index;
                merged.widget_visible = self.settings.widget_visible;
                merged.legacy_placement_pending = self.settings.legacy_placement_pending;
                merged.legacy_visibility_pending = self.settings.legacy_visibility_pending;
                self.settings = merged;
            }
        }
        self.synced_settings = persisted.clone();
    }

    pub(super) fn sync_taskbar_lock(&mut self, persisted_lock: bool) {
        // Preserve a local toggle; unrelated edits adopt the latest menu value.
        if self.settings.lock_taskbar == self.synced_taskbar_lock {
            self.settings.lock_taskbar = persisted_lock;
        }
        self.synced_taskbar_lock = persisted_lock;
    }

    pub(super) fn save_settings(&mut self) {
        let persisted = app_settings::load_settings();
        self.sync_poll_interval(persisted.poll_interval_ms);
        self.sync_taskbar_lock(persisted.lock_taskbar);
        self.sync_other_settings(&persisted);
        match app_settings::save_settings(&self.settings) {
            Ok(()) => {
                self.synced_settings = self.settings.clone();
                self.synced_poll_interval_ms = self.settings.poll_interval_ms;
                self.synced_taskbar_lock = self.settings.lock_taskbar;
                self.settings_error = None;
                self.notify_owner();
            }
            Err(error) => {
                self.settings_error = Some(format!(
                    "{}: {error}",
                    self.language().text("Unable to save settings")
                ));
            }
        }
    }

    pub(super) fn activate_theme(&mut self, path: PathBuf) {
        let language = self.language();
        let theme = match theme_engine::load_theme(&path) {
            Ok(theme) => theme,
            Err(error) => {
                self.theme_error = Some(format!(
                    "{}: {error}",
                    language.text("Unable to open theme")
                ));
                return;
            }
        };
        self.activate_loaded_theme(theme, path);
    }

    pub(super) fn activate_loaded_theme(&mut self, theme: ThemeDocument, path: PathBuf) {
        let language = self.language();
        self.theme = theme;
        self.theme_path = Some(path.clone());
        self.settings.active_theme_path = Some(path.to_string_lossy().into_owned());
        self.settings.custom_theme_enabled = true;
        self.theme_error = None;
        if let Err(error) = app_settings::save_settings(&self.settings) {
            self.theme_error = Some(format!(
                "{}: {error}",
                language.text("The theme could not be activated")
            ));
        } else {
            self.notify_owner();
        }
    }

    pub(super) fn update_usage_cache(&mut self, mut cache: UsageCache) -> bool {
        cache.data.select_accounts(&self.settings.accounts);
        let poll_ok = cache.poll_ok && !cache.data.is_empty();
        let has_error = !cache.poll_ok
            || (cache.data.is_empty()
                && cache
                    .data
                    .accounts
                    .iter()
                    .any(|account| account.error.is_some()));
        let changed = self.usage.as_ref() != Some(&cache.data)
            || self.usage_poll_ok != poll_ok
            || self.usage_has_error != has_error;
        if changed {
            crate::diagnose::log_lazy(|| {
                format!(
                    "dashboard loaded usage cache: updated={} accounts={} poll_ok={poll_ok}",
                    cache.updated_unix,
                    cache.data.accounts.len()
                )
            });
            self.usage = Some(cache.data);
            self.usage_poll_ok = poll_ok;
            self.usage_has_error = has_error;
        }
        changed
    }

    pub(super) fn refresh_usage_cache(&mut self) {
        if self.last_cache_read.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.last_cache_read = Instant::now();
        if let Some(owner) = crate::dashboard::resolve_monitor_owner(self.owner) {
            self.owner = owner;
        }
        self.update_status = crate::dashboard::read_update_status(self.owner);
        let persisted = app_settings::load_settings();
        self.sync_poll_interval(persisted.poll_interval_ms);
        self.sync_taskbar_lock(persisted.lock_taskbar);
        self.sync_other_settings(&persisted);
        self.sync_monitor_save_error(crate::dashboard::read_settings_save_error(self.owner));
        if let Some(cache) = app_settings::load_usage_cache() {
            self.update_usage_cache(cache);
        }
    }

    pub(super) fn request_update_action(&mut self) {
        if self.update_status.is_busy() {
            return;
        }
        let install = matches!(
            self.update_status,
            crate::dashboard::UpdateStatus::Available(_)
        );
        self.send_update_action(install);
    }

    fn send_update_action(&mut self, install: bool) {
        // A refresh click must still check and prompt even if an automatic
        // check has found a release since the dashboard last read the status.
        let message = if install {
            native_interop::WM_APP_UPDATE_ACTION
        } else {
            native_interop::WM_APP_CHECK_FOR_UPDATES
        };
        match studio_diagnostics::send_owner_message(self.owner, message) {
            Ok(()) => {
                self.update_status = if install {
                    crate::dashboard::UpdateStatus::Applying
                } else {
                    crate::dashboard::UpdateStatus::Checking
                };
                self.last_cache_read = Instant::now();
            }
            Err(error) => self.theme_error = Some(error),
        }
    }

    pub(super) fn version_button(&mut self, ui: &mut egui::Ui) -> egui::Response {
        use crate::dashboard::UpdateStatus;
        let language = self.language();
        let (icon, tooltip) = match &self.update_status {
            UpdateStatus::Available(version) => (
                LucideIcon::Download,
                language
                    .text("Click to update to v{version}")
                    .replace("{version}", version),
            ),
            UpdateStatus::Checking => (
                LucideIcon::RefreshCw,
                language.strings().checking_for_updates.to_string(),
            ),
            UpdateStatus::Applying => (
                LucideIcon::Download,
                language.strings().applying_update.to_string(),
            ),
            UpdateStatus::Idle => (
                LucideIcon::RefreshCw,
                language.text("Check for updates").to_string(),
            ),
        };
        let response = ui
            .scope(|ui| {
                ui.add_enabled_ui(!self.update_status.is_busy(), |ui| {
                    let icon_id = egui::IdSalt::new("version-update-icon");
                    let version = format!("v{}", env!("CARGO_PKG_VERSION"));
                    let background = ui.painter().add(egui::Shape::Noop);
                    let button = egui::WidgetAtom::new((
                        egui::RichText::new(&version).size(16.0).color(muted()),
                        egui::Atom::custom(icon_id, egui::vec2(12.0, 12.0)),
                    ))
                    .gap(4.0)
                    // The footer row is bottom-aligned; centre the contents
                    // independently so its extra height is not all above them.
                    .align2(egui::Align2::LEFT_CENTER)
                    .sense(egui::Sense::click())
                    .min_size(egui::vec2(0.0, CONTROL_HEIGHT))
                    .frame(egui::Frame::new().inner_margin(egui::Margin {
                        left: 5,
                        right: 5,
                        top: 2,
                        bottom: 4,
                    }))
                    .show(ui);
                    if button.response.hovered() || button.response.has_focus() {
                        ui.painter().set(
                            background,
                            egui::Shape::rect_filled(
                                button.response.rect,
                                4.0,
                                crate::ui::theme::menu_hover(),
                            ),
                        );
                    }
                    button.response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &version)
                    });
                    if let Some(rect) = button.rect(icon_id) {
                        crate::ui::components::icon::paint_centered_icon(
                            ui,
                            rect.translate(egui::vec2(0.0, 1.0)),
                            icon,
                            12.0,
                            muted(),
                        );
                    }
                    button.response
                })
                .inner
            })
            .inner;
        let response = response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(&tooltip)
            .on_disabled_hover_text(&tooltip);
        if response.clicked() {
            self.request_update_action();
        }
        response
    }

    pub(super) fn shell(&mut self, ui: &mut egui::Ui) {
        let language = self.language();
        let full_height = ui.available_height();
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(DEFAULT_MENU_WIDTH, full_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| self.nav_panel(ui, language, full_height),
            );
            ui.add(egui::Separator::default().spacing(2.0));
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), full_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_min_height(full_height);
                    self.theme_error_banner(ui, language);
                    match self.page {
                        Page::Settings => self.settings_page(ui),
                        Page::Diagnostics => self.diagnostics_page(ui),
                    }
                },
            );
        });
    }

    /// Left menu: page links on top, GitHub link and version at the bottom.
    fn nav_panel(&mut self, ui: &mut egui::Ui, language: LanguageId, full_height: f32) {
        ui.set_width(DEFAULT_MENU_WIDTH);
        ui.set_min_height(full_height);
        ui.painter().rect_filled(ui.max_rect(), 0.0, menu_surface());
        egui::Frame::new()
            .inner_margin(egui::Margin {
                left: 8,
                right: 8,
                top: 20,
                bottom: 4,
            })
            .show(ui, |ui| {
                ui.set_width(DEFAULT_MENU_WIDTH - 16.0);
                nav(
                    ui,
                    &mut self.page,
                    Page::Settings,
                    language.text("Settings"),
                );
                ui.separator();
                nav(
                    ui,
                    &mut self.page,
                    Page::Diagnostics,
                    language.text("Diagnostics"),
                );
                ui.allocate_ui_with_layout(
                    ui.available_size(),
                    egui::Layout::bottom_up(egui::Align::Min),
                    |ui| self.nav_footer(ui),
                );
            });
    }

    fn nav_footer(&mut self, ui: &mut egui::Ui) {
        const GITHUB_URL: &str = "https://github.com/fix2it/Claude-Code-Usage-Monitor";

        ui.allocate_ui_with_layout(
            egui::vec2(
                ui.available_width(),
                crate::ui::components::navigation::ITEM_HEIGHT,
            ),
            egui::Layout::left_to_right(egui::Align::Max),
            |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                crate::ui::components::navigation::github_link(ui, GITHUB_URL);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Max), |ui| {
                    self.version_button(ui);
                });
            },
        );
    }

    /// Dismissible banner for the last theme error, if any.
    fn theme_error_banner(&mut self, ui: &mut egui::Ui, language: LanguageId) {
        if let Some(error) = self.theme_error.clone() {
            egui::Frame::new()
                .fill(egui::Color32::from_rgb(70, 34, 34))
                .corner_radius(egui::CornerRadius::same(5))
                .inner_margin(egui::Margin::symmetric(10, 7))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.colored_label(egui::Color32::from_rgb(255, 190, 178), error);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add(lucide_icon_button(LucideIcon::X))
                                .on_hover_text(language.text("Dismiss error"))
                                .clicked()
                            {
                                self.theme_error = None;
                            }
                        });
                    });
                });
            ui.add_space(8.0);
        }
    }
}
