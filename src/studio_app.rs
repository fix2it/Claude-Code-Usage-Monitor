//! DPI-aware settings dashboard and visual theme studio, hosted in a separate process.

use crate::app_settings::{self, SettingsFile, UsageCache, POLL_1_MIN};
use crate::localization::{self, LanguageId};
use crate::models::AppUsageData;
use crate::native_interop::{self, WM_APP_REFRESH_NOW, WM_APP_SETTINGS_UPDATED};
use crate::providers::PROVIDER_DESCRIPTORS;
use crate::theme_engine::{self, ThemeDocument};
use crate::ui::components::dropdown::{
    dropdown_selectable_label, dropdown_selectable_value, Dropdown,
};
use crate::ui::components::icon::{
    icon_only_button as lucide_icon_button, labeled_icon_button as lucide_labeled_button,
};
use crate::ui::components::layout::{
    setting_row, setting_row_with_mark, setting_separator, settings_scroll_area,
    settings_section as section,
};
use crate::ui::components::number_field::NumberField;
use crate::ui::components::toggle::Toggle;
use crate::ui::theme::{configure_style, menu_surface, muted, provider_mark_glyph};
use crate::ui::tokens::{
    CONTROL_HEIGHT, DEFAULT_DASHBOARD_HEIGHT, DEFAULT_DASHBOARD_WIDTH, DEFAULT_MENU_WIDTH,
};
use eframe::egui;
use lucide_icons::Icon as LucideIcon;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR,
};
use windows::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    PostMessageW, SendMessageW, ICON_BIG, ICON_SMALL, WM_SETICON,
};

pub fn handle_cli_mode(args: &[String]) -> bool {
    if !args.iter().any(|argument| argument == "--studio") {
        return false;
    }
    let owner = args
        .iter()
        .position(|value| value == "--owner")
        .and_then(|index| args.get(index + 1))
        .and_then(|value| value.parse::<isize>().ok())
        .unwrap_or(0);
    let owner_hwnd = HWND(owner as *mut _);
    let mut dashboard_instance = match crate::dashboard::claim_instance() {
        Ok(Some(instance)) => Some(instance),
        Ok(None) => return true,
        Err(error) => {
            crate::dashboard::report_launch_failure(owner_hwnd, &error);
            return true;
        }
    };
    let initial_page = Page::Settings;
    let settings = app_settings::load_settings();
    let dashboard_width = settings.dashboard_width.unwrap_or(DEFAULT_DASHBOARD_WIDTH);
    let dashboard_height = settings
        .dashboard_height
        .unwrap_or(DEFAULT_DASHBOARD_HEIGHT);
    let dashboard_icon = eframe::icon_data::from_png_bytes(include_bytes!("icons/16x16.png"))
        .expect("src/icons/16x16.png must be a valid PNG app icon");
    let result = if args.iter().any(|arg| arg == "--dashboard-warp") {
        warp::run(
            dashboard_width,
            dashboard_height,
            dashboard_icon,
            owner,
            initial_page,
        )
        .map_err(|error| match std::env::var("CCUM_DASHBOARD_OPENGL_ERROR") {
            Ok(gl_error) => format!("OpenGL: {gl_error}; WARP: {error}"),
            Err(_) => format!("WARP: {error}"),
        })
    } else {
        run_dashboard_with_fallback(|renderer, gl_error| match renderer {
            DashboardRenderer::OpenGl => run_dashboard(
                dashboard_width,
                dashboard_height,
                dashboard_icon.clone(),
                owner,
                initial_page,
            ),
            DashboardRenderer::Warp => {
                // winit cannot create another event loop after eframe has created one.
                // Release our process slot before starting a fresh WARP dashboard. The
                // child claims the same guard, so concurrent launch requests still coalesce.
                if let Some(handle) = dashboard_instance.take() {
                    unsafe {
                        let _ = windows::Win32::Foundation::CloseHandle(handle);
                    }
                }
                launch_warp_dashboard(args, gl_error.unwrap_or_default())
                    .map_err(|error| eframe::Error::AppCreation(error.into()))
            }
        })
    };
    if let Some(handle) = dashboard_instance.take() {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(handle);
        }
    }
    let failure = match result {
        Ok(()) => return true,
        Err(failure) => failure,
    };
    let settings = app_settings::load_settings();
    let language = localization::resolve_language(
        settings.language.as_deref().and_then(LanguageId::from_code),
    );
    crate::dashboard::report_launch_failure(
        owner_hwnd,
        &format!(
            "{}: {}",
            language.text("The dashboard could not initialize"),
            failure
        ),
    );
    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DashboardRenderer {
    OpenGl,
    Warp,
}

fn launch_warp_dashboard(args: &[String], gl_error: &str) -> Result<(), String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let mut command = std::process::Command::new(executable);
    command.args(args.iter().skip(1)).arg("--dashboard-warp");
    command.env("CCUM_DASHBOARD_OPENGL_ERROR", gl_error);
    if crate::diagnose::is_enabled() {
        command.arg("--diagnose-append");
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn run_dashboard_with_fallback(
    mut run: impl FnMut(DashboardRenderer, Option<&str>) -> eframe::Result,
) -> Result<(), String> {
    let glow_error = match run(DashboardRenderer::OpenGl, None) {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    crate::diagnose::log(format!(
        "dashboard OpenGL initialization failed: {glow_error}"
    ));

    // Context/configuration failures can occur before the OpenGL painter exists.
    if matches!(
        &glow_error,
        eframe::Error::OpenGL(_) | eframe::Error::Glutin(_) | eframe::Error::NoGlutinConfigs(..)
    ) {
        match run(DashboardRenderer::Warp, Some(&glow_error.to_string())) {
            Ok(()) => Ok(()),
            Err(error) => {
                crate::diagnose::log(format!("dashboard WARP initialization failed: {error}"));
                Err(format!("OpenGL: {glow_error}; WARP: {error}"))
            }
        }
    } else {
        Err(format!("OpenGL: {glow_error}"))
    }
}

fn run_dashboard(
    width: f32,
    height: f32,
    icon: egui::IconData,
    owner: isize,
    initial_page: Page,
) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(crate::dashboard::DASHBOARD_TITLE)
            .with_inner_size([width, height])
            .with_icon(icon),
        renderer: eframe::Renderer::Glow,
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        "ClaudeCodeUsageMonitor.Studio",
        options,
        Box::new(move |context| Ok(Box::new(StudioApp::new(context, owner, initial_page)))),
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Settings,
    Diagnostics,
}

struct StudioApp {
    owner: isize,
    update_status: crate::dashboard::UpdateStatus,
    diagnostics: studio_diagnostics::DiagnosticsView,
    page: Page,
    settings: SettingsFile,
    synced_poll_interval_ms: u32,
    synced_taskbar_lock: bool,
    poll_interval_editor_generation: u64,
    startup_enabled: bool,
    theme: ThemeDocument,
    theme_path: Option<PathBuf>,
    available_themes: AvailableThemesCache,
    usage: Option<AppUsageData>,
    usage_poll_ok: bool,
    usage_has_error: bool,
    last_cache_read: Instant,
    settings_error: Option<String>,
    theme_error: Option<String>,
}

mod studio_core;
mod studio_diagnostics;
mod studio_settings;
mod warp;

impl eframe::App for StudioApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_window_size();
    }
}

impl StudioApp {
    fn draw(&mut self, ui: &mut egui::Ui) {
        if let Some(size) = ui
            .ctx()
            .input(|input| input.viewport().inner_rect.map(|rect| rect.size()))
        {
            self.settings.dashboard_width = Some(size.x);
            self.settings.dashboard_height = Some(size.y);
        }
        self.refresh_usage_cache();
        egui::Frame::new()
            .fill(menu_surface())
            .inner_margin(egui::Margin {
                left: 10,
                right: 10,
                top: 0,
                bottom: 10,
            })
            .show(ui, |ui| self.shell(ui));
        ui.ctx().request_repaint_after(Duration::from_millis(500));
    }

    fn save_window_size(&self) {
        // Reload first so a monitor-process settings update made while the
        // dashboard was open is not overwritten by this final size save.
        let mut settings = app_settings::load_settings();
        settings.dashboard_width = self.settings.dashboard_width;
        settings.dashboard_height = self.settings.dashboard_height;
        if let Err(error) = app_settings::save_settings(&settings) {
            crate::diagnose::log(format!("dashboard size save failed: {error}"));
        }
    }
}

fn style_native_titlebar(context: &impl HasWindowHandle) {
    let Ok(window_handle) = context.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = window_handle.as_raw() else {
        return;
    };
    let hwnd = HWND(handle.hwnd.get() as *mut _);
    let (large_icon, small_icon) = crate::tray_icon::load_app_icons();
    // COLORREF uses 0x00BBGGRR. Keep the caption integrated with the app while
    // retaining the subtle outline that distinguishes ordinary Windows apps.
    let surface_color = 0x0020_2020u32;
    let border_color = 0x0054_5454u32;
    let text_color = 0x00F0_F0F0u32;
    unsafe {
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
        for (attribute, color) in [
            (DWMWA_CAPTION_COLOR, surface_color),
            (DWMWA_BORDER_COLOR, border_color),
            (DWMWA_TEXT_COLOR, text_color),
        ] {
            let _ = DwmSetWindowAttribute(
                hwnd,
                attribute,
                std::ptr::from_ref(&color).cast(),
                std::mem::size_of_val(&color) as u32,
            );
        }
    }
}

mod studio_utilities;
use studio_utilities::*;
#[cfg(test)]
mod tests;
