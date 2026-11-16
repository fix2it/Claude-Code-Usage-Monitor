use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Diagnostics::Debug::IsDebuggerPresent;
use windows::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW};
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcess, TerminateProcess, WaitForSingleObject,
};
use windows::Win32::UI::Accessibility::HWINEVENTHOOK;
use windows::Win32::UI::Controls::{HOVER_DEFAULT, WM_MOUSEHOVER, WM_MOUSELEAVE};
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetDoubleClickTime, ReleaseCapture, SetCapture, TrackMouseEvent, TME_HOVER, TME_LEAVE,
    TRACKMOUSEEVENT,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app_settings::{
    self, load_settings, save_settings, LegacyPlacement, PlacementOverride, SettingsFile,
    POLL_15_MIN, POLL_15_MIN_SECONDS, POLL_1_HOUR, POLL_1_HOUR_SECONDS, POLL_1_MIN,
    POLL_1_MIN_SECONDS, POLL_5_MIN, POLL_5_MIN_SECONDS,
};
use crate::context_menu::{self, ContextMenuAction, ContextMenuItem, ContextMenuItemKind};
use crate::diagnose;
use crate::localization::{self, LanguageId, Strings};
use crate::models::AppUsageData;
use crate::native_interop::{
    self, TIMER_CLOCK, TIMER_COUNTDOWN, TIMER_CROSSFADE_SCHEDULE, TIMER_MOUSE_CLICK, TIMER_POLL,
    TIMER_RESET_POLL, TIMER_TRAY_HOVER, TIMER_TRAY_REPOSITION, TIMER_UPDATE_CHECK,
    TIMER_WINDOW_STATE, WM_APP_DISABLE_DIAGNOSTICS, WM_APP_ENABLE_DIAGNOSTICS,
    WM_APP_OPEN_DASHBOARD, WM_APP_QUIT, WM_APP_REFRESH_NOW, WM_APP_SETTINGS_UPDATED,
    WM_APP_TASKBAR_COLLISION, WM_APP_TRAY, WM_APP_USAGE_UPDATED,
};
use crate::poller;
use crate::providers::{ProviderId, ProviderSet};
use crate::theme;
use crate::theme_engine::{
    self, Canvas, DataContext, HorizontalAnchor, MouseActionEffect, MouseActionOverrideKey,
    MouseEventKind, ReferenceRegion, SurfaceNest, ThemeDocument, ThemeRuntime, VerticalAnchor,
};
use crate::tray_icon;
use crate::updater::{self, AvailableUpdate, InstallChannel, ReleaseDescriptor, UpdateCheckResult};

/// Copyable HWND value used by the watchdog after the UI thread publishes it.
#[derive(Clone, Copy, PartialEq, Eq)]
struct SendHwnd(isize);

// SAFETY: this wrapper never transfers ownership of a window. Cross-thread users
// only pass the value back to Win32 APIs that explicitly accept handles created
// by another thread (for example IsWindow and PostMessageW).
unsafe impl Send for SendHwnd {}

impl SendHwnd {
    fn from_hwnd(hwnd: HWND) -> Self {
        Self(hwnd.0 as isize)
    }
    fn to_hwnd(self) -> HWND {
        HWND(self.0 as *mut _)
    }
}

/// Copyable event-hook value whose lifetime remains owned by the UI controller.
#[derive(Clone, Copy)]
struct SendWinEventHook(isize);

// SAFETY: the hook is only stored or passed to UnhookWinEvent. Callback work is
// marshalled through Win32; Rust data is never dereferenced through this value.
unsafe impl Send for SendWinEventHook {}

impl SendWinEventHook {
    fn from_hook(hook: HWINEVENTHOOK) -> Self {
        Self(hook.0 as isize)
    }

    fn to_hook(self) -> HWINEVENTHOOK {
        HWINEVENTHOOK(self.0 as *mut _)
    }
}

/// Shared application state
struct AppState {
    hwnd: SendHwnd,
    taskbar_hwnd: Option<SendHwnd>,
    tray_notify_hwnd: Option<SendHwnd>,
    win_event_hook: Option<SendWinEventHook>,
    is_dark: bool,
    embedded: bool,
    language_override: Option<LanguageId>,
    language: LanguageId,
    install_channel: InstallChannel,

    providers: ProviderSet,
    pub(super) show_antigravity_gemini: bool,
    pub(super) show_antigravity_external: bool,
    /// External models of agy pool o (pool a's is `show_antigravity_external`).
    pub(super) show_antigravity_external_x: bool,
    /// Hover state and hit-test rects of the provider and split-pool tiles.
    pub(super) tiles: tiles::Tiles,
    /// Selected widget view mode (1..3) and its continuous animation phase
    /// (0.0 = mode 1, 1.0 = mode 2, 2.0 = mode 3).
    view_mode: u8,
    mode_phase: f64,
    widget_modes: [(u8, f64); crate::theme_engine::WIDGET_SLOTS],
    /// +1 while the click cycle moves 1 -> 3, -1 while it moves back.
    /// The wheel overwrites this explicitly; clicks bounce off the ends.
    widget_directions: [i8; crate::theme_engine::WIDGET_SLOTS],
    widget_animations: [Option<PhaseAnimation>; crate::theme_engine::WIDGET_SLOTS],
    accounts: crate::accounts::AccountSettings,

    data: Option<AppUsageData>,
    prepared_metrics: Option<theme_engine::DisplayMetrics>,

    poll_interval_ms: u32,
    retry_count: u32,
    force_notify_auth_error: bool,
    auth_error_paused_polling: bool,
    auth_watch_mode: poller::CredentialWatchMode,
    auth_watch_snapshot: poller::CredentialWatchSnapshot,
    last_poll_ok: bool,
    last_poll_failure: Option<poller::PollFailure>,
    /// When the last poll succeeded, for the tray tooltip's "updated HH:MM".
    last_poll_ok_at: Option<SystemTime>,
    update_status: UpdateStatus,
    last_update_check_unix: Option<u64>,

    taskbar_index: usize,
    tray_offset: i32,
    dragging: bool,
    pending_drag: bool,
    drag_start_cursor: POINT,
    drag_start_origin: POINT,
    drag_start_client_x: i32,
    auto_ejected: bool,
    auto_ejected_for_capacity: bool,
    auto_ejected_origin: Option<POINT>,
    auto_ejected_host: Option<app_settings::FloatingHost>,
    /// When true, never auto-eject to floating on taskbar collisions.
    /// The widget stays docked (but can still be dragged along the bar).
    lock_taskbar: bool,
    is_switching_window_style: bool,
    is_snapped: bool,
    placement_override: Option<PlacementOverride>,
    floating_card_opacity: Option<u8>,
    window_state_timer_active: bool,

    custom_theme_enabled: bool,
    usage_countdown: bool,
    use_windows_default_font: bool,
    show_pace: bool,
    show_fable: bool,
    show_fable_x: bool,
    claude_pools_one_widget: bool,
    codex_pools_one_widget: bool,
    antigravity_pools_one_widget: bool,
    active_theme_path: Option<PathBuf>,
    active_theme: Option<ThemeDocument>,
    theme_clock_interval: Option<Duration>,
    tray_theme_uses_current_time: bool,
    mirror_hwnds: Vec<SendHwnd>,
    desktop_hwnds: Vec<Option<SendHwnd>>,
    mouse_action_overrides: HashMap<MouseActionOverrideKey, theme_engine::Expression>,
    hovered_mouse_layer: Option<(usize, String)>,
    pending_mouse_click: Option<PendingMouseClick>,
    suppress_next_left_up: bool,
}

impl AppState {
    /// All animation slots share the crossfade timer, including the view-mode
    /// transition. The timer stops only when every slot has settled.
    pub(super) fn is_any_animation_active(&self) -> bool {
        self.tiles.any_animating() || self.widget_animations.iter().any(Option::is_some)
    }

    pub(super) fn stop_crossfade_timer_if_idle(&self, hwnd: HWND) {
        if !self.is_any_animation_active() {
            animation_timer::stop(hwnd);
        }
    }

    pub(super) fn is_account_profile_enabled(
        &self,
        provider: ProviderId,
        id: &str,
        default_if_missing: bool,
    ) -> bool {
        self.providers.contains(provider)
            && match provider {
                ProviderId::Claude => self
                    .accounts
                    .claude
                    .is_profile_enabled(id, default_if_missing),
                ProviderId::Codex => self
                    .accounts
                    .codex
                    .is_profile_enabled(id, default_if_missing),
                _ => false,
            }
    }

    /// The "Pools on one widget" choice of a two-pool provider.
    pub(super) fn pools_one_widget_mut(&mut self, provider: ProviderId) -> Option<&mut bool> {
        match provider {
            ProviderId::Claude => Some(&mut self.claude_pools_one_widget),
            ProviderId::Codex => Some(&mut self.codex_pools_one_widget),
            ProviderId::Antigravity => Some(&mut self.antigravity_pools_one_widget),
            _ => None,
        }
    }

    pub(super) fn is_dual_mode_active(&self, provider: ProviderId) -> bool {
        let (main_id, x_id) = match provider {
            ProviderId::Claude => ("default", "claude_x"),
            ProviderId::Codex => ("default", "codex_x"),
            _ => return false,
        };
        self.is_account_profile_enabled(provider, main_id, true)
            && self.is_account_profile_enabled(provider, x_id, false)
    }
}

#[derive(Clone, Debug)]
struct PendingMouseClick {
    surface_index: usize,
    object_id: String,
}

const RETRY_BASE_MS: u32 = 30_000; // 30 seconds

// Menu item IDs for update frequency
const IDM_FREQ_1MIN: u16 = 10;
const IDM_FREQ_5MIN: u16 = 11;
const IDM_FREQ_15MIN: u16 = 12;
const IDM_FREQ_1HOUR: u16 = 13;
const IDM_START_WITH_WINDOWS: u16 = 20;
const IDM_LOCK_TASKBAR: u16 = 21;
const IDM_SHOW_PACE: u16 = 22;
const IDM_SHOW_FABLE: u16 = 23;
const IDM_SHOW_FABLE_X: u16 = 24;
const IDM_CLAUDE_POOLS_ONE_WIDGET: u16 = 25;
const IDM_ANTIGRAVITY_POOLS_ONE_WIDGET: u16 = 26;
const IDM_CODEX_POOLS_ONE_WIDGET: u16 = 27;
const IDM_VERSION_ACTION: u16 = 31;
const IDM_LANG_SYSTEM: u16 = 100;
const IDM_LANG_FIRST: u16 = 101;
const IDM_DASHBOARD: u16 = 71;
// 62 stays the whole-provider Antigravity toggle used by saved custom menus.
pub(super) const IDM_PROVIDER_ANTIGRAVITY_GEMINI: u16 = 68;
pub(super) const IDM_PROVIDER_ANTIGRAVITY_EXTERNAL: u16 = 67;
pub(super) const IDM_PROVIDER_ANTIGRAVITY_X: u16 = 75;
pub(super) const IDM_PROVIDER_ANTIGRAVITY_A: u16 = 76;
pub(super) const IDM_PROVIDER_ANTIGRAVITY_EXTERNAL_X: u16 = 77;
pub(super) const IDM_PROVIDER_CODEX_MAIN: u16 = 69;
pub(super) const IDM_PROVIDER_CODEX_X: u16 = 70;
pub(super) const IDM_PROVIDER_CLAUDE_MAIN: u16 = 72;
pub(super) const IDM_PROVIDER_CLAUDE_X: u16 = 73;

const WM_DPICHANGED_MSG: u32 = 0x02E0;
const WM_APP_UPDATE_CHECK_COMPLETE: u32 = WM_APP + 2;
const TRAY_ICON_UPDATE_REPOSITION_SUPPRESS_MS: u64 = 750;
const WINDOW_STATE_INTERVAL_MS: u32 = 250;

fn language_menu_command_id(language: LanguageId) -> u16 {
    IDM_LANG_FIRST
        .checked_add(u16::try_from(language.index()).expect("language index exceeds u16"))
        .expect("language menu command id exceeds u16")
}

fn language_from_menu_command_id(command: u16) -> Option<LanguageId> {
    command
        .checked_sub(IDM_LANG_FIRST)
        .and_then(|index| LanguageId::from_index(index.into()))
}

fn open_web_url(hwnd: HWND, url: &str, failure_message: &'static str) {
    if !native_interop::open_web_url(Some(hwnd), url) {
        diagnose::log(failure_message);
    }
}

/// How often the watchdog thread polls for an explorer.exe restart (which
/// recreates the taskbar and wipes our tray-icon registration).
const TASKBAR_WATCH_INTERVAL_SECS: u64 = 2;

const TASKBAR_COLLISION_REPOSITION: usize = 2;

/// How long the UI thread may stay unresponsive before the hang watchdog treats
/// the widget as wedged and replaces it with a fresh process. Long enough that a
/// slow repaint or a busy shell never costs a relaunch.
const UI_HANG_RECOVERY_SECS: u64 = 30;

/// Consecutive unanswered probes required on top of that delay. Elapsed time
/// alone would also count a system sleep that began while a probe was failing,
/// and relaunch on wake after a single missed probe on either side of it.
const UI_HANG_MIN_SILENT_PROBES: u32 = 10;

/// A long scheduling gap (for example, system sleep) breaks a run of probes.
const UI_HANG_MAX_PROBE_GAP_SECS: u64 = 10;

/// How long a single liveness probe waits for the UI thread to answer.
const UI_PROBE_TIMEOUT_MS: u32 = 1_000;

/// Environment counter of back-to-back hang relaunches. A replacement that
/// freezes in turn, behind an explorer.exe that stays hung for instance, is
/// given twice as long as the previous one before the next attempt, instead of
/// being replaced every half minute for as long as the cause lasts.
const ENV_HANG_RELAUNCHES: &str = "CCUM_HANG_RELAUNCHES";

/// A hang relaunch only extends the streak when the replacement freezes within
/// this long of being started; past it, the next hang starts over at 30 s.
const HANG_STREAK_WINDOW_SECS: u64 = 600;

/// Cap on the doubling: 30 s, then 1, 2, 4 and at most 8 minutes.
const HANG_BACKOFF_MAX_DOUBLINGS: u32 = 4;

/// Stable controller handle published for the hang watchdog outside of `STATE`.
///
/// The hang watchdog has to diagnose a wedged UI thread even when that thread
/// is wedged while holding the state mutex, so it never takes that lock. Written
/// once, right after the window is created.
static CONTROLLER_HWND: AtomicIsize = AtomicIsize::new(0);

/// Prevent duplicate recovery attempts by the UI hang watchdog.
static RELAUNCH_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

static SUPPRESS_TRAY_REPOSITION_UNTIL: Mutex<Option<Instant>> = Mutex::new(None);

/// Current system DPI (96 = 100% scaling, 144 = 150%, 192 = 200%, etc.)
static CURRENT_DPI: AtomicU32 = AtomicU32::new(96);
static POLL_IN_FLIGHT: AtomicBool = AtomicBool::new(false);
static POLL_PENDING: AtomicBool = AtomicBool::new(false);

/// Re-query the monitor DPI for our window and update the cached value.
/// Uses GetDpiForWindow which returns the live DPI (unlike GetDpiForSystem
/// which is cached at process startup and never changes).
fn refresh_dpi() {
    let hwnd = {
        let state = lock_state();
        state.as_ref().map(|s| s.hwnd.to_hwnd())
    };
    if let Some(hwnd) = hwnd {
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        if dpi > 0 {
            CURRENT_DPI.store(dpi, Ordering::Relaxed);
        }
    }
}

fn display_scale(display_index: usize) -> f64 {
    let displays = native_interop::find_monitors();
    let Some(display) = displays
        .get(display_index)
        .copied()
        .or_else(|| displays.first().copied())
    else {
        return 1.0;
    };
    monitor_scale(display)
}

fn monitor_scale(display: native_interop::DisplayMonitor) -> f64 {
    let mut dpi_x = 96;
    let mut dpi_y = 96;
    if unsafe { GetDpiForMonitor(display.handle, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }
        .is_ok()
        && dpi_x > 0
    {
        (dpi_x as f64 / 96.0).clamp(0.25, 8.0)
    } else {
        let system_dpi = unsafe { GetDpiForSystem() };
        if system_dpi > 0 {
            (system_dpi as f64 / 96.0).clamp(0.25, 8.0)
        } else {
            1.0
        }
    }
}

fn migrated_theme_placement(legacy: LegacyPlacement) -> (usize, i32) {
    let displays = native_interop::find_monitors();
    let taskbars = native_interop::find_taskbars();
    let display_index = taskbars
        .get(legacy.taskbar_index)
        .or_else(|| taskbars.first())
        .and_then(|taskbar| native_interop::monitor_for_taskbar(&taskbars, &displays, taskbar.hwnd))
        .and_then(|monitor| {
            displays
                .iter()
                .position(|display| display.handle == monitor)
        })
        .unwrap_or_else(|| legacy.taskbar_index.min(displays.len().saturating_sub(1)));
    let offset_x = legacy_offset_to_theme_offset(legacy.tray_offset, display_scale(display_index));
    (display_index, offset_x)
}

fn legacy_offset_to_theme_offset(tray_offset: i32, scale: f64) -> i32 {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    -((tray_offset.max(0) as f64 / scale).round() as i32)
}

fn theme_surface_scale(theme: &ThemeDocument, surface_index: usize) -> f64 {
    let display_index = theme
        .surfaces
        .get(surface_index)
        .map(|surface| surface.placement.reference.display)
        .unwrap_or(theme.placement.reference.display);
    display_scale(display_index)
}

fn logical_host_dimension(physical: i32, scale: f64) -> u32 {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    (physical.max(1) as f64 / scale)
        .round()
        .clamp(1.0, u32::MAX as f64) as u32
}

fn scaled_theme_dimension(logical: u32, scale: f64) -> i32 {
    (logical as f64 * scale).round().clamp(1.0, 8192.0) as i32
}

/// Spacing below which two relaunches are treated as a storm (e.g. explorer.exe
/// crash-looping); when detected we back off instead of spawning in a tight loop.
const RELAUNCH_THROTTLE_SECS: u64 = 10;
const RELAUNCH_BACKOFF_SECS: u64 = 30;
/// Environment flag set on a relaunched child so it waits for the previous
/// instance's single-instance mutex instead of exiting immediately.
const ENV_RELAUNCH: &str = "CCUM_RELAUNCH";
/// Unix timestamp (seconds) of the relaunch that spawned this process, passed to
/// the child so it can detect a relaunch storm.
const ENV_LAST_RELAUNCH_UNIX: &str = "CCUM_LAST_RELAUNCH_UNIX";

/// A process restart is reserved for an unresponsive UI thread. Shell-hosted
/// surface loss is recovered in place by the controller.
fn relaunch_after_ui_hang(streak: u32) {
    if RELAUNCH_IN_PROGRESS.swap(true, Ordering::AcqRel) {
        return;
    }

    // Back off if we are relaunching very soon after the relaunch that spawned
    // us: that signals the shell is crash-looping, not a one-off restart.
    let now = now_unix_secs();
    let last = std::env::var(ENV_LAST_RELAUNCH_UNIX)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    if last != 0 && now.saturating_sub(last) < RELAUNCH_THROTTLE_SECS {
        diagnose::log_recovery("relaunch storm detected; backing off before relaunching");
        std::thread::sleep(Duration::from_secs(RELAUNCH_BACKOFF_SECS));
    }

    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            diagnose::log_recovery(format!(
                "watchdog: unable to resolve current executable: {error}"
            ));
            RELAUNCH_IN_PROGRESS.store(false, Ordering::Release);
            return;
        }
    };

    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // Without this the replacement truncates the log on startup and takes the
    // reason for its own relaunch with it - the one entry worth reading after
    // the widget has recovered on its own. Diagnostics switched on from the
    // dashboard leave no flag on the command line, and --diagnose-append is
    // only read alongside --diagnose, so both are carried over.
    if diagnose::is_enabled() {
        for flag in ["--diagnose", "--diagnose-append"] {
            if !args.iter().any(|arg| arg == flag) {
                args.push(flag.to_string());
            }
        }
    }
    let mut command = std::process::Command::new(exe);
    command
        .args(&args)
        .env(ENV_RELAUNCH, "1")
        .env(ENV_LAST_RELAUNCH_UNIX, now.to_string())
        .env_remove(ENV_HANG_RELAUNCHES);
    command.env(ENV_HANG_RELAUNCHES, streak.saturating_add(1).to_string());
    match command.spawn() {
        Ok(_) => {
            diagnose::log_recovery("watchdog: relaunched fresh instance, exiting old one");
            // Skip DLL detach on a wedged process: another thread may hold a
            // lock needed by detach code. Recovery diagnostics are already flushed.
            unsafe {
                let _ = TerminateProcess(GetCurrentProcess(), 0);
            }
            std::process::exit(0);
        }
        Err(error) => {
            diagnose::log_recovery(format!(
                "watchdog: unable to spawn relaunched instance: {error}"
            ));
            RELAUNCH_IN_PROGRESS.store(false, Ordering::Release);
        }
    }
}

/// Ask the UI thread to acknowledge an empty message.
///
/// The hang watchdog runs on its own thread, so `SendMessageTimeoutW` here is
/// answered by the window's owning thread: it comes back at once while the
/// message loop is pumping, and times out once that loop is parked inside a
/// call that never returns.
///
/// The probe is sent every cycle rather than gated behind `IsHungAppWindow`,
/// which stayed false while the UI thread of a taskbar-embedded widget was
/// suspended, and only turned true once a frozen widget had been orphaned.
fn ui_thread_responds(hwnd: HWND) -> bool {
    unsafe {
        // A missing controller is shutdown, not an unresponsive UI thread;
        // calling it responsive keeps the two verdicts from overlapping.
        if !IsWindow(Some(hwnd)).as_bool() {
            return true;
        }
        let mut acknowledged = 0usize;
        SendMessageTimeoutW(
            hwnd,
            WM_NULL,
            WPARAM(0),
            LPARAM(0),
            SMTO_ABORTIFHUNG,
            UI_PROBE_TIMEOUT_MS,
            Some(&mut acknowledged),
        )
        .0 != 0
    }
}

/// Relaunch the widget once its UI thread stops answering messages.
///
/// A UI thread parked in a call that never returns keeps every window handle
/// valid and every parent alive, so the surface watchdog sees nothing wrong
/// while the widget stops drawing and stops polling - in the case that prompted
/// this, for well over an hour. The probe runs on its own thread and never
/// takes the state mutex: a UI thread wedged while holding that lock is
/// precisely what it has to survive, and the surface watchdog does take it.
fn spawn_ui_hang_watchdog() {
    std::thread::spawn(move || {
        let mut silence: Option<UiSilence> = None;
        loop {
            std::thread::sleep(Duration::from_secs(TASKBAR_WATCH_INTERVAL_SECS));
            // Stepping through the UI thread in a debugger leaves it as silent
            // as a wedged one, and a relaunch would end the debugging session.
            if unsafe { IsDebuggerPresent() }.as_bool() {
                silence = None;
                continue;
            }
            let primary = SendHwnd(CONTROLLER_HWND.load(Ordering::Relaxed)).to_hwnd();
            if primary.is_invalid() || ui_thread_responds(primary) {
                silence = None;
                continue;
            }
            let now = Instant::now();
            let current = silence.get_or_insert_with(|| {
                UiSilence::new(
                    now,
                    hang_relaunch_streak(
                        std::env::var(ENV_HANG_RELAUNCHES).ok().as_deref(),
                        std::env::var(ENV_LAST_RELAUNCH_UNIX).ok().as_deref(),
                        now_unix_secs(),
                    ),
                )
            });
            current.record_probe(now);
            let delay = hang_recovery_delay(current.streak);
            // Elapsed time rather than a cycle count alone: a probe that times
            // out makes its own cycle longer than the sleep interval.
            if current.recovery_due(now) {
                let streak = current.streak;
                diagnose::log_recovery(format!(
                    "watchdog: UI thread stopped answering for {}s -> relaunching",
                    delay.as_secs()
                ));
                silence = None;
                relaunch_after_ui_hang(streak);
            }
        }
    });
}

/// An unbroken run of unanswered probes, with the streak it is measured against.
struct UiSilence {
    since: Instant,
    last_probe: Instant,
    silent_probes: u32,
    streak: u32,
}

impl UiSilence {
    fn new(now: Instant, streak: u32) -> Self {
        Self {
            since: now,
            last_probe: now,
            silent_probes: 0,
            streak,
        }
    }

    fn record_probe(&mut self, now: Instant) {
        if now.duration_since(self.last_probe) >= Duration::from_secs(UI_HANG_MAX_PROBE_GAP_SECS) {
            self.since = now;
            self.silent_probes = 0;
        }
        self.last_probe = now;
        self.silent_probes = self.silent_probes.saturating_add(1);
    }

    fn recovery_due(&self, now: Instant) -> bool {
        self.silent_probes >= UI_HANG_MIN_SILENT_PROBES
            && now.duration_since(self.since) >= hang_recovery_delay(self.streak)
    }
}

/// Back-to-back hang relaunches leading up to this process, read from the
/// variables its parent set when relaunching it. A process that has been
/// running past HANG_STREAK_WINDOW_SECS starts a fresh streak.
fn hang_relaunch_streak(
    hang_relaunches: Option<&str>,
    last_relaunch_unix: Option<&str>,
    now_unix: u64,
) -> u32 {
    let started = last_relaunch_unix
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    if started == 0 || now_unix.saturating_sub(started) >= HANG_STREAK_WINDOW_SECS {
        return 0;
    }
    hang_relaunches
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

/// Silence tolerated before a relaunch, doubled for each back-to-back hang.
fn hang_recovery_delay(streak: u32) -> Duration {
    Duration::from_secs(UI_HANG_RECOVERY_SECS << streak.min(HANG_BACKOFF_MAX_DOUBLINGS))
}

/// Detect explorer.exe restarts and recover from them.
///
/// Shell recovery is posted to the controller, which recreates missing surfaces
/// on their owning UI thread without disturbing polling or dashboard state.
fn spawn_taskbar_watchdog() {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(TASKBAR_WATCH_INTERVAL_SECS));
        let controller = controller_for(HWND::default());
        if controller.is_invalid() {
            return;
        }
        unsafe {
            let _ = PostMessageW(Some(controller), WM_APP_RECONCILE, WPARAM(0), LPARAM(0));
        }

        static LAST_WATCHDOG_TRAY_RECT: Mutex<Option<RECT>> = Mutex::new(None);
        let (reposition_target, current_tray_rect) = {
            let state = lock_state();
            if let Some(s) = state.as_ref() {
                let tray_hwnd = s.tray_notify_hwnd.map(|h| h.to_hwnd());
                let target_hwnd = controller_for(s.hwnd.to_hwnd());
                (
                    target_hwnd,
                    tray_hwnd.and_then(native_interop::get_window_rect_safe),
                )
            } else {
                (HWND::default(), None)
            }
        };
        if !reposition_target.is_invalid() && current_tray_rect.is_some() {
            let mut last_rect = LAST_WATCHDOG_TRAY_RECT
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if rect_changed(*last_rect, current_tray_rect) {
                *last_rect = current_tray_rect;
                unsafe {
                    let _ = PostMessageW(
                        Some(reposition_target),
                        native_interop::WM_APP_TRAY_REPOSITION,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
        }

        let collision_action = lock_state().as_ref().and_then(|state| {
            taskbar_collision_action(state)
                .map(|action| (controller_for(state.hwnd.to_hwnd()), action))
        });

        if let Some((target_hwnd, action)) = collision_action {
            unsafe {
                let _ = PostMessageW(
                    Some(target_hwnd),
                    native_interop::WM_APP_TASKBAR_COLLISION,
                    WPARAM(action),
                    LPARAM(0),
                );
            }
        }
    });
}

fn taskbar_collision_action(state: &AppState) -> Option<usize> {
    if state.dragging || state.pending_drag || state.is_switching_window_style {
        return None;
    }
    let taskbar = state.taskbar_hwnd?.to_hwnd();
    let bounds = native_interop::get_taskbar_rect(taskbar)?;
    // Do not interpret an auto-hide slide or reveal strip as a capacity change.
    if !unsafe { IsWindowVisible(taskbar).as_bool() } || !taskbar_collision::taskbar_shown(bounds) {
        return None;
    }
    if state.auto_ejected {
        let target = restored_dock_rect(state, taskbar, bounds)?;
        if taskbar_lock_applies(state) {
            return (!positioning::taskbar_ejection_required(bounds, target, false, true))
                .then_some(0);
        }
        let occupancy = taskbar_collision::cached(taskbar, bounds)?;
        let margin = if state.auto_ejected_for_capacity {
            0
        } else {
            (20.0 * CURRENT_DPI.load(Ordering::Relaxed) as f64 / 96.0).round() as i32
        };
        occupancy.can_restore(target, margin).then_some(0)
    } else if state.embedded
        && state
            .placement_override
            .as_ref()
            .is_none_or(|p| p.nest != "floating")
    {
        let widget = native_interop::get_window_rect_safe(state.hwnd.to_hwnd())?;
        let locked = taskbar_lock_applies(state);
        // A layout pass during a slide can leave a stale child offset. Restore
        // the configured position before deciding the layout cannot fit.
        if positioning::taskbar_ejection_required(bounds, widget, false, locked)
            && restored_dock_rect(state, taskbar, bounds).is_some_and(|target| {
                positioning::taskbar_reposition_required(bounds, widget, target, locked)
            })
        {
            return Some(TASKBAR_COLLISION_REPOSITION);
        }
        // Visibility fallback needs only native geometry; an unavailable or
        // stale accessibility sample must not leave the widget clipped.
        let overlaps_apps = if positioning::taskbar_surface_is_clipped(bounds, widget) || locked {
            false
        } else {
            taskbar_collision::cached(taskbar, bounds)?.overlaps_app_controls(widget)
        };
        positioning::taskbar_ejection_required(bounds, widget, overlaps_apps, locked).then_some(1)
    } else {
        None
    }
}

static STATE: Mutex<Option<AppState>> = Mutex::new(None);

/// Lock STATE safely, recovering from poisoned mutex
fn lock_state() -> MutexGuard<'static, Option<AppState>> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

fn theme_runtime_from_state(state: &AppState) -> ThemeRuntime {
    let (poll_ok, has_error) = poll_display_state(
        state.last_poll_ok,
        state.retry_count,
        state.auth_error_paused_polling,
        state.data.as_ref(),
    );
    let nest = if state.auto_ejected {
        SurfaceNest::Floating
    } else if let Some(ref ov) = state.placement_override {
        if ov.nest == "floating" {
            SurfaceNest::Floating
        } else {
            SurfaceNest::Taskbar
        }
    } else {
        SurfaceNest::Taskbar
    };
    let opacity = state.floating_card_opacity.unwrap_or(85);
    let runtime = ThemeRuntime::from_providers(state.providers)
        .with_poll_state(poll_ok, has_error)
        .with_language(state.language)
        .with_countdown(state.usage_countdown)
        .with_windows_default_font(state.use_windows_default_font)
        .with_show_pace(state.show_pace)
        .with_show_fable(state.show_fable)
        .with_show_fable_x(state.show_fable_x)
        .with_pool_layouts(
            state.claude_pools_one_widget,
            state.codex_pools_one_widget,
            state.antigravity_pools_one_widget,
        )
        .with_pool_split_support(
            state
                .active_theme
                .as_ref()
                .is_some_and(ThemeDocument::supports_pool_splitting),
        )
        .with_view_mode(state.view_mode)
        .with_mode_phase(state.mode_phase)
        .with_opencode_state(state.tiles[Tile::OpenCode].phase)
        .with_qoder_state(state.tiles[Tile::Qoder].phase)
        .with_agy_external_state(state.tiles[Tile::AgyExternal].phase)
        .with_split_hover_phases(state.tiles.split_phases())
        .with_nest(nest)
        .with_floating_card_opacity(opacity)
        .with_antigravity_pools(
            state.show_antigravity_gemini,
            effective_antigravity_external(state),
            state.tiles[Tile::Antigravity].phase,
        )
        .with_antigravity_external_x(state.show_antigravity_external_x)
        .with_antigravity_x(
            state.providers.contains(ProviderId::Antigravity)
                && state.show_antigravity_gemini
                && state.accounts.antigravity_x().is_some(),
        )
        .with_antigravity_a(state.accounts.antigravity_a_enabled())
        .with_claude_state(
            state.tiles[Tile::Claude].hovered,
            state.is_account_profile_enabled(ProviderId::Claude, "default", true),
            state.is_account_profile_enabled(ProviderId::Claude, "claude_x", false),
            state.tiles[Tile::Claude].phase,
        )
        .with_codex_state(
            state.tiles[Tile::Codex].hovered,
            state.is_account_profile_enabled(ProviderId::Codex, "default", true),
            state.is_account_profile_enabled(ProviderId::Codex, "codex_x", false),
            state.tiles[Tile::Codex].phase,
        );
    app_settings::widget_mode_keys()
        .zip(state.widget_modes.iter())
        .fold(runtime, |runtime, (key, &(_, phase))| {
            runtime.with_widget_mode_phase(key, phase)
        })
}

/// A transient outage can keep presenting the last real reading while its
/// retry runs. Authentication failures and failures without cached data still
/// need the explicit error state.
fn poll_display_state(
    last_poll_ok: bool,
    retry_count: u32,
    auth_error_paused_polling: bool,
    data: Option<&AppUsageData>,
) -> (bool, bool) {
    if let Some(data) = data.filter(|data| !data.accounts.is_empty()) {
        let has_error = data.accounts.iter().any(|account| account.error.is_some());
        return (
            !data.is_empty(),
            data.is_empty() && (has_error || retry_count > 0),
        );
    }
    let has_usable_stale_data = !auth_error_paused_polling
        && data.is_some_and(|data| data.iter().any(|(_, usage)| usage.stale));
    (
        last_poll_ok || has_usable_stale_data,
        // A partial-failure retry after an overall success is not an error.
        !last_poll_ok && retry_count > 0 && !has_usable_stale_data,
    )
}

fn effective_theme_from_state(state: &AppState) -> Option<ThemeDocument> {
    theme_with_placement(state, state.auto_ejected)
}

/// Bumped whenever `AppState::active_theme` is replaced; part of the
/// effective-theme cache key.
static ACTIVE_THEME_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[derive(PartialEq)]
struct EffectiveThemeKey {
    generation: u64,
    overrides: HashMap<theme_engine::MouseActionOverrideKey, theme_engine::Expression>,
    auto_ejected: bool,
    placement_override: Option<PlacementOverride>,
}

type EffectiveThemeCache = Option<(EffectiveThemeKey, std::sync::Arc<ThemeDocument>)>;
static EFFECTIVE_THEME_CACHE: Mutex<EffectiveThemeCache> = Mutex::new(None);

/// Per-frame effective theme (F2). Animation frames reuse the theme built by
/// the last render while the inputs it depends on are unchanged; a full render
/// always rebuilds it, which covers display, DPI and taskbar changes (they all
/// end in a full pass). Floating placement depends on per-frame data and
/// metrics, so it is never cached.
fn effective_theme_for_frame(
    state: &AppState,
    trigger: RenderTrigger,
) -> std::sync::Arc<ThemeDocument> {
    let floating = if state.auto_ejected {
        state.auto_ejected_origin.is_some()
    } else {
        state
            .placement_override
            .as_ref()
            .is_some_and(|p| p.nest == "floating")
    };
    let build = || {
        std::sync::Arc::new(
            effective_theme_from_state(state).unwrap_or_else(ThemeDocument::starter),
        )
    };
    if floating {
        return build();
    }
    let key = EffectiveThemeKey {
        generation: ACTIVE_THEME_GENERATION.load(Ordering::Relaxed),
        overrides: state.mouse_action_overrides.clone(),
        auto_ejected: state.auto_ejected,
        placement_override: state.placement_override.clone(),
    };
    let mut cache = EFFECTIVE_THEME_CACHE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if trigger == RenderTrigger::Animation {
        if let Some((cached_key, theme)) = cache.as_ref() {
            if *cached_key == key {
                return theme.clone();
            }
        }
    }
    let theme = build();
    *cache = Some((key, theme.clone()));
    theme
}

fn theme_with_placement(state: &AppState, auto_ejected: bool) -> Option<ThemeDocument> {
    let mut theme = state.active_theme.as_ref().map(|theme| {
        theme_engine::apply_mouse_action_overrides(theme, &state.mouse_action_overrides)
    })?;
    let floating = if auto_ejected {
        state.auto_ejected_origin.map(|point| (None, point))
    } else {
        state
            .placement_override
            .as_ref()
            .filter(|p| p.nest == "floating")
            .map(|p| {
                (
                    Some((p.monitor_index, p.monitor_device.clone())),
                    POINT {
                        x: p.screen_x,
                        y: p.screen_y,
                    },
                )
            })
    };
    if let Some((saved_monitor, point)) = floating {
        let displays = native_interop::find_monitors();
        let selected = saved_monitor
            .and_then(|(index, device)| {
                positioning::resolve_saved_monitor(&displays, index, device.as_deref())
            })
            .and_then(|index| displays.get(index).copied().map(|d| (index, d)))
            .unwrap_or_else(|| positioning::monitor_for_point(&displays, point));
        apply_floating_position(&mut theme, state, selected.0, selected.1, point);
    } else if let Some(p) = state
        .placement_override
        .as_ref()
        .filter(|p| p.nest == "taskbar")
    {
        let displays = native_interop::find_monitors();
        let index = positioning::resolve_saved_monitor(
            &displays,
            p.monitor_index,
            p.monitor_device.as_deref(),
        )
        .unwrap_or(0);
        let horizontal = taskbar_is_horizontal(index);
        let placement = if p.screen_x > 0 || p.screen_y > 0 {
            let offset = if horizontal { p.screen_x } else { p.screen_y };
            positioning::taskbar_dock_placement(index, offset, display_scale(index), horizontal)
        } else {
            positioning::dock_placement(index, p.tray_offset, display_scale(index), horizontal)
        };
        positioning::override_primary_placement(&mut theme, placement);
    }
    Some(theme)
}

fn apply_floating_position(
    theme: &mut ThemeDocument,
    state: &AppState,
    index: usize,
    display: native_interop::DisplayMonitor,
    point: POINT,
) {
    let mut placement = positioning::floating_placement(index);
    let saved_host = if state.auto_ejected {
        state.auto_ejected_host.as_ref()
    } else {
        state
            .placement_override
            .as_ref()
            .and_then(|p| p.floating_host.as_ref())
    };
    placement.host_dimensions =
        floating_host_for_theme(theme, saved_host).map(|host| (host.width, host.height));
    positioning::override_primary_placement(theme, placement.clone());
    let scale = monitor_scale(display);
    let runtime = theme_runtime_for_surface(theme, 0, theme_runtime_from_state(state));
    let frame = positioning::widget_frame_with_cached_metrics(
        theme,
        state.data.as_ref(),
        runtime,
        scale,
        state.prepared_metrics.as_ref(),
    );
    let offset = positioning::clamped_floating_offset(point, display.rect, &frame, scale);
    placement.offset_x = offset.x;
    placement.offset_y = offset.y;
    positioning::override_primary_placement(theme, placement);
}

/// Resolve the actual rectangle that will be restored, including authored
/// placement or a saved drag offset. The watchdog must test this same target.
fn restored_dock_rect(state: &AppState, taskbar: HWND, taskbar_rect: RECT) -> Option<RECT> {
    let theme = theme_with_placement(state, false)?;
    let surface = theme.surfaces.first()?;
    if surface
        .placement
        .nest
        .resolve(surface.placement.reference.region)
        != SurfaceNest::Taskbar
    {
        return None;
    }
    let displays = native_interop::find_monitors();
    let display = displays
        .get(surface.placement.reference.display)
        .or_else(|| displays.first())?;
    if native_interop::taskbar_for_monitor(
        &native_interop::find_taskbars(),
        &displays,
        display.handle,
    )
    .map(|candidate| candidate.hwnd)
        != Some(taskbar)
    {
        return None;
    }
    let runtime = theme_runtime_for_surface(&theme, 0, theme_runtime_from_state(state));
    let scale = monitor_scale(*display);
    let (frame, offsets) = restored_dock_geometry(state, &theme, runtime, scale);
    let mut placement = surface.placement.clone();
    placement.offset_x = offsets.offset_x;
    placement.offset_y = offsets.offset_y;
    let tray = native_interop::find_child_window(taskbar, "TrayNotifyWnd")
        .and_then(native_interop::get_window_rect_safe);
    Some(positioning::surface_screen_rect(
        &placement,
        frame.width,
        frame.height,
        scale,
        display.rect,
        Some(taskbar_rect),
        tray,
    ))
}

fn restored_dock_geometry(
    state: &AppState,
    theme: &ThemeDocument,
    runtime: ThemeRuntime,
    scale: f64,
) -> (
    positioning::WidgetFrame,
    theme_engine::ResolvedPlacementValues,
) {
    let metrics = state.prepared_metrics.as_ref().map_or_else(
        || theme_engine::display_metrics(theme, state.data.as_ref(), runtime),
        |metrics| theme_engine::metrics_for_phase(metrics.clone(), runtime),
    );
    let frame = positioning::widget_frame_with_cached_metrics(
        theme,
        state.data.as_ref(),
        runtime,
        scale,
        Some(&metrics),
    );
    let offsets = theme_engine::resolve_surface_placement_with_metrics(
        theme,
        0,
        state.data.as_ref(),
        runtime,
        &metrics,
    );
    (frame, offsets)
}

fn theme_has_floating_surface(theme: &ThemeDocument) -> bool {
    theme.surfaces.iter().any(|surface| {
        surface
            .placement
            .nest
            .resolve(surface.placement.reference.region)
            == SurfaceNest::Floating
    })
}

fn window_state_timer_required(state: &AppState) -> bool {
    state.custom_theme_enabled
        && effective_theme_from_state(state)
            .as_ref()
            .is_some_and(theme_has_floating_surface)
}

fn sync_window_state_timer(hwnd: HWND) {
    let required = lock_state()
        .as_ref()
        .is_some_and(window_state_timer_required);
    set_window_state_timer(hwnd, required);
}

fn set_window_state_timer(hwnd: HWND, required: bool) {
    {
        let mut state = lock_state();
        let Some(state) = state.as_mut() else {
            return;
        };
        if required == state.window_state_timer_active {
            return;
        }
        state.window_state_timer_active = required;
    }
    unsafe {
        if required {
            set_controller_timer(
                Some(hwnd),
                TIMER_WINDOW_STATE,
                WINDOW_STATE_INTERVAL_MS,
                None,
            );
        } else {
            let _ = kill_controller_timer(Some(hwnd), TIMER_WINDOW_STATE);
        }
    }
}

/// Serializes `save_state_settings` so two writers cannot interleave their
/// load-modify-save cycles; held across the file I/O instead of `STATE`.
static SETTINGS_IO: Mutex<()> = Mutex::new(());

fn save_state_settings() {
    // Lock order: SETTINGS_IO, then STATE. The state lock is held only while
    // copying fields, so file I/O never blocks other `STATE` users (F35).
    let _io = SETTINGS_IO.lock().unwrap_or_else(|e| e.into_inner());
    let mut persisted = load_settings();
    {
        let state = lock_state();
        let Some(s) = state.as_ref() else {
            return;
        };
        persisted.tray_offset = s.tray_offset;
        persisted.taskbar_index = s.taskbar_index;
        persisted.legacy_placement_pending = false;
        persisted.widget_visible = true;
        persisted.legacy_visibility_pending = false;
        persisted.poll_interval_ms = s.poll_interval_ms;
        persisted.language = s
            .language_override
            .map(|language| language.code().to_string());
        persisted.last_update_check_unix = s.last_update_check_unix;
        persisted.set_enabled_providers(s.providers);
        persisted.show_antigravity_gemini = Some(s.show_antigravity_gemini);
        persisted.show_antigravity_external = Some(s.show_antigravity_external);
        persisted.show_antigravity_external_x = Some(s.show_antigravity_external_x);
        persisted.custom_theme_enabled = s.custom_theme_enabled;
        persisted.show_pace = s.show_pace;
        persisted.show_fable = s.show_fable;
        persisted.show_fable_x = Some(s.show_fable_x);
        persisted.claude_pools_one_widget = s.claude_pools_one_widget;
        persisted.codex_pools_one_widget = s.codex_pools_one_widget;
        persisted.antigravity_pools_one_widget = s.antigravity_pools_one_widget;
        persisted.view_mode = s.view_mode;
        persisted.widget_modes = widget_mode_overrides(s.view_mode, &s.widget_modes);
        persisted.active_theme_path = s
            .active_theme_path
            .as_ref()
            .map(|path| path.to_string_lossy().to_string());
        persisted.placement_override = s.placement_override.clone();
        persisted.floating_card_opacity = s.floating_card_opacity;
        persisted.lock_taskbar = s.lock_taskbar;
        persisted.accounts = s.accounts.clone();
        // The dashboard process owns its dimensions, so leave the freshly
        // loaded values unchanged when monitor actions persist settings.
    }
    if let Err(error) = save_settings(&persisted) {
        diagnose::log(format!("unable to save settings: {error}"));
    }
}

fn save_settings_or_log(settings: &SettingsFile, context: &str) {
    if let Err(error) = save_settings(settings) {
        diagnose::log(format!("{context}: {error}"));
    }
}

pub(super) fn toggle_taskbar_lock(hwnd: HWND) {
    let enabled = {
        let mut state = lock_state();
        let Some(state) = state.as_mut() else {
            return;
        };
        state.lock_taskbar = !state.lock_taskbar;
        state.lock_taskbar
    };
    save_state_settings();
    if enabled {
        redock_locked_widget(hwnd);
    }
    diagnose::log(format!("taskbar lock toggled: {enabled}"));
}

fn taskbar_created_message() -> u32 {
    static MESSAGE: OnceLock<u32> = OnceLock::new();
    *MESSAGE.get_or_init(|| unsafe {
        let name = native_interop::wide_str("TaskbarCreated");
        RegisterWindowMessageW(PCWSTR::from_raw(name.as_ptr()))
    })
}

fn set_window_title(hwnd: HWND, strings: Strings) {
    unsafe {
        let title = native_interop::wide_str(strings.window_title);
        let _ = SetWindowTextW(hwnd, PCWSTR::from_raw(title.as_ptr()));
    }
}

fn show_info_message(hwnd: HWND, title: &str, message: &str) {
    unsafe {
        let title_wide = native_interop::wide_str(title);
        let message_wide = native_interop::wide_str(message);
        let _ = MessageBoxW(
            Some(hwnd),
            PCWSTR::from_raw(message_wide.as_ptr()),
            PCWSTR::from_raw(title_wide.as_ptr()),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

fn show_error_message(hwnd: HWND, title: &str, message: &str) {
    unsafe {
        let title_wide = native_interop::wide_str(title);
        let message_wide = native_interop::wide_str(message);
        let _ = MessageBoxW(
            Some(hwnd),
            PCWSTR::from_raw(message_wide.as_ptr()),
            PCWSTR::from_raw(title_wide.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

fn apply_language_to_state(state: &mut AppState, language_override: Option<LanguageId>) {
    invalidate_metrics(state);
    state.language_override = language_override;
    state.language = localization::resolve_language(language_override);
    set_window_title(state.hwnd.to_hwnd(), state.language.strings());
}

fn update_language_change() -> bool {
    let mut state = lock_state();
    let Some(app_state) = state.as_mut() else {
        return false;
    };

    if app_state.language_override.is_some() {
        return false;
    }

    let new_language = localization::detect_system_language();
    if new_language == app_state.language {
        return false;
    }

    apply_language_to_state(app_state, None);
    true
}

const STARTUP_REGISTRY_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const STARTUP_REGISTRY_KEY: &str = "ClaudeCodeUsageMonitor";

/// Returns true only if the startup registry value points to this executable.
pub(crate) fn is_startup_enabled() -> bool {
    unsafe {
        let path = native_interop::wide_str(STARTUP_REGISTRY_PATH);
        let key_name = native_interop::wide_str(STARTUP_REGISTRY_KEY);

        let mut hkey = HKEY::default();
        let result = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR::from_raw(path.as_ptr()),
            None,
            KEY_READ,
            &mut hkey,
        );
        if result.is_err() {
            return false;
        }

        // Query the size of the value
        let mut data_size: u32 = 0;
        let result = RegQueryValueExW(
            hkey,
            PCWSTR::from_raw(key_name.as_ptr()),
            None,
            None,
            None,
            Some(&mut data_size),
        );
        if result.is_err() || data_size == 0 {
            let _ = RegCloseKey(hkey);
            return false;
        }

        // Read the value
        let mut buf = vec![0u8; data_size as usize];
        let result = RegQueryValueExW(
            hkey,
            PCWSTR::from_raw(key_name.as_ptr()),
            None,
            None,
            Some(buf.as_mut_ptr()),
            Some(&mut data_size),
        );
        let _ = RegCloseKey(hkey);
        if result.is_err() {
            return false;
        }

        // Convert the registry value (UTF-16) to a string
        let wide_slice =
            std::slice::from_raw_parts(buf.as_ptr() as *const u16, data_size as usize / 2);
        let reg_value = String::from_utf16_lossy(wide_slice)
            .trim_end_matches('\0')
            .to_string();

        // Get the current executable path
        let mut exe_buf = [0u16; 260];
        let len = GetModuleFileNameW(None, &mut exe_buf) as usize;
        if len == 0 {
            return false;
        }
        let current_exe = String::from_utf16_lossy(&exe_buf[..len]);

        // Case-insensitive comparison (Windows paths are case-insensitive)
        reg_value.eq_ignore_ascii_case(&current_exe)
    }
}

pub(crate) fn set_startup_enabled(enable: bool) {
    unsafe {
        let path = native_interop::wide_str(STARTUP_REGISTRY_PATH);

        let mut hkey = HKEY::default();
        let result = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR::from_raw(path.as_ptr()),
            None,
            KEY_SET_VALUE,
            &mut hkey,
        );
        if result.is_err() {
            return;
        }

        let key_name = native_interop::wide_str(STARTUP_REGISTRY_KEY);

        if enable {
            let mut exe_buf = [0u16; 260];
            let len = GetModuleFileNameW(None, &mut exe_buf) as usize;
            if len > 0 {
                // Write the wide string including null terminator
                let byte_len = ((len + 1) * 2) as u32;
                let _ = RegSetValueExW(
                    hkey,
                    PCWSTR::from_raw(key_name.as_ptr()),
                    None,
                    REG_SZ,
                    Some(std::slice::from_raw_parts(
                        exe_buf.as_ptr() as *const u8,
                        byte_len as usize,
                    )),
                );
            }
        } else {
            let _ = RegDeleteValueW(hkey, PCWSTR::from_raw(key_name.as_ptr()));
        }

        let _ = RegCloseKey(hkey);
    }
}

fn total_widget_width_for_state(state: &AppState) -> i32 {
    widget_frame_for_state(state, None).width
}

fn widget_frame_for_state(state: &AppState, nest: Option<SurfaceNest>) -> positioning::WidgetFrame {
    effective_theme_from_state(state).as_ref().map_or(
        positioning::WidgetFrame {
            width: 1,
            height: 1,
            content_width: 1,
            inset: 0,
        },
        |theme| {
            let runtime = theme_runtime_for_surface(theme, 0, theme_runtime_from_state(state));
            let mut runtime = nest.map_or(runtime, |nest| runtime.with_nest(nest));
            if nest == Some(SurfaceNest::Floating) {
                if let Some(host) = floating_host_for_theme(theme, None) {
                    runtime = runtime.with_host_dimensions(host.width, host.height);
                }
            }
            let scale = theme_surface_scale(theme, 0);
            positioning::widget_frame_with_cached_metrics(
                theme,
                state.data.as_ref(),
                runtime,
                scale,
                state.prepared_metrics.as_ref(),
            )
        },
    )
}

fn floating_frame_for_state(
    state: &AppState,
    display_index: usize,
    scale: f64,
) -> Option<positioning::WidgetFrame> {
    let mut theme = effective_theme_from_state(state)?;
    let mut placement = positioning::floating_placement(display_index);
    placement.host_dimensions =
        floating_host_for_theme(&theme, None).map(|host| (host.width, host.height));
    positioning::override_primary_placement(&mut theme, placement);
    let runtime = theme_runtime_for_surface(&theme, 0, theme_runtime_from_state(state));
    Some(positioning::widget_frame(
        &theme,
        state.data.as_ref(),
        runtime,
        scale,
    ))
}

fn clear_auto_ejection(state: &mut AppState) {
    state.auto_ejected = false;
    state.auto_ejected_for_capacity = false;
    state.auto_ejected_origin = None;
    state.auto_ejected_host = None;
}

fn replace_active_theme(state: &mut AppState, loaded: ThemeDocument, path: Option<PathBuf>) {
    // A fallback belongs to the configured root that was ejected. Preserve it
    // during ordinary refreshes, but never carry it into a new theme or host.
    let changed = state.active_theme.as_ref().is_none_or(|current| {
        current.id != loaded.id
            || current.surfaces.first().map(|s| (&s.id, &s.placement))
                != loaded.surfaces.first().map(|s| (&s.id, &s.placement))
    }) || path
        .as_ref()
        .is_some_and(|p| state.active_theme_path.as_ref() != Some(p));
    if changed {
        clear_auto_ejection(state);
    }
    state.active_theme = Some(loaded);
    ACTIVE_THEME_GENERATION.fetch_add(1, Ordering::Relaxed);
    invalidate_metrics(state);
    invalidate_hover_tile_rects(state);
    if path.is_some() {
        state.active_theme_path = path;
    }
}

fn update_configured_placement(
    state: &mut AppState,
    placement: Option<PlacementOverride>,
    taskbar_index: usize,
    tray_offset: i32,
) {
    if state.placement_override != placement
        || state.taskbar_index != taskbar_index
        || state.tray_offset != tray_offset
    {
        clear_auto_ejection(state);
    }
    state.placement_override = placement;
    state.taskbar_index = taskbar_index;
    state.tray_offset = tray_offset;
}

fn apply_custom_theme(
    hwnd: HWND,
    _enabled: bool,
    path: Option<PathBuf>,
    document: Option<ThemeDocument>,
) -> Result<(), String> {
    let loaded = match (document, path.as_deref()) {
        (Some(document), _) => Some(document),
        (None, Some(path)) => Some(theme_engine::load_theme(path)?),
        (None, None) => lock_state()
            .as_ref()
            .and_then(|state| state.active_theme.clone()),
    };
    let loaded = loaded.unwrap_or_else(ThemeDocument::starter);
    let theme_clock_interval = loaded.current_time_refresh_interval();
    let tray_theme_uses_current_time = theme_tray_uses_current_time(&loaded);
    let old_hook = {
        let mut state = lock_state();
        let Some(state) = state.as_mut() else {
            return Err("Application is not ready".into());
        };
        state.custom_theme_enabled = true;
        replace_active_theme(state, loaded, path);
        state.theme_clock_interval = theme_clock_interval;
        state.tray_theme_uses_current_time = tray_theme_uses_current_time;
        state.mouse_action_overrides.clear();
        state.hovered_mouse_layer = None;
        state.pending_mouse_click = None;
        state.suppress_next_left_up = false;
        state.embedded = false;
        state.win_event_hook.take()
    };
    if let Some(hook) = old_hook {
        native_interop::unhook_win_event(hook.to_hook());
    }
    unsafe {
        native_interop::make_popup(hwnd, false);
        ensure_layered_window(hwnd);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_NOTOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
    sync_surface_windows();
    sync_window_state_timer(hwnd);
    schedule_countdown_timer();
    schedule_clock_timer();
    Ok(())
}

fn total_widget_height_for_state(state: &AppState) -> i32 {
    widget_frame_for_state(state, None).height
}

fn total_widget_height() -> i32 {
    lock_state()
        .as_ref()
        .map(total_widget_height_for_state)
        .unwrap_or(1)
}

fn total_widget_width() -> i32 {
    lock_state()
        .as_ref()
        .map(total_widget_width_for_state)
        .unwrap_or(1)
}

fn schedule_countdown_timer() {
    let state = lock_state();
    let s = match state.as_ref() {
        Some(s) => s,
        None => return,
    };

    let hwnd = s.hwnd.to_hwnd();
    if !s.last_poll_ok {
        unsafe {
            let _ = kill_controller_timer(Some(hwnd), TIMER_COUNTDOWN);
            let _ = kill_controller_timer(Some(hwnd), TIMER_RESET_POLL);
        }
        return;
    }

    // If a reset time has passed, poll every 5s to pick up fresh data
    if s.retry_count == 0 && s.data.as_ref().is_some_and(poller::app_is_past_reset) {
        unsafe {
            set_controller_timer(Some(hwnd), TIMER_RESET_POLL, 5_000, None);
        }
    }

    let min_delay = s.data.as_ref().and_then(|data| {
        data.all_usage()
            .flat_map(|usage| usage.sections())
            .filter_map(|section| poller::time_until_display_change(section.resets_at))
            .min()
    });

    let ms = min_delay
        .unwrap_or(Duration::from_secs(60))
        .as_millis()
        .max(1000) as u32;

    unsafe {
        set_controller_timer(Some(hwnd), TIMER_COUNTDOWN, ms, None);
    }
}

fn schedule_clock_timer() {
    let state = lock_state();
    let Some(s) = state.as_ref() else {
        return;
    };
    let hwnd = s.hwnd.to_hwnd();
    let Some(interval) = s.theme_clock_interval else {
        unsafe {
            let _ = kill_controller_timer(Some(hwnd), TIMER_CLOCK);
        }
        return;
    };
    let ms = time_until_next_clock_refresh(interval).as_millis().max(1) as u32;
    unsafe {
        set_controller_timer(Some(hwnd), TIMER_CLOCK, ms, None);
    }
}

fn time_until_next_clock_refresh(interval: Duration) -> Duration {
    let interval_ms = interval.as_millis().max(1);
    let elapsed_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);
    let remaining_ms = interval_ms - elapsed_ms % interval_ms;
    Duration::from_millis(remaining_ms as u64)
}

fn check_theme_change() {
    theme::invalidate_dark_mode_cache();
    let new_dark = theme::is_dark_mode();
    let changed = {
        let mut state = lock_state();
        if let Some(s) = state.as_mut() {
            if s.is_dark != new_dark {
                s.is_dark = new_dark;
                true
            } else {
                false
            }
        } else {
            false
        }
    };
    if changed {
        render_layered();
    }
}

fn check_language_change() {
    if update_language_change() {
        render_layered();
    }
}

const MODE_TRANSITION_DURATION_MS: u64 = 200;
const ANTIGRAVITY_CROSSFADE_TICK_MS: u32 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Easing {
    /// Constant-speed interpolation, kept as the reference behaviour for unit
    /// tests. Stage 3 moved the live hover animations to Smoothstep.
    #[cfg(test)]
    Linear,
    /// Ease-in-out now used by every live hover and view-mode animation.
    Smoothstep,
}

impl Easing {
    fn apply(self, progress: f64) -> f64 {
        let p = progress.clamp(0.0, 1.0);
        match self {
            #[cfg(test)]
            Easing::Linear => p,
            Easing::Smoothstep => p * p * (3.0 - 2.0 * p),
        }
    }
}

/// Pure interpolation used by every animation slot. Keeping the easing out of
/// the timer makes the choreography unit-testable without a clock.
pub(super) fn interpolate_phase(start: f64, target: f64, progress: f64, easing: Easing) -> f64 {
    start + (target - start) * easing.apply(progress)
}

#[derive(Clone, Copy, Debug)]
pub(super) struct PhaseAnimation {
    pub(super) start: f64,
    pub(super) target: f64,
    pub(super) start_time: std::time::Instant,
    pub(super) duration: std::time::Duration,
    pub(super) last_rendered: std::time::Instant,
    pub(super) easing: Easing,
}

impl PhaseAnimation {
    pub(super) fn new(
        start: f64,
        target: f64,
        duration: std::time::Duration,
        easing: Easing,
    ) -> Self {
        let now = std::time::Instant::now();
        Self {
            start,
            target,
            start_time: now,
            duration,
            last_rendered: now,
            easing,
        }
    }

    /// Reference linear interpolation, exercised by unit tests.
    #[cfg(test)]
    pub(super) fn linear(start: f64, target: f64, duration: std::time::Duration) -> Self {
        Self::new(start, target, duration, Easing::Linear)
    }

    /// Every live hover animation eases in and out over 200 ms.
    pub(super) fn hover(start: f64, target: f64, duration: std::time::Duration) -> Self {
        Self::new(start, target, duration, Easing::Smoothstep)
    }

    pub(super) fn current(&self) -> (f64, bool) {
        let elapsed = self.start_time.elapsed();
        if elapsed >= self.duration || self.duration.is_zero() {
            (self.target, false)
        } else {
            let progress = elapsed.as_secs_f64() / self.duration.as_secs_f64();
            (
                interpolate_phase(self.start, self.target, progress, self.easing),
                true,
            )
        }
    }

    pub(super) fn tick_slot(
        slot: &mut Option<Self>,
        value_slot: &mut f64,
        throttle: bool,
    ) -> (bool, bool) {
        let Some(mut anim) = *slot else {
            return (false, false);
        };
        let now = std::time::Instant::now();
        if throttle && now.duration_since(anim.last_rendered) < std::time::Duration::from_millis(15)
        {
            return (false, true);
        }
        let (value, more) = anim.current();
        *value_slot = value;
        anim.last_rendered = now;
        if more {
            *slot = Some(anim);
        } else {
            *value_slot = anim.target;
            *slot = None;
        }
        (true, more)
    }
}

/// Bounce the view-mode cycle at the ends: `1 -> 2 -> 3 -> 2 -> 1 -> ...`.
/// A click uses the widget's stored direction, so reaching either
/// end reverses the next click instead of stopping the cycle.
pub(super) fn next_view_mode(current: u8, direction: i8) -> (u8, i8) {
    let next = i16::from(current) + i16::from(direction);
    match next {
        4 => (2, -1),
        0 => (2, 1),
        n => (n.clamp(1, 3) as u8, direction),
    }
}

/// One wheel notch moves a single step towards the requested end: `+1`
/// (wheel down) heads to mode 3, `-1` (wheel up) to mode 1. Unlike a click,
/// the wheel overwrites the bounce direction explicitly, so it can reverse a
/// click cycle mid-way. A step past either boundary is ignored.
pub(super) fn wheel_view_mode(current: u8, toward: i8) -> Option<(u8, i8)> {
    if toward != 1 && toward != -1 {
        return None;
    }
    let next = i16::from(current) + i16::from(toward);
    (1..=3).contains(&next).then_some((next as u8, toward))
}

/// A view-mode segment always starts from the CURRENT phase, so an
/// interrupting click or wheel notch eases from the reached value instead of
/// jumping back to the previous mode's phase.
pub(super) fn mode_phase_animation(current: f64, mode: u8) -> PhaseAnimation {
    PhaseAnimation::hover(
        current,
        (f64::from(mode.clamp(1, 3)) - 1.0).clamp(0.0, 2.0),
        Duration::from_millis(MODE_TRANSITION_DURATION_MS),
    )
}

fn invalidate_metrics(state: &mut AppState) {
    state.prepared_metrics = None;
    invalidate_hover_tile_rects(state);
}

fn prepared_metrics(
    state: &mut AppState,
    theme: &ThemeDocument,
    runtime: ThemeRuntime,
) -> theme_engine::DisplayMetrics {
    let metrics = state
        .prepared_metrics
        .get_or_insert_with(|| theme_engine::display_metrics(theme, state.data.as_ref(), runtime));
    theme_engine::metrics_for_phase(metrics.clone(), runtime)
}

/// Drop cached provider hit-test rects when widget geometry changes.
fn invalidate_hover_tile_rects(state: &mut AppState) {
    state.tiles.clear_rects();
}

/// Toggle the opt-in Fable column and invalidate the geometry-dependent hover
/// cache. The column changes the root surface width in both built-in themes
/// (between Claude and Codex in Compact, last in Classic), so enabling or
/// disabling it shifts the provider hit-test rects.
fn toggle_show_fable_x(state: &mut AppState) {
    state.show_fable_x = !state.show_fable_x;
    invalidate_metrics(state);
    invalidate_hover_tile_rects(state);
}

/// The parent agy item (`gemini`) or its External models child. The parent
/// turns agy itself on or off and is checked only while agy is on, so after
/// the last pool was switched off one click brings agy back. The last
/// provider cannot be turned off. Returns true when the provider set changed.
fn toggle_antigravity_models(state: &mut AppState, gemini: bool) -> bool {
    let agy_on = state.providers.contains(ProviderId::Antigravity);
    if gemini {
        let target = !(state.show_antigravity_gemini && agy_on);
        if !target && state.providers.len() <= 1 && agy_on {
            return false;
        }
        set_antigravity_shown(state, target);
    } else {
        state.show_antigravity_external = !state.show_antigravity_external;
    }
    invalidate_metrics(state);
    let any_pool = (state.show_antigravity_gemini || state.show_antigravity_external)
        && state.providers.contains(ProviderId::Antigravity);
    if !any_pool {
        state.tiles.reset(Tile::Antigravity);
    }
    gemini
}

/// Agy on or off as a whole: the provider and its parent menu item move
/// together, so the menu never shows agy checked while it is off.
fn set_antigravity_shown(state: &mut AppState, on: bool) {
    state.providers.set(ProviderId::Antigravity, on);
    state.show_antigravity_gemini = on;
    if on {
        // agy back on with no pool chosen shows pool a, as before pools.
        if !state.accounts.antigravity_a_enabled() && state.accounts.antigravity_x().is_none() {
            state.accounts.set_antigravity_a_enabled(true);
        }
    } else {
        state.tiles.reset(Tile::Antigravity);
        state.tiles.reset(Tile::AgyExternal);
    }
}

/// Main and X pools of Claude and Codex: the X profile's id, menu name and
/// config directory, and the provider's tile.
fn account_pool_defaults(
    provider: ProviderId,
) -> Option<(&'static str, &'static str, &'static str, Tile)> {
    match provider {
        ProviderId::Claude => Some(("claude_x", "Claude Code X", "~/.claude_x", Tile::Claude)),
        ProviderId::Codex => Some(("codex_x", "Codex X", "~/.codex_x", Tile::Codex)),
        _ => None,
    }
}

/// "Claude (Main)" / "Claude Code X" and the Codex pair. A missing profile is
/// created first. Switching off the last pool turns the provider off (refused
/// when it is the only provider); switching a pool on while the provider is
/// off turns it on. Returns true when the provider set changed.
fn toggle_account_pool(state: &mut AppState, provider: ProviderId, main: bool) -> bool {
    let Some((x_id, x_name, x_config_dir, tile)) = account_pool_defaults(provider) else {
        return false;
    };
    let target_id = if main { "default" } else { x_id };
    let provider_on = state.providers.contains(provider);
    let pools = match provider {
        ProviderId::Claude => &mut state.accounts.claude,
        ProviderId::Codex => &mut state.accounts.codex,
        _ => return false,
    };
    if !pools.profiles.iter().any(|profile| profile.id == target_id) {
        pools.profiles.push(if main {
            crate::accounts::AccountProfile::default()
        } else {
            crate::accounts::AccountProfile {
                id: x_id.into(),
                name: x_name.into(),
                config_dir: x_config_dir.into(),
                credentials_path: String::new(),
                enabled: false,
            }
        });
        pools.used_ids.insert(target_id.into());
    }
    let main_on = provider_on && pools.is_profile_enabled("default", true);
    let x_on = provider_on && pools.is_profile_enabled(x_id, false);
    let (current, other) = if main {
        (main_on, x_on)
    } else {
        (x_on, main_on)
    };
    let target = !current;
    if !target && !other && state.providers.len() <= 1 && provider_on {
        // Keep at least one provider active.
        return false;
    }
    if let Some(profile) = pools
        .profiles
        .iter_mut()
        .find(|profile| profile.id == target_id)
    {
        profile.enabled = target;
    }
    let any_pool = target || other;
    let provider_changed = any_pool != provider_on;
    state.providers.set(provider, any_pool);
    if !any_pool {
        state.tiles.reset(tile);
    }
    // A pool switch can add or drop the Fable column, which moves every
    // widget's hover area.
    invalidate_metrics(state);
    provider_changed
}

/// "Antigravity (Main)": agy pool a, like Claude's main pool. Switching off
/// the last pool turns agy off (refused when agy is the only provider);
/// switching a pool on while agy is off turns agy on. Returns true when the
/// provider set changed.
fn toggle_antigravity_a(state: &mut AppState) -> bool {
    let provider_on = state.providers.contains(ProviderId::Antigravity);
    let a_on = provider_on && state.accounts.antigravity_a_enabled();
    let o_on = provider_on && state.accounts.antigravity_x().is_some();
    let target = !a_on;
    if !target && !o_on {
        if state.providers.len() <= 1 {
            return false;
        }
        state.accounts.set_antigravity_a_enabled(false);
        set_antigravity_shown(state, false);
        return true;
    }
    state.accounts.set_antigravity_a_enabled(target);
    reselect_antigravity_reading(state, target);
    if target && !provider_on {
        set_antigravity_shown(state, true);
        return true;
    }
    false
}

/// Re-point agy's reading at the pools now on, before the next poll lands:
/// pool A switched off hands agy to pool O at once, and pool A switched back
/// on drops pool O's figures instead of showing them as pool A's.
fn reselect_antigravity_reading(state: &mut AppState, pool_a_turned_on: bool) {
    if let Some(data) = state.data.as_mut() {
        if pool_a_turned_on {
            data.remove(ProviderId::Antigravity);
        }
        data.select_accounts(&state.accounts);
    }
}

/// Set agy pool o on or off; its profile keeps its place when off.
fn set_antigravity_x_enabled(accounts: &mut crate::accounts::AccountSettings, on: bool) {
    if accounts.antigravity_x().is_some() != on {
        toggle_antigravity_x(accounts);
    }
}

/// "Antigravity (OpenCode)": agy pool o, like Claude's X pool, with the same
/// last-pool rules as pool a. Returns true when the provider set changed.
fn toggle_antigravity_pool_o(state: &mut AppState) -> bool {
    let provider_on = state.providers.contains(ProviderId::Antigravity);
    let a_on = provider_on && state.accounts.antigravity_a_enabled();
    let o_on = provider_on && state.accounts.antigravity_x().is_some();
    let target = !o_on;
    if !target && !a_on {
        if state.providers.len() <= 1 {
            return false;
        }
        set_antigravity_x_enabled(&mut state.accounts, false);
        set_antigravity_shown(state, false);
        return true;
    }
    let was_o_only = state.accounts.antigravity_o_only();
    set_antigravity_x_enabled(&mut state.accounts, target);
    let changed = target && !provider_on;
    if changed {
        set_antigravity_shown(state, true);
    }
    // Pool O's figures stood for agy; pool A's own come with the poll.
    reselect_antigravity_reading(state, was_o_only);
    changed
}

/// Pool o's External models child, like Claude's per-pool Fable toggle.
fn toggle_antigravity_external_x(state: &mut AppState) {
    state.show_antigravity_external_x = !state.show_antigravity_external_x;
    invalidate_metrics(state);
}

/// External models of the pool that fills agy's single-pool layout: pool o's
/// choice while only pool o is on, otherwise pool a's.
fn effective_antigravity_external(state: &AppState) -> bool {
    if state.accounts.antigravity_o_only() {
        state.show_antigravity_external_x
    } else {
        state.show_antigravity_external
    }
}

/// Turn the OpenCode agy pool on or off. The profile is created on first use
/// and keeps its place in settings when turned off, like `claude_x`.
fn toggle_antigravity_x(accounts: &mut crate::accounts::AccountSettings) {
    let id = crate::accounts::ANTIGRAVITY_X_ID;
    if !accounts.antigravity.profiles.iter().any(|p| p.id == id) {
        accounts
            .antigravity
            .profiles
            .push(crate::accounts::AccountSettings::antigravity_x_profile());
        accounts.antigravity.used_ids.insert(id.into());
    }
    if let Some(profile) = accounts
        .antigravity
        .profiles
        .iter_mut()
        .find(|p| p.id == id)
    {
        profile.enabled = !profile.enabled;
    }
}

fn toggle_show_fable(state: &mut AppState) {
    state.show_fable = !state.show_fable;
    invalidate_metrics(state);
    invalidate_hover_tile_rects(state);
}

fn widget_modes_from_settings(
    settings: &SettingsFile,
) -> [(u8, f64); crate::theme_engine::WIDGET_SLOTS] {
    let mut modes = [(settings.view_mode, f64::from(settings.view_mode) - 1.0);
        crate::theme_engine::WIDGET_SLOTS];
    for (slot, key) in modes.iter_mut().zip(app_settings::widget_mode_keys()) {
        let mode = settings.widget_mode(key);
        *slot = (mode, f64::from(mode) - 1.0);
    }
    modes
}

/// Only modes that differ from `view_mode` are saved, so widgets without an
/// override keep following the shared default.
fn widget_mode_overrides(
    view_mode: u8,
    modes: &[(u8, f64); crate::theme_engine::WIDGET_SLOTS],
) -> std::collections::BTreeMap<String, u8> {
    let mut saved: std::collections::BTreeMap<String, u8> = app_settings::widget_mode_keys()
        .zip(modes.iter())
        .filter(|(_, &(mode, _))| mode != view_mode)
        .map(|(key, &(mode, _))| (key.to_owned(), mode))
        .collect();
    // Loading copies an agy override into a missing external-models key (the
    // migration for files saved before that key existed), so save the
    // external mode explicitly whenever agy has an override.
    if saved.contains_key("antigravity") {
        let external = app_settings::widget_mode_keys()
            .zip(modes.iter())
            .find(|(key, _)| *key == crate::theme_engine::AGY_EXTERNAL_KEY)
            .map(|(_, &(mode, _))| mode);
        if let Some(mode) = external {
            saved.insert(crate::theme_engine::AGY_EXTERNAL_KEY.to_owned(), mode);
        }
    }
    saved
}

/// Change the selected slots from their own current phases.
fn change_widget_mode(state: &mut AppState, index: usize, shift: bool, wheel: Option<i8>) -> bool {
    let Some(&(current, _)) = state.widget_modes.get(index) else {
        return false;
    };
    // With Shift a wheel step at an end still brings the other widgets to
    // the hit widget's mode, so Shift always means "all widgets in one mode".
    let next = match wheel {
        Some(toward) => {
            wheel_view_mode(current, toward).or_else(|| shift.then_some((current, toward)))
        }
        None => Some(next_view_mode(current, state.widget_directions[index])),
    };
    let Some((mode, direction)) = next else {
        return false;
    };
    let mut changed = false;
    for i in 0..state.widget_modes.len() {
        if !shift && i != index {
            continue;
        }
        state.widget_directions[i] = direction;
        if state.widget_modes[i].0 != mode {
            state.widget_modes[i].0 = mode;
            state.widget_animations[i] = Some(mode_phase_animation(state.widget_modes[i].1, mode));
            changed = true;
        }
    }
    // Once every widget is in one mode, it becomes the shared `view_mode`, so
    // the saved file has no overrides and a restart restores the shared phase.
    // The shared phase itself follows the widgets in `tick_animations`.
    if state.widget_modes.iter().all(|&(other, _)| other == mode) {
        state.view_mode = mode;
    }
    if changed {
        // Prepared metrics do not depend on the mode (`metrics_for_phase`
        // applies each phase), so only the hit-test rects are dropped.
        invalidate_hover_tile_rects(state);
    }
    changed
}

/// Themes that read the shared `display.*` row bindings see one phase. While
/// every widget is in the same mode it follows them (slot 0 as the source),
/// so Shift input animates such themes as before.
fn sync_shared_mode_phase(state: &mut AppState) {
    let (mode, phase) = state.widget_modes[0];
    if state.widget_modes.iter().all(|&(other, _)| other == mode) {
        state.mode_phase = phase;
    }
}

pub(super) fn apply_widget_mode(hwnd: HWND, index: usize, shift: bool, wheel: Option<i8>) {
    let changed = {
        let mut state = lock_state();
        state
            .as_mut()
            .is_some_and(|state| change_widget_mode(state, index, shift, wheel))
    };
    if !changed {
        return;
    }
    mouse::reset_mouse_move_throttle();
    save_state_settings();
    ensure_crossfade_timer(hwnd);
    render_layered_animation();
}

pub(super) fn stop_crossfade_timer_if_idle(hwnd: HWND) {
    if let Some(state) = lock_state().as_ref() {
        state.stop_crossfade_timer_if_idle(hwnd);
    }
}

pub(super) fn ensure_crossfade_timer(hwnd: HWND) {
    animation_timer::start(hwnd);
}

/// Advance every animation slot on the shared crossfade timer. One render
/// per tick instead of one per slot; the slot values are identical to the
/// three separate tick functions this replaces.
pub(super) fn tick_animations(hwnd: HWND, throttle: bool) {
    let (should_render, still_animating, mode_finished) = {
        let mut state = lock_state();
        let Some(state) = state.as_mut() else {
            return;
        };
        let mut render = false;
        let mut active = false;
        let mode_was_active = state.widget_animations.iter().any(Option::is_some);
        let (changed, more) = state.tiles.tick(throttle);
        render |= changed;
        active |= more;
        for (slot, (_, phase)) in state
            .widget_animations
            .iter_mut()
            .zip(state.widget_modes.iter_mut())
        {
            let (changed, more) = PhaseAnimation::tick_slot(slot, phase, throttle);
            render |= changed;
            active |= more;
        }
        sync_shared_mode_phase(state);
        (
            render,
            active,
            mode_was_active && state.widget_animations.iter().all(Option::is_none),
        )
    };
    if !still_animating {
        stop_crossfade_timer_if_idle(hwnd);
    }
    if mode_finished {
        // The provider hit-test rects were built for the previous width. Drop
        // them so the settle render below rebuilds them, then re-evaluate
        // hover: a shrinking, right-pinned window never receives a synthetic
        // WM_MOUSELEAVE for a cursor it moved out from under.
        if let Some(state) = lock_state().as_mut() {
            invalidate_hover_tile_rects(state);
        }
    }
    if should_render {
        if mode_finished {
            render_layered();
        } else {
            render_layered_animation();
        }
    }
    if mode_finished {
        // The hoverable objects can change even when the window geometry
        // does not, so recheck hover once the transition has settled.
        mouse::request_hover_revalidation(hwnd);
    }
}

/// Whether entering (or leaving) `tile` animates its phase. A tile whose
/// provider has nothing to reveal keeps its phase but still records hover.
fn tile_hover_animates(state: &AppState, tile: Tile, hovered: bool) -> bool {
    match tile {
        // The single-pool Fable row also follows Claude's hover phase. Keep
        // it current even while Fable is off, so toggling Fable over an
        // already-hovered tile immediately reveals the correct row.
        Tile::Claude | Tile::Split(_) => true,
        Tile::Codex => state.is_dual_mode_active(ProviderId::Codex),
        // Dual mode fades Gemini to Claude/GPT; a single pool fades to the
        // hidden one.
        Tile::Antigravity => {
            (state.show_antigravity_gemini || state.show_antigravity_external)
                && state.providers.contains(ProviderId::Antigravity)
        }
        Tile::OpenCode => state.providers.contains(ProviderId::OpenCode),
        // Only a second extra pool has anything to reveal on hover.
        Tile::Qoder => {
            let has_second_pool = state
                .data
                .as_ref()
                .and_then(|data| data.get(ProviderId::Qoder))
                .is_some_and(|usage| usage.week.is_some());
            state.providers.contains(ProviderId::Qoder) && (has_second_pool || !hovered)
        }
        Tile::AgyExternal => state.providers.contains(ProviderId::Antigravity) || !hovered,
    }
}

/// The cursor entered or left `tile`. Arms the animation timer, never renders.
fn set_tile_hover(hwnd: HWND, tile: Tile, hovered: bool) {
    let animating = lock_state().as_mut().is_some_and(|state| {
        let animate = tile_hover_animates(state, tile, hovered);
        state.tiles.set_hovered(tile, hovered, animate)
    });
    if animating {
        ensure_crossfade_timer(hwnd);
    }
}

fn reload_external_settings(hwnd: HWND) {
    let settings = load_settings();
    let language_override = settings.language.as_deref().and_then(LanguageId::from_code);
    let theme_path = settings.active_theme_path.as_ref().map(PathBuf::from);
    let (providers_changed, lock_enabled) = {
        let mut state = lock_state();
        let Some(state) = state.as_mut() else {
            return;
        };
        apply_external_settings(state, &settings, language_override, hwnd)
    };
    unsafe {
        set_controller_timer(Some(hwnd), TIMER_POLL, settings.poll_interval_ms, None);
    }
    let _ = apply_custom_theme(hwnd, settings.custom_theme_enabled, theme_path, None);
    if lock_enabled {
        // apply_custom_theme leaves the window as a popup; re-embed a
        // previously ejected widget so the lock takes effect at once.
        redock_locked_widget(hwnd);
    }
    if providers_changed {
        request_poll(hwnd);
    }
    sync_tray_icon(hwnd);
    position_at_taskbar();
    render_layered();
}

/// Copy reloaded settings into the state. Returns whether the provider or
/// account selection changed and whether the taskbar lock is on.
fn apply_external_settings(
    state: &mut AppState,
    settings: &SettingsFile,
    language_override: Option<LanguageId>,
    hwnd: HWND,
) -> (bool, bool) {
    let providers_changed =
        state.providers != settings.enabled_providers() || state.accounts != settings.accounts;
    let gemini_enabled = settings.antigravity_gemini_enabled();
    let external_enabled = settings.antigravity_external_enabled();
    state.show_antigravity_gemini = gemini_enabled;
    state.show_antigravity_external = external_enabled;
    state.show_antigravity_external_x = settings.antigravity_external_x_enabled();
    state.accounts = settings.accounts.clone();
    if let Some(data) = state.data.as_mut() {
        data.select_accounts(&settings.accounts);
    }
    state.poll_interval_ms = settings.poll_interval_ms;
    state.providers = settings.enabled_providers();
    state.usage_countdown = settings.usage_countdown;
    state.use_windows_default_font = settings.use_windows_default_font;
    state.show_pace = settings.show_pace;
    state.show_fable = settings.show_fable;
    state.show_fable_x = settings.show_fable_x.unwrap_or(settings.show_fable);
    state.claude_pools_one_widget = settings.claude_pools_one_widget;
    state.codex_pools_one_widget = settings.codex_pools_one_widget;
    state.antigravity_pools_one_widget = settings.antigravity_pools_one_widget;
    // An external settings reload replaces the transient view state too.
    state.view_mode = settings.view_mode;
    state.mode_phase = (f64::from(settings.view_mode) - 1.0).clamp(0.0, 2.0);
    state.widget_animations = [None; crate::theme_engine::WIDGET_SLOTS];
    state.widget_directions = [1; crate::theme_engine::WIDGET_SLOTS];
    state.widget_modes = widget_modes_from_settings(settings);
    reset_hidden_tiles(state);
    state.stop_crossfade_timer_if_idle(hwnd);
    invalidate_metrics(state);
    invalidate_hover_tile_rects(state);
    update_configured_placement(
        state,
        settings.placement_override.clone(),
        settings.taskbar_index,
        settings.tray_offset,
    );
    state.floating_card_opacity = settings.floating_card_opacity;
    state.lock_taskbar = settings.lock_taskbar;
    apply_language_to_state(state, language_override);
    (providers_changed, state.lock_taskbar)
}

/// Stop the hover of tiles whose provider (or pool) is no longer shown.
fn reset_hidden_tiles(state: &mut AppState) {
    let any_pool = (state.show_antigravity_gemini || state.show_antigravity_external)
        && state.providers.contains(ProviderId::Antigravity);
    for (tile, shown) in [
        (Tile::Antigravity, any_pool),
        (Tile::Codex, state.providers.contains(ProviderId::Codex)),
        (Tile::Claude, state.providers.contains(ProviderId::Claude)),
        (
            Tile::OpenCode,
            state.providers.contains(ProviderId::OpenCode),
        ),
        (Tile::Qoder, state.providers.contains(ProviderId::Qoder)),
        (
            Tile::AgyExternal,
            state.providers.contains(ProviderId::Antigravity),
        ),
    ] {
        if !shown {
            state.tiles.reset(tile);
        }
    }
}

/// Use the configured host, not the transient popup created by a drag or
/// auto-ejection. An intentional floating placement takes precedence over the
/// theme's authored host.
fn taskbar_lock_applies(state: &AppState) -> bool {
    if !state.lock_taskbar {
        return false;
    }
    match state.placement_override.as_ref().map(|p| p.nest.as_str()) {
        Some("taskbar") => true,
        Some("floating") => false,
        _ => state.active_theme.as_ref().is_some_and(|theme| {
            theme.surfaces.first().is_some_and(|surface| {
                surface
                    .placement
                    .nest
                    .resolve(surface.placement.reference.region)
                    == SurfaceNest::Taskbar
            })
        }),
    }
}

/// A locked taskbar widget dragged off the taskbar turns the lock off, so the
/// drop lands as an ordinary floating widget. A widget whose placement the
/// lock does not cover (already floating) keeps the user's preference.
/// Returns whether the lock was released.
fn release_taskbar_lock_for_desktop_drop(state: &mut AppState) -> bool {
    if !taskbar_lock_applies(state) {
        return false;
    }
    state.lock_taskbar = false;
    true
}

fn clear_locked_auto_ejection(state: &mut AppState) -> bool {
    if !taskbar_lock_applies(state) || !state.auto_ejected {
        return false;
    }
    clear_auto_ejection(state);
    true
}

/// Restore a locked taskbar widget after auto-ejection. Intentional floating,
/// desktop and tray hosts retain their placement.
fn redock_locked_widget(hwnd: HWND) {
    let taskbar_hwnd = {
        let mut state = lock_state();
        let Some(state) = state.as_mut() else {
            return;
        };
        // Locking suppresses collisions with app buttons, but must not restore
        // a mostly hidden layout into the parent. Use the same rule as ejection.
        if let Some(taskbar) = state.taskbar_hwnd.map(SendHwnd::to_hwnd) {
            let Some(bounds) = native_interop::get_taskbar_rect(taskbar) else {
                return;
            };
            let Some(target) = restored_dock_rect(state, taskbar, bounds) else {
                return;
            };
            if !taskbar_collision::taskbar_shown(bounds)
                || positioning::taskbar_ejection_required(bounds, target, false, true)
            {
                return;
            }
        }
        if !clear_locked_auto_ejection(state) {
            return;
        }
        state.is_switching_window_style = true;
        state.taskbar_hwnd.map(|h| h.to_hwnd())
    };
    // Re-embed first so position_at_taskbar sees a docked window.
    if let Some(taskbar) = taskbar_hwnd {
        native_interop::embed_as_child(hwnd, taskbar);
        diagnose::log("taskbar lock enabled: re-docked floating widget");
    }
    if let Some(state) = lock_state().as_mut() {
        state.is_switching_window_style = false;
    }
    position_at_taskbar();
    render_layered();
}

fn suppress_tray_reposition_for(duration: Duration) {
    let mut until = SUPPRESS_TRAY_REPOSITION_UNTIL
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    *until = Some(Instant::now() + duration);
}

fn tray_reposition_is_suppressed() -> bool {
    let now = Instant::now();
    let mut until = SUPPRESS_TRAY_REPOSITION_UNTIL
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    match *until {
        Some(deadline) if now < deadline => true,
        Some(_) => {
            *until = None;
            false
        }
        None => false,
    }
}

mod lifecycle;
use lifecycle::{controller_for, kill_controller_timer, set_controller_timer, WM_APP_RECONCILE};
mod host_geometry;
mod menu_commands;
mod message_loop;
mod poll;
mod render;
mod startup;
mod surfaces;
mod tray_status;
mod update_flow;

use host_geometry::*;
use menu_commands::*;
use message_loop::wnd_proc;
use poll::*;
use render::*;
pub use startup::run;
use surfaces::*;
use tray_status::*;
use update_flow::*;
mod positioning;
mod taskbar_collision;
use positioning::*;
mod animation_timer;
mod mouse;
mod tiles;
use tiles::Tile;
mod widget_tooltip;
use mouse::*;
mod window_context_menu;
use window_context_menu::*;

#[cfg(test)]
mod placement_tests;

#[cfg(test)]
mod layered_window_tests;

#[cfg(test)]
mod ui_hang_watchdog_tests {
    use super::*;
    use std::sync::mpsc;
    use windows::Win32::System::Threading::GetCurrentThreadId;

    #[test]
    fn recovery_requires_both_the_delay_and_consecutive_probes() {
        let start = Instant::now();
        let mut silence = UiSilence::new(start, 0);
        for seconds in 0..10 {
            let now = start + Duration::from_secs(seconds * 2);
            silence.record_probe(now);
            assert!(!silence.recovery_due(now));
        }
        for seconds in 10..=15 {
            let now = start + Duration::from_secs(seconds * 2);
            silence.record_probe(now);
        }
        assert!(silence.recovery_due(start + Duration::from_secs(30)));

        let mut sparse = UiSilence::new(start, 0);
        for seconds in [0, 9, 18, 27, 36] {
            let now = start + Duration::from_secs(seconds);
            sparse.record_probe(now);
            assert!(!sparse.recovery_due(now));
        }
    }

    #[test]
    fn sleep_breaks_silence_even_after_many_failed_probes() {
        let start = Instant::now();
        let mut silence = UiSilence::new(start, 0);
        for probe in 0..UI_HANG_MIN_SILENT_PROBES {
            silence.record_probe(start + Duration::from_secs(u64::from(probe) * 2));
        }
        let wake = start + Duration::from_secs(600);
        silence.record_probe(wake);
        assert_eq!(silence.silent_probes, 1);
        assert!(!silence.recovery_due(wake));
        for probe in 1..=15 {
            silence.record_probe(wake + Duration::from_secs(probe * 2));
        }
        assert!(silence.recovery_due(wake + Duration::from_secs(30)));
    }

    struct WindowThread {
        hwnd: SendHwnd,
        thread_id: u32,
        release: mpsc::Sender<()>,
        handle: std::thread::JoinHandle<()>,
    }

    /// Own a message-only window on a thread of its own. A pumping thread runs
    /// a message loop until WM_QUIT; a wedged one blocks without ever pumping
    /// until released, which is how a stuck UI thread looks from outside.
    fn spawn_window_thread(pumping: bool) -> WindowThread {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release, release_rx) = mpsc::channel::<()>();
        let handle = std::thread::spawn(move || unsafe {
            let class = native_interop::wide_str("STATIC");
            let hwnd = CreateWindowExW(
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
                None,
                None,
            )
            .expect("create isolated message-only test window");
            let ready = (SendHwnd::from_hwnd(hwnd), GetCurrentThreadId());
            if pumping {
                // Report ready from inside the loop, so a probe can never land
                // between the handshake and the first GetMessageW.
                PostThreadMessageW(ready.1, WM_APP, WPARAM(0), LPARAM(0))
                    .expect("queue the readiness marker");
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    if msg.message == WM_APP {
                        ready_tx.send(ready).expect("hand the test window back");
                    }
                    DispatchMessageW(&msg);
                }
            } else {
                ready_tx.send(ready).expect("hand the test window back");
                let _ = release_rx.recv();
            }
            let _ = DestroyWindow(hwnd);
        });
        let (hwnd, thread_id) = ready_rx.recv().expect("test window created");
        WindowThread {
            hwnd,
            thread_id,
            release,
            handle,
        }
    }

    #[test]
    fn hang_probe_tells_a_pumping_ui_thread_from_a_wedged_one() {
        let pumping = spawn_window_thread(true);
        let wedged = spawn_window_thread(false);

        assert!(ui_thread_responds(pumping.hwnd.to_hwnd()));
        assert!(!ui_thread_responds(wedged.hwnd.to_hwnd()));

        wedged.release.send(()).expect("release the wedged thread");
        wedged.handle.join().expect("wedged thread exits");
        unsafe {
            PostThreadMessageW(pumping.thread_id, WM_QUIT, WPARAM(0), LPARAM(0))
                .expect("stop the pumping thread");
        }
        pumping.handle.join().expect("pumping thread exits");
    }

    #[test]
    fn hang_probe_leaves_a_destroyed_window_to_the_surface_watchdog() {
        let gone = spawn_window_thread(false);
        let hwnd = gone.hwnd;
        gone.release.send(()).expect("release the window thread");
        gone.handle.join().expect("window thread exits");

        assert!(ui_thread_responds(hwnd.to_hwnd()));
    }

    #[test]
    fn back_to_back_hang_relaunches_double_the_tolerated_silence() {
        assert_eq!(hang_recovery_delay(0), Duration::from_secs(30));
        assert_eq!(hang_recovery_delay(1), Duration::from_secs(60));
        assert_eq!(hang_recovery_delay(4), Duration::from_secs(480));
        assert_eq!(hang_recovery_delay(9), Duration::from_secs(480));
    }

    #[test]
    fn a_hang_streak_only_carries_over_while_the_replacement_is_fresh() {
        let now = 1_000_000;
        let fresh = (now - 60).to_string();
        let stale = (now - 600).to_string();

        assert_eq!(hang_relaunch_streak(Some("2"), Some(&fresh), now), 2);
        assert_eq!(hang_relaunch_streak(Some("2"), Some(&stale), now), 0);
        assert_eq!(hang_relaunch_streak(None, Some(&fresh), now), 0);
        assert_eq!(hang_relaunch_streak(Some("2"), None, now), 0);
        assert_eq!(hang_relaunch_streak(Some("garbage"), Some(&fresh), now), 0);
    }
}

#[cfg(test)]
mod language_menu_tests {
    use super::*;

    #[test]
    fn generated_language_menu_commands_round_trip() {
        assert_eq!(language_from_menu_command_id(IDM_LANG_SYSTEM), None);
        for language in LanguageId::ALL {
            assert_eq!(
                language_from_menu_command_id(language_menu_command_id(language)),
                Some(language)
            );
        }
    }
}

#[cfg(test)]
mod poll_display_state_tests {
    use super::*;
    use crate::models::UsageData;

    fn cached_usage(stale: bool) -> AppUsageData {
        let usage = UsageData {
            stale,
            ..Default::default()
        };
        [(ProviderId::Claude, usage)].into_iter().collect()
    }

    #[test]
    fn transient_failure_keeps_a_stale_reading_displayable() {
        let data = cached_usage(true);
        assert_eq!(
            poll_display_state(false, 1, false, Some(&data)),
            (true, false)
        );
    }

    #[test]
    fn failures_without_stale_data_remain_errors() {
        let fresh = cached_usage(false);
        let stale = cached_usage(true);

        assert_eq!(poll_display_state(false, 1, false, None), (false, true));
        assert_eq!(poll_display_state(true, 1, false, None), (true, false));
        assert_eq!(
            poll_display_state(false, 1, false, Some(&fresh)),
            (false, true)
        );
        assert_eq!(
            poll_display_state(false, 1, true, Some(&stale)),
            (false, true)
        );
    }
}

#[cfg(test)]
mod placement_regression_tests;

#[cfg(test)]
mod credential_watch_worker_tests {
    use super::*;

    #[test]
    fn a_manual_refresh_queued_during_a_slow_watch_is_not_lost() {
        let in_flight = AtomicBool::new(true);
        let pending = AtomicBool::new(false);
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let in_flight = &in_flight;
            let pending = &pending;
            let worker = scope.spawn(move || {
                let mut calls = Vec::new();
                run_poll_worker(in_flight, pending, true, |scheduled| {
                    calls.push(scheduled);
                    if scheduled {
                        started_tx.send(()).unwrap();
                        resume_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                    }
                });
                calls
            });
            started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            // A second timer tick cannot acquire the worker while discovery is
            // blocked, but an explicit refresh can queue for that worker.
            assert!(in_flight
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err());
            pending.store(true, Ordering::Release);
            resume_tx.send(()).unwrap();
            assert_eq!(worker.join().unwrap(), [true, false]);
        });
        assert!(!in_flight.load(Ordering::Acquire));
        assert!(!pending.load(Ordering::Acquire));
    }
}

#[cfg(test)]
mod phase_animation_tests {
    use super::{
        interpolate_phase, invalidate_hover_tile_rects, mode_phase_animation, next_view_mode,
        wheel_view_mode, AppState, Easing, PhaseAnimation,
    };
    use std::time::Duration;

    #[test]
    fn tile_hover_arms_a_200ms_animation_without_rendering() {
        // Normalize line endings so a CRLF checkout does not break the split.
        let source = include_str!("window.rs").replace("\r\n", "\n");
        let setter = source
            .split("\nfn set_tile_hover(")
            .nth(1)
            .unwrap()
            .split("\n}\n")
            .next()
            .unwrap();
        assert!(!setter.contains("render_layered"));
        assert!(setter.contains("ensure_crossfade_timer(hwnd)"));

        let mut tiles = super::tiles::Tiles::default();
        for tile in super::Tile::ALL {
            assert!(tiles.set_hovered(tile, true, true));
            let animation = tiles[tile].animation.as_ref().unwrap();
            assert_eq!(animation.duration, Duration::from_millis(200));
            // agy fades its Gemini row out (1 -> 0); every other tile fades in.
            let expected = if tile == super::Tile::Antigravity {
                0.0
            } else {
                1.0
            };
            assert_eq!(animation.target, expected, "{tile:?}");
            assert!(
                !tiles.set_hovered(tile, true, true),
                "no restart while hovered"
            );
        }
    }

    #[test]
    fn tile_table_matches_the_theme_objects() {
        let ids = super::tiles::Tiles::object_ids();
        assert_eq!(
            &ids[..6],
            &[
                "antigravity-provider",
                "claude-provider",
                "codex-provider",
                "opencode-provider",
                "qoder-provider",
                crate::theme_engine::ANTIGRAVITY_EXTERNAL_WIDGET,
            ]
        );
        let mut tiles = super::tiles::Tiles::default();
        let bounds: Vec<_> = (0..ids.len())
            .map(|i| Some((i as f64, 0.0, 1.0, 1.0)))
            .collect();
        tiles.set_rects(0, &bounds);
        for (i, tile) in super::Tile::ALL.into_iter().enumerate() {
            assert_eq!(tiles.rect(0, tile), bounds[i], "{tile:?}");
        }
        for (i, pool) in crate::theme_engine::SplitPool::ALL.into_iter().enumerate() {
            assert_eq!(pool.index(), i, "SplitPool::ALL follows the phase order");
            assert_eq!(super::Tile::Split(pool).object_id(), pool.object_id());
        }
    }

    #[test]
    fn mirror_surfaces_keep_their_own_tile_rects() {
        // F56: a mirror's tiles sit elsewhere; its rects never borrow the
        // primary surface's, and a missing mirror rect is rebuilt.
        let mut tiles = super::tiles::Tiles::default();
        let primary = [Some((0.0, 0.0, 10.0, 10.0)); super::Tile::ALL.len()];
        let mut mirror = [Some((50.0, 0.0, 10.0, 10.0)); super::Tile::ALL.len()];
        mirror[1] = None;
        tiles.set_rects(0, &primary);
        assert!(tiles.rects_incomplete(1), "mirror rects not built yet");
        tiles.set_rects(1, &mirror);
        assert_eq!(tiles.rect(0, super::Tile::Codex), primary[2]);
        assert_eq!(tiles.rect(1, super::Tile::Codex), mirror[2]);
        assert_eq!(tiles.rect(1, super::Tile::Claude), None);
        assert!(!tiles.rects_incomplete(0));
        assert!(tiles.rects_incomplete(1));
        tiles.clear_rects();
        assert!(tiles.rects_incomplete(0) && tiles.rects_incomplete(1));
    }

    #[test]
    fn crossfade_animation_interpolates_and_completes() {
        let anim = PhaseAnimation::linear(1.0, 0.0, Duration::from_millis(0));
        let (value, more) = anim.current();
        assert_eq!(value, 0.0);
        assert!(!more);

        let anim = PhaseAnimation::linear(0.0, 1.0, Duration::from_secs(10));
        let (value, more) = anim.current();
        assert!(value < 0.1);
        assert!(more);

        let anim_return = PhaseAnimation::linear(0.0, 1.0, Duration::from_millis(0));
        let (value, more) = anim_return.current();
        assert_eq!(value, 1.0);
        assert!(!more);
    }

    #[test]
    fn linear_easing_matches_the_original_hover_crossfade() {
        for start in [0.0, 0.25, 1.0] {
            for target in [0.0, 1.0] {
                assert_eq!(interpolate_phase(start, target, 0.0, Easing::Linear), start);
                assert_eq!(
                    interpolate_phase(start, target, 1.0, Easing::Linear),
                    target
                );
                let mid = interpolate_phase(start, target, 0.5, Easing::Linear);
                assert_eq!(mid, (start + target) / 2.0);
            }
        }
    }

    #[test]
    fn smoothstep_eases_in_out_symmetrically_and_stays_monotonic() {
        assert_eq!(interpolate_phase(0.0, 1.0, 0.0, Easing::Smoothstep), 0.0);
        assert_eq!(interpolate_phase(0.0, 1.0, 1.0, Easing::Smoothstep), 1.0);
        assert_eq!(interpolate_phase(0.0, 1.0, 0.5, Easing::Smoothstep), 0.5);
        for step in [0.1, 0.2, 0.3, 0.4] {
            let early = interpolate_phase(0.0, 1.0, step, Easing::Smoothstep);
            let late = interpolate_phase(0.0, 1.0, 1.0 - step, Easing::Smoothstep);
            assert!(
                (early + late - 1.0).abs() < 1e-9,
                "{step}: {early} + {late}"
            );
        }
        let mut previous = 0.0_f64;
        for i in 0..=100 {
            let value = interpolate_phase(0.0, 1.0, i as f64 / 100.0, Easing::Smoothstep);
            assert!(value >= previous, "{i}: {value} < {previous}");
            assert!((0.0..=1.0).contains(&value));
            previous = value;
        }
        assert_eq!(interpolate_phase(0.0, 1.0, -1.0, Easing::Smoothstep), 0.0);
        assert_eq!(interpolate_phase(0.0, 1.0, 2.0, Easing::Smoothstep), 1.0);
    }

    #[test]
    fn interrupting_an_animation_restarts_from_the_current_value() {
        let reached = interpolate_phase(0.0, 2.0, 0.5, Easing::Smoothstep);
        let redirected =
            PhaseAnimation::new(reached, 0.0, Duration::from_millis(200), Easing::Smoothstep);
        assert_eq!(redirected.start, reached);
        assert_eq!(redirected.target, 0.0);
        assert_eq!(redirected.easing, Easing::Smoothstep);
        // The redirected animation really interpolates from the reached
        // value instead of jumping back to the old start.
        assert_eq!(
            interpolate_phase(reached, 0.0, 0.0, Easing::Smoothstep),
            reached
        );
        assert_eq!(
            interpolate_phase(reached, 0.0, 0.5, Easing::Smoothstep),
            reached / 2.0
        );
        assert_eq!(
            interpolate_phase(reached, 0.0, 1.0, Easing::Smoothstep),
            0.0
        );
    }

    #[test]
    fn completed_animation_clears_its_slot() {
        let mut slot = Some(PhaseAnimation::linear(0.0, 1.0, Duration::from_millis(0)));
        let mut value = 0.0;
        let (rendered, more) = PhaseAnimation::tick_slot(&mut slot, &mut value, false);
        assert!(rendered);
        assert!(!more);
        assert!(slot.is_none());
        assert_eq!(value, 1.0);

        let mut empty: Option<PhaseAnimation> = None;
        let mut idle = 0.0;
        assert_eq!(
            PhaseAnimation::tick_slot(&mut empty, &mut idle, false),
            (false, false)
        );
    }

    #[test]
    fn animation_timer_stays_active_until_every_slot_settles() {
        use super::placement_regression_tests::{placement, state_for};
        use crate::theme_engine::ThemeDocument;

        let mut state = state_for(ThemeDocument::starter(), placement("taskbar"));
        assert!(!state.is_any_animation_active());
        let active = || Some(PhaseAnimation::linear(0.0, 1.0, Duration::from_millis(200)));

        for tile in super::Tile::ALL {
            assert!(state.tiles.set_hovered(tile, true, true));
            assert!(state.is_any_animation_active(), "{tile:?}");
            state.tiles.reset(tile);
            assert!(!state.is_any_animation_active(), "{tile:?}");
        }
        for index in 0..crate::theme_engine::WIDGET_SLOTS {
            state.widget_animations[index] = active();
            assert!(state.is_any_animation_active(), "slot {index}");
            state.widget_animations[index] = None;
            assert!(!state.is_any_animation_active(), "slot {index}");
        }
    }

    #[test]
    fn view_mode_change_invalidates_cached_hover_tiles() {
        use super::placement_regression_tests::{placement, state_for};
        use crate::theme_engine::ThemeDocument;

        let mut state = state_for(ThemeDocument::starter(), placement("taskbar"));
        state
            .tiles
            .set_rects(0, &[Some((1.0, 2.0, 3.0, 4.0)); super::Tile::ALL.len()]);
        assert!(state.tiles.rect(0, super::Tile::Claude).is_some());

        invalidate_hover_tile_rects(&mut state);

        for tile in super::Tile::ALL {
            assert!(state.tiles.rect(0, tile).is_none(), "{tile:?}");
        }
    }

    #[test]
    fn show_fable_toggle_invalidates_cached_hover_rects() {
        use super::placement_regression_tests::{placement, state_for};
        use crate::theme_engine::ThemeDocument;

        fn seed_rects(state: &mut AppState) {
            state
                .tiles
                .set_rects(0, &[Some((1.0, 2.0, 3.0, 4.0)); super::Tile::ALL.len()]);
        }
        fn assert_rects_cleared(state: &AppState) {
            for tile in super::Tile::ALL {
                assert!(
                    state.tiles.rect(0, tile).is_none(),
                    "{tile:?} rect survives"
                );
            }
        }

        let mut state = state_for(ThemeDocument::starter(), placement("taskbar"));
        seed_rects(&mut state);
        let before = state.show_fable;

        super::toggle_show_fable(&mut state);
        assert_ne!(state.show_fable, before, "the setting must actually flip");
        assert_rects_cleared(&state);

        // A repaint rebuilds the rects for the new width. The reverse flip must
        // drop those freshly built rects too, not only the ones from before the
        // first toggle.
        seed_rects(&mut state);
        super::toggle_show_fable(&mut state);
        assert_eq!(state.show_fable, before, "the toggle must be reversible");
        assert_rects_cleared(&state);

        seed_rects(&mut state);
        let before_x = state.show_fable_x;
        super::toggle_show_fable_x(&mut state);
        assert_ne!(state.show_fable_x, before_x);
        assert_eq!(state.show_fable, before);
        assert_rects_cleared(&state);
    }

    #[test]
    fn live_hover_animations_use_smoothstep() {
        let anim = PhaseAnimation::hover(0.0, 1.0, Duration::from_millis(200));
        assert_eq!(anim.easing, Easing::Smoothstep);
        assert_eq!(anim.duration, Duration::from_millis(200));
        // The eased curve is slower at the ends and symmetric at the midpoint.
        assert_eq!(interpolate_phase(0.0, 1.0, 0.5, Easing::Smoothstep), 0.5);
        assert!(interpolate_phase(0.0, 1.0, 0.25, Easing::Smoothstep) < 0.25);
        assert!(interpolate_phase(0.0, 1.0, 0.75, Easing::Smoothstep) > 0.75);
    }

    #[test]
    fn asymmetric_opacities_avoid_double_exposure_under_eased_progress() {
        // Easing changes the timing but the asymmetric ramp must still keep
        // the two layer sets from being fully visible at the same time.
        for step in 0..=100 {
            let t = step as f64 / 100.0;
            let progress = interpolate_phase(0.0, 1.0, t, Easing::Smoothstep);
            let (normal, hover) = crate::theme_engine::asymmetric_crossfade_opacities(progress);
            assert!((0.0..=1.0).contains(&normal), "{t}: {normal}");
            assert!((0.0..=1.0).contains(&hover), "{t}: {hover}");
            assert!(normal + hover <= 1.0 + 1e-9, "{t}: {normal} + {hover}");
        }
    }

    #[test]
    fn widget_inputs_and_animation_are_independent() {
        use super::placement_regression_tests::{placement, state_for};
        let mut state = state_for(
            crate::theme_engine::ThemeDocument::starter(),
            placement("taskbar"),
        );
        assert!(super::change_widget_mode(&mut state, 0, false, None));
        assert_eq!(state.widget_modes[0].0, 2);
        assert!(state.widget_modes[1..].iter().all(|slot| slot.0 == 1));
        assert!(super::change_widget_mode(&mut state, 1, false, Some(1)));
        assert_eq!(state.widget_modes[1].0, 2);
        assert!(super::change_widget_mode(&mut state, 0, false, None));
        assert!(super::change_widget_mode(&mut state, 0, false, None));
        assert_eq!(state.widget_directions[0], -1);
        assert_eq!(state.widget_directions[1], 1);
        assert_eq!(state.widget_modes[0].0, 2);
        assert!(super::change_widget_mode(&mut state, 0, true, None));
        assert!(state.widget_modes.iter().all(|slot| slot.0 == 1));
        assert!(state
            .widget_directions
            .iter()
            .all(|&direction| direction == -1));
        assert_eq!(state.view_mode, 1);
    }

    #[test]
    fn shift_wheel_uses_hit_widget_as_source() {
        use super::placement_regression_tests::{placement, state_for};
        let mut state = state_for(
            crate::theme_engine::ThemeDocument::starter(),
            placement("taskbar"),
        );
        state.widget_modes[1] = (2, 1.0);
        assert!(super::change_widget_mode(&mut state, 1, true, Some(1)));
        assert!(state.widget_modes.iter().all(|slot| slot.0 == 3));
        assert_eq!(state.view_mode, 3);

        // The hit widget is already at the end: Shift still aligns the rest.
        state.widget_modes[4] = (1, 0.0);
        assert!(super::change_widget_mode(&mut state, 1, true, Some(1)));
        assert!(state.widget_modes.iter().all(|slot| slot.0 == 3));
        assert!(state.widget_animations[4].is_some());
        // Nothing left to change: no save, no render.
        state.widget_animations = [None; crate::theme_engine::WIDGET_SLOTS];
        assert!(!super::change_widget_mode(&mut state, 1, true, Some(1)));
        assert!(!super::change_widget_mode(&mut state, 1, false, Some(1)));

        // Individual changes that bring every widget to one mode make it the
        // shared mode, so nothing is saved as an override.
        state.view_mode = 1;
        state.widget_modes = [(2, 1.0); crate::theme_engine::WIDGET_SLOTS];
        state.widget_modes[3] = (1, 0.0);
        assert!(super::change_widget_mode(&mut state, 3, false, Some(1)));
        assert_eq!(state.view_mode, 2);
        assert!(super::widget_mode_overrides(state.view_mode, &state.widget_modes).is_empty());
    }

    #[test]
    fn shared_phase_follows_widgets_only_while_modes_agree() {
        use super::placement_regression_tests::{placement, state_for};
        let mut state = state_for(
            crate::theme_engine::ThemeDocument::starter(),
            placement("taskbar"),
        );
        state.widget_modes = [(3, 1.5); crate::theme_engine::WIDGET_SLOTS];
        super::sync_shared_mode_phase(&mut state);
        assert_eq!(state.mode_phase, 1.5);
        state.widget_modes[2] = (1, 0.0);
        state.widget_modes[0] = (3, 2.0);
        super::sync_shared_mode_phase(&mut state);
        assert_eq!(state.mode_phase, 1.5, "mixed modes keep the shared phase");
    }

    #[test]
    fn tick_only_advances_animated_widget() {
        let mut modes = [(1, 0.0); crate::theme_engine::WIDGET_SLOTS];
        let mut animations = [None; crate::theme_engine::WIDGET_SLOTS];
        animations[2] = Some(PhaseAnimation::linear(0.0, 1.0, Duration::ZERO));
        for (animation, (_, phase)) in animations.iter_mut().zip(modes.iter_mut()) {
            PhaseAnimation::tick_slot(animation, phase, false);
        }
        assert_eq!(modes[2].1, 1.0);
        assert!(modes
            .iter()
            .enumerate()
            .all(|(i, slot)| i == 2 || slot.1 == 0.0));
    }

    #[test]
    fn widget_mode_overrides_keep_only_differences() {
        let mut modes = [(2, 1.0); crate::theme_engine::WIDGET_SLOTS];
        assert!(super::widget_mode_overrides(2, &modes).is_empty());
        modes[0] = (3, 2.0);
        let saved = super::widget_mode_overrides(2, &modes);
        assert_eq!(saved.len(), 1);
        assert_eq!(saved.get("cursor"), Some(&3));
        let mut settings = crate::app_settings::SettingsFile::default();
        settings.view_mode = 2;
        settings.widget_modes = saved;
        assert_eq!(super::widget_modes_from_settings(&settings), modes);
        settings.view_mode = 1;
        let reloaded = super::widget_modes_from_settings(&settings);
        assert_eq!(reloaded[0], (3, 2.0));
        assert!(reloaded[1..].iter().all(|&slot| slot == (1, 0.0)));
    }

    #[test]
    fn agy_external_mode_equal_to_global_survives_save_and_reload() {
        let keys: Vec<_> = crate::app_settings::widget_mode_keys().collect();
        let agy = keys.iter().position(|k| *k == "antigravity").unwrap();
        let ext = keys
            .iter()
            .position(|k| *k == crate::theme_engine::AGY_EXTERNAL_KEY)
            .unwrap();
        let mut modes = [(2, 1.0); crate::theme_engine::WIDGET_SLOTS];
        modes[agy] = (3, 2.0);
        let mut settings = crate::app_settings::SettingsFile::default();
        settings.view_mode = 2;
        settings.widget_modes = super::widget_mode_overrides(2, &modes);
        let encoded = serde_json::to_string(&settings).unwrap();
        let mut reloaded: crate::app_settings::SettingsFile =
            serde_json::from_str(&encoded).unwrap();
        reloaded.normalize();
        let restored = super::widget_modes_from_settings(&reloaded);
        assert_eq!(restored[agy].0, 3);
        assert_eq!(restored[ext].0, 2);
    }

    #[test]
    fn split_widget_modes_change_independently_and_round_trip() {
        use super::placement_regression_tests::{placement, state_for};
        let mut state = state_for(
            crate::theme_engine::ThemeDocument::starter(),
            placement("taskbar"),
        );
        let keys: Vec<_> = crate::app_settings::widget_mode_keys().collect();
        for key in ["claude_x", "antigravity_o", "agy_external_o", "codex_x"] {
            let index = keys.iter().position(|candidate| *candidate == key).unwrap();
            assert!(super::change_widget_mode(&mut state, index, false, Some(1)));
            assert_eq!(state.widget_modes[index].0, 2);
        }
        assert_eq!(
            state.widget_modes[keys.iter().position(|key| *key == "claude").unwrap()].0,
            1
        );
        assert_eq!(
            state.widget_modes[keys.iter().position(|key| *key == "antigravity").unwrap()].0,
            1
        );
        let mut settings = crate::app_settings::SettingsFile::default();
        settings.widget_modes = super::widget_mode_overrides(state.view_mode, &state.widget_modes);
        let mut restored: crate::app_settings::SettingsFile =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        restored.normalize();
        for (saved, live) in super::widget_modes_from_settings(&restored)
            .iter()
            .zip(state.widget_modes.iter())
        {
            assert_eq!(saved.0, live.0);
        }
        let index = keys.iter().position(|key| *key == "claude_x").unwrap();
        assert!(super::change_widget_mode(&mut state, index, true, Some(1)));
        assert!(state.widget_modes.iter().all(|slot| slot.0 == 3));
    }

    #[test]
    fn view_mode_cycle_bounces_1_2_3_2_1() {
        let mut mode = 1u8;
        let mut direction = 1i8;
        let mut seen = vec![mode];
        for _ in 0..8 {
            let (next, dir) = next_view_mode(mode, direction);
            mode = next;
            direction = dir;
            seen.push(mode);
        }
        assert_eq!(seen, vec![1, 2, 3, 2, 1, 2, 3, 2, 1]);
        // At mode 1 the cycle has just reversed, so the next step goes up.
        assert_eq!(direction, -1);
        assert_eq!(next_view_mode(3, 1), (2, -1));
        assert_eq!(next_view_mode(1, -1), (2, 1));
        assert_eq!(next_view_mode(2, 1), (3, 1));
        assert_eq!(next_view_mode(2, -1), (1, -1));
    }

    #[test]
    fn wheel_steps_towards_each_end_and_ignores_impossible_steps() {
        // Wheel down (+1) heads to mode 3, wheel up (-1) heads to mode 1.
        assert_eq!(wheel_view_mode(1, 1), Some((2, 1)));
        assert_eq!(wheel_view_mode(2, 1), Some((3, 1)));
        assert_eq!(wheel_view_mode(3, 1), None, "no step past mode 3");
        assert_eq!(wheel_view_mode(3, -1), Some((2, -1)));
        assert_eq!(wheel_view_mode(2, -1), Some((1, -1)));
        assert_eq!(wheel_view_mode(1, -1), None, "no step below mode 1");
        // The wheel overwrites the bounce direction explicitly at any mode.
        assert_eq!(wheel_view_mode(2, 1), Some((3, 1)));
        assert_eq!(wheel_view_mode(2, -1), Some((1, -1)));
        // A zero delta or a stray magnitude is not a wheel step.
        assert_eq!(wheel_view_mode(2, 0), None);
        assert_eq!(wheel_view_mode(2, 2), None);
        assert_eq!(wheel_view_mode(2, -2), None);
    }

    #[test]
    fn mode_animation_eases_from_the_current_phase_over_200_ms() {
        let anim = mode_phase_animation(0.0, 3);
        assert_eq!(anim.start, 0.0);
        assert_eq!(anim.target, 2.0);
        assert_eq!(anim.easing, Easing::Smoothstep);
        assert_eq!(anim.duration, Duration::from_millis(200));
        // Interrupting mid-segment starts from the reached value, not from
        // the previous mode's endpoint.
        let reached = interpolate_phase(0.0, 2.0, 0.5, Easing::Smoothstep);
        let redirected = mode_phase_animation(reached, 1);
        assert_eq!(redirected.start, reached);
        assert_eq!(redirected.target, 0.0);
        assert_eq!(redirected.easing, Easing::Smoothstep);
        assert_eq!(redirected.duration, Duration::from_millis(200));
        // A hand-edited/out-of-range mode cannot leave the 0..2 phase.
        assert_eq!(mode_phase_animation(1.0, 0).target, 0.0);
        assert_eq!(mode_phase_animation(1.0, 9).target, 2.0);
    }

    #[test]
    fn mixed_click_and_wheel_inputs_keep_one_global_cycle() {
        let mut mode = 1u8;
        let mut direction = 1i8;
        let mut trace = vec![mode];

        // Two clicks: 1 -> 2 -> 3.
        for _ in 0..2 {
            let (next, dir) = next_view_mode(mode, direction);
            mode = next;
            direction = dir;
            trace.push(mode);
        }
        assert_eq!(trace, vec![1, 2, 3]);
        assert_eq!(direction, 1);

        // Wheel up from 3 sets the reverse direction explicitly: 3 -> 2.
        let (next, dir) = wheel_view_mode(mode, -1).expect("wheel up is possible at mode 3");
        mode = next;
        direction = dir;
        trace.push(mode);
        assert_eq!((mode, direction), (2, -1));

        // A click now continues back to 1 instead of bouncing forward again.
        let (next, dir) = next_view_mode(mode, direction);
        mode = next;
        direction = dir;
        trace.push(mode);
        assert_eq!((mode, direction), (1, -1));

        // Wheel up at the lower boundary is ignored; the next click reverses.
        assert_eq!(wheel_view_mode(mode, -1), None);
        let (next, dir) = next_view_mode(mode, direction);
        assert_eq!((next, dir), (2, 1));

        assert_eq!(trace, vec![1, 2, 3, 2, 1]);
    }
}

#[cfg(test)]
mod menu_command_id_tests {
    use super::*;

    /// A provider toggle that shares a command id with another menu item
    /// toggles that item instead (Qoder once reused Antigravity's 67).
    #[test]
    fn provider_menu_ids_do_not_collide_with_other_commands() {
        let fixed = [
            IDM_FREQ_1MIN,
            IDM_FREQ_5MIN,
            IDM_FREQ_15MIN,
            IDM_FREQ_1HOUR,
            IDM_START_WITH_WINDOWS,
            IDM_LOCK_TASKBAR,
            IDM_SHOW_PACE,
            IDM_SHOW_FABLE,
            IDM_SHOW_FABLE_X,
            IDM_VERSION_ACTION,
            IDM_DASHBOARD,
            IDM_PROVIDER_ANTIGRAVITY_GEMINI,
            IDM_PROVIDER_ANTIGRAVITY_EXTERNAL,
            IDM_PROVIDER_ANTIGRAVITY_X,
            IDM_PROVIDER_ANTIGRAVITY_A,
            IDM_PROVIDER_ANTIGRAVITY_EXTERNAL_X,
            IDM_PROVIDER_CODEX_MAIN,
            IDM_PROVIDER_CODEX_X,
            IDM_PROVIDER_CLAUDE_MAIN,
            IDM_PROVIDER_CLAUDE_X,
            IDM_LANG_SYSTEM,
        ];
        for provider in ProviderId::ALL {
            let id = provider.descriptor().native_menu_command_id;
            assert!(!fixed.contains(&id), "{provider:?} reuses command id {id}");
            assert!(id != 1 && id < IDM_LANG_FIRST, "{provider:?} id {id}");
        }
    }
}

#[cfg(test)]
mod antigravity_x_tests {
    use super::toggle_antigravity_x;
    use crate::accounts::{AccountSettings, ANTIGRAVITY_X_ID};

    #[test]
    fn the_opencode_pool_toggle_creates_its_profile_once_and_keeps_it_when_off() {
        let mut accounts = AccountSettings::default();
        assert!(accounts.antigravity_x().is_none());
        toggle_antigravity_x(&mut accounts);
        let profile = accounts.antigravity_x().unwrap();
        assert_eq!(profile.id, ANTIGRAVITY_X_ID);
        assert!(profile.config_dir.is_empty());
        assert!(accounts.antigravity.used_ids.contains(ANTIGRAVITY_X_ID));
        toggle_antigravity_x(&mut accounts);
        assert!(accounts.antigravity_x().is_none());
        assert_eq!(accounts.antigravity.profiles.len(), 1);
        toggle_antigravity_x(&mut accounts);
        assert_eq!(accounts.antigravity.profiles.len(), 1);
        assert!(accounts.antigravity_x().is_some());
        // Claude and Codex profiles are untouched.
        assert_eq!(accounts.claude, AccountSettings::default().claude);
        assert_eq!(accounts.codex, AccountSettings::default().codex);
    }

    #[test]
    fn a_hidden_duplicate_opencode_pool_adds_no_tray_error() {
        use crate::localization::LanguageId;
        use crate::models::{AccountUsage, AppUsageData};
        use crate::poller::{PollError, PollFailure};
        use crate::providers::{ProviderId, ProviderSet};
        let mut profile = AccountSettings::antigravity_x_profile();
        profile.enabled = true;
        let mut data = AppUsageData::default();
        data.accounts.push(AccountUsage {
            provider: ProviderId::Antigravity,
            profile,
            source_signature: "fixture".into(),
            source_path: None,
            usage: None,
            error: Some(PollError::NoCredentials),
            selected: false,
        });
        let providers = ProviderSet::from_enabled([ProviderId::Antigravity]);
        assert!(
            super::tray_error_lines(Some(&data), None, providers, LanguageId::English).is_empty()
        );
        // Pool A's own failure is still reported next to pool O.
        let failure = PollFailure {
            provider: ProviderId::Antigravity,
            error: PollError::AuthRequired,
        };
        assert_eq!(
            super::tray_error_lines(Some(&data), Some(failure), providers, LanguageId::English)
                .len(),
            1
        );
        // Pool A polled as an account while pool O is on: its error is listed
        // and a transient one schedules the usual partial retry.
        data.accounts.push(AccountUsage {
            provider: ProviderId::Antigravity,
            profile: AccountSettings::antigravity_a_profile(),
            source_signature: "fixture".into(),
            source_path: None,
            usage: None,
            error: Some(PollError::NetworkError),
            selected: true,
        });
        let lines = super::tray_error_lines(Some(&data), None, providers, LanguageId::English);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("agy (A)"), "{lines:?}");
        let mut accounts = AccountSettings::default();
        accounts
            .antigravity
            .profiles
            .push(AccountSettings::antigravity_x_profile());
        accounts.antigravity.profiles[0].enabled = true;
        assert!(
            super::partial_poll_retry_delay_ms(&data, providers, &accounts, 0, 900_000, 0)
                .is_some()
        );
    }
}

#[cfg(test)]
mod antigravity_pool_toggle_tests {
    use super::placement_regression_tests::{placement, state_for};
    use crate::providers::{ProviderId, ProviderSet};
    use crate::theme_engine::ThemeDocument;

    #[test]
    fn agy_pools_switch_independently_and_old_settings_keep_pool_a_on() {
        let old: crate::accounts::AccountSettings = serde_json::from_str("{}").unwrap();
        assert!(old.antigravity_a_enabled());
        let mut state = state_for(ThemeDocument::starter(), placement("taskbar"));
        state.providers = ProviderSet::from_enabled([ProviderId::Antigravity, ProviderId::Claude]);
        // Pool O on, then pool A off: agy stays on with pool O alone.
        super::toggle_antigravity_pool_o(&mut state);
        assert!(state.accounts.antigravity_x().is_some());
        assert!(!super::toggle_antigravity_a(&mut state));
        assert!(!state.accounts.antigravity_a_enabled());
        assert!(state.providers.contains(ProviderId::Antigravity));
        assert!(super::theme_runtime_from_state(&state).antigravity_o_only());
        // Pool A back on, without touching pool O.
        assert!(!super::toggle_antigravity_a(&mut state));
        assert!(state.accounts.antigravity_a_enabled());
        assert!(state.accounts.antigravity_x().is_some());
        // Last pool off turns agy off with both pools off, like Claude.
        super::toggle_antigravity_a(&mut state);
        assert!(super::toggle_antigravity_pool_o(&mut state));
        assert!(!state.providers.contains(ProviderId::Antigravity));
        assert!(!state.accounts.antigravity_a_enabled());
        assert!(state.accounts.antigravity_x().is_none());
        // "Antigravity (OpenCode)" alone brings agy back with only pool o.
        assert!(super::toggle_antigravity_pool_o(&mut state));
        assert!(state.providers.contains(ProviderId::Antigravity));
        assert!(state.accounts.antigravity_o_only());
        assert!(super::toggle_antigravity_pool_o(&mut state));
        assert!(!state.providers.contains(ProviderId::Antigravity));
        // As the only provider, the last pool cannot be switched off.
        state.providers = ProviderSet::from_enabled([ProviderId::Antigravity]);
        assert!(!super::toggle_antigravity_a(&mut state));
        assert!(state.accounts.antigravity_a_enabled());
        // The setting survives a settings round trip.
        state.accounts.set_antigravity_a_enabled(false);
        let saved = serde_json::to_string(&state.accounts).unwrap();
        let loaded: crate::accounts::AccountSettings = serde_json::from_str(&saved).unwrap();
        assert!(!loaded.antigravity_a_enabled());
    }

    #[test]
    fn agy_pool_a_off_makes_pool_o_the_agy_reading_from_poll_to_tray() {
        use crate::localization::LanguageId;
        use crate::poller::PollError;
        let mut settings = crate::accounts::AccountSettings::default();
        let mut profile = crate::accounts::AccountSettings::antigravity_x_profile();
        profile.credentials_path = "C:\\account-tests\\opencode.db".into();
        profile.enabled = true;
        settings.antigravity.profiles.push(profile);
        let agy = ProviderSet::from_enabled([ProviderId::Antigravity]);
        let fixture = |percentage: f64| crate::models::UsageData {
            session: crate::models::UsageSection {
                available: true,
                percentage,
                resets_at: None,
            },
            weekly: crate::models::UsageSection {
                available: true,
                percentage,
                resets_at: None,
            },
            ..Default::default()
        };
        // Both pools on: pool A (15 %) is agy's reading.
        let both = crate::poller::poll_accounts_for_test(agy, &settings, |_, path| {
            Ok(fixture(if path.is_some() { 70.0 } else { 15.0 }))
        })
        .unwrap();
        assert_eq!(
            both.get(ProviderId::Antigravity)
                .unwrap()
                .session
                .percentage,
            15.0
        );
        // Pool A off: only pool O is polled and it is agy's selected reading.
        settings.set_antigravity_a_enabled(false);
        let fresh = crate::poller::poll_accounts_for_test(agy, &settings, |_, path| {
            assert!(path.is_some(), "pool A is off and must not be polled");
            Ok(fixture(70.0))
        })
        .unwrap();
        let data = crate::poller::carry_forward_failures(fresh, &both, agy);
        assert_eq!(
            data.get(ProviderId::Antigravity)
                .unwrap()
                .session
                .percentage,
            70.0
        );
        assert!(data.antigravity_x().unwrap().selected);
        assert!(!data.accounts.iter().any(|a| a.is_antigravity_pool_a()));
        let lines = super::tray_usage_summary_lines(&data, agy, LanguageId::English, false);
        // Gemini line from pool O, plus its (empty) external-models line.
        assert!(lines[0].contains("5h: 70%"), "{lines:?}");
        assert!(lines.iter().all(|line| !line.contains("15%")), "{lines:?}");
        // Pool O failing with pool A off: one tray error, and no stale pool A
        // reading carried in its place.
        let failed = crate::poller::poll_accounts_for_test(agy, &settings, |_, _| {
            Err(PollError::AuthRequired)
        });
        let failed = match failed {
            Ok(data) => data,
            Err(_) => panic!("an account failure is published as data"),
        };
        let failed = crate::poller::carry_forward_failures(failed, &both, agy);
        assert!(failed.get(ProviderId::Antigravity).is_none());
        let failure = crate::poller::PollFailure {
            provider: ProviderId::Antigravity,
            error: PollError::AuthRequired,
        };
        let errors =
            super::tray_error_lines(Some(&failed), Some(failure), agy, LanguageId::English);
        assert_eq!(errors.len(), 1, "{errors:?}");
    }

    #[test]
    fn pool_o_reading_is_never_shown_or_carried_as_pool_a_after_a_settings_reload() {
        use crate::poller::PollError;
        let mut o_only = crate::accounts::AccountSettings::default();
        let mut profile = crate::accounts::AccountSettings::antigravity_x_profile();
        profile.credentials_path = "C:\\account-tests\\opencode.db".into();
        profile.enabled = true;
        o_only.antigravity.profiles.push(profile);
        o_only.set_antigravity_a_enabled(false);
        let agy = ProviderSet::from_enabled([ProviderId::Antigravity]);
        let fixture = |percentage: f64| crate::models::UsageData {
            session: crate::models::UsageSection {
                available: true,
                percentage,
                resets_at: None,
            },
            ..Default::default()
        };
        let shown =
            crate::poller::poll_accounts_for_test(agy, &o_only, |_, _| Ok(fixture(70.0))).unwrap();
        assert!(shown.antigravity_from_pool_o());
        // The marker survives the usage cache.
        let cached: crate::models::AppUsageData =
            serde_json::from_str(&serde_json::to_string(&shown).unwrap()).unwrap();
        assert_eq!(cached, shown);
        // A settings reload (not the menu) turns pool A back on next to pool O.
        let mut both = o_only.clone();
        both.set_antigravity_a_enabled(true);
        for reselect in [true, false] {
            let mut previous = cached.clone();
            if reselect {
                // What `reload_external_settings` does with the new accounts.
                previous.select_accounts(&both);
                assert!(previous.get(ProviderId::Antigravity).is_none());
            }
            assert!(previous.antigravity_pool_a_reading().is_none());
            // Pool A fails transiently; pool O answers again.
            let fresh = crate::poller::poll_accounts_for_test(agy, &both, |_, path| match path {
                Some(_) => Ok(fixture(70.0)),
                None => Err(PollError::NetworkError),
            })
            .unwrap();
            let merged = crate::poller::merge_poll_progress(fresh, &previous, &both);
            let pool_a = merged
                .accounts
                .iter()
                .find(|account| account.is_antigravity_pool_a())
                .unwrap();
            assert!(pool_a.usage.is_none(), "reselect={reselect}");
            assert!(
                merged.get(ProviderId::Antigravity).is_none(),
                "pool A shows pool O's figures (reselect={reselect})"
            );
            assert!(!merged.antigravity_from_pool_o());
            assert_eq!(
                merged
                    .antigravity_x()
                    .unwrap()
                    .usage
                    .as_ref()
                    .unwrap()
                    .session
                    .percentage,
                70.0
            );
            let carried = crate::poller::carry_forward_failures(
                crate::models::AppUsageData::default(),
                &previous,
                agy,
            );
            assert!(carried.get(ProviderId::Antigravity).is_none());
        }
    }

    #[test]
    fn agy_pool_switches_reselect_the_shown_reading_at_once() {
        let mut state = state_for(ThemeDocument::starter(), placement("taskbar"));
        state.providers = ProviderSet::from_enabled([ProviderId::Antigravity, ProviderId::Claude]);
        let mut data = crate::models::AppUsageData::default();
        data.insert(ProviderId::Antigravity, crate::models::UsageData::default());
        state.data = Some(data);
        super::toggle_antigravity_pool_o(&mut state);
        // Pool A off before any pool O poll: pool A's figures disappear.
        super::toggle_antigravity_a(&mut state);
        assert!(state
            .data
            .as_ref()
            .unwrap()
            .get(ProviderId::Antigravity)
            .is_none());
    }

    #[test]
    fn agy_menu_mirrors_claude_with_a_per_pool_external_models_toggle() {
        use crate::context_menu::ContextMenuAction as Action;
        let checked = |state: &super::AppState, action: Action| {
            super::window_context_menu::context_menu_checked(state, &action)
        };
        let mut state = state_for(ThemeDocument::starter(), placement("taskbar"));
        state.providers = ProviderSet::from_enabled([ProviderId::Antigravity, ProviderId::Claude]);
        state.show_antigravity_gemini = true;
        state.show_antigravity_external = true;
        state.show_antigravity_external_x = false;
        assert!(checked(&state, Action::ToggleAntigravityA));
        assert!(!checked(&state, Action::ToggleAntigravityX));
        super::toggle_antigravity_pool_o(&mut state);
        assert!(checked(&state, Action::ToggleAntigravityX));
        // Each pool's External models is its own toggle.
        assert!(checked(&state, Action::ToggleAntigravityExternal));
        assert!(!checked(&state, Action::ToggleAntigravityExternalX));
        let runtime = super::theme_runtime_from_state(&state);
        assert!(runtime.antigravity_external_enabled);
        assert!(!runtime.antigravity_external_x_enabled);
        super::toggle_antigravity_external_x(&mut state);
        assert!(checked(&state, Action::ToggleAntigravityExternalX));
        // Pool a off: pool o fills the single layout with its own choice.
        state.show_antigravity_external_x = false;
        super::toggle_antigravity_a(&mut state);
        let runtime = super::theme_runtime_from_state(&state);
        assert!(runtime.antigravity_o_only());
        assert!(!runtime.antigravity_external_enabled);
        // Turning a pool off keeps its External models value.
        assert!(state.show_antigravity_external);
    }

    #[test]
    fn pool_layout_menu_checks_and_runtime_follow_settings() {
        use crate::context_menu::ContextMenuAction as Action;
        let mut state = state_for(ThemeDocument::starter(), placement("taskbar"));
        for (claude, agy) in [(true, true), (false, true), (true, false), (false, false)] {
            state.claude_pools_one_widget = claude;
            state.codex_pools_one_widget = claude;
            state.antigravity_pools_one_widget = agy;
            assert_eq!(
                super::window_context_menu::context_menu_checked(
                    &state,
                    &Action::ToggleClaudePoolsOneWidget
                ),
                claude
            );
            assert_eq!(
                super::window_context_menu::context_menu_checked(
                    &state,
                    &Action::ToggleAntigravityPoolsOneWidget
                ),
                agy
            );
            assert_eq!(
                super::window_context_menu::context_menu_checked(
                    &state,
                    &Action::ToggleCodexPoolsOneWidget
                ),
                claude
            );
            let runtime = super::theme_runtime_from_state(&state);
            assert_eq!(runtime.claude_pools_one_widget, claude);
            assert_eq!(runtime.codex_pools_one_widget, claude);
            assert_eq!(runtime.antigravity_pools_one_widget, agy);
        }
    }

    #[test]
    fn old_settings_give_both_pools_the_single_external_models_choice() {
        for old in [true, false] {
            let json = format!(r#"{{"show_antigravity":true,"show_antigravity_external":{old}}}"#);
            let settings: crate::app_settings::SettingsFile = serde_json::from_str(&json).unwrap();
            assert_eq!(settings.antigravity_external_enabled(), old);
            assert_eq!(settings.antigravity_external_x_enabled(), old);
            assert!(settings.accounts.antigravity_a_enabled());
        }
        let settings: crate::app_settings::SettingsFile = serde_json::from_str(
            r#"{"show_antigravity_external":true,"show_antigravity_external_x":false}"#,
        )
        .unwrap();
        assert!(settings.antigravity_external_enabled());
        assert!(!settings.antigravity_external_x_enabled());
    }

    #[test]
    fn last_agy_pool_off_unchecks_the_parent_and_one_parent_click_restores_agy() {
        use crate::context_menu::ContextMenuAction;
        let checked = |state: &super::AppState, action: ContextMenuAction| {
            super::window_context_menu::context_menu_checked(state, &action)
        };
        for last_pool_is_o in [false, true] {
            let mut state = state_for(ThemeDocument::starter(), placement("taskbar"));
            state.providers =
                ProviderSet::from_enabled([ProviderId::Antigravity, ProviderId::Claude]);
            state.show_antigravity_gemini = true;
            if last_pool_is_o {
                // Pool O on, pool A off, then pool O off.
                super::toggle_antigravity_pool_o(&mut state);
                super::toggle_antigravity_a(&mut state);
                super::toggle_antigravity_pool_o(&mut state);
            } else {
                super::toggle_antigravity_a(&mut state);
            }
            assert!(!state.providers.contains(ProviderId::Antigravity));
            assert!(!checked(&state, ContextMenuAction::ToggleAntigravityGemini));
            // One click on the parent brings agy back with pool A on.
            assert!(super::toggle_antigravity_models(&mut state, true));
            assert!(state.providers.contains(ProviderId::Antigravity));
            assert!(checked(&state, ContextMenuAction::ToggleAntigravityGemini));
            assert!(checked(&state, ContextMenuAction::ToggleAntigravityA));
            assert!(state.accounts.antigravity_a_enabled());
            assert!(state.accounts.antigravity_x().is_none());
            // A second click turns it off again, as before.
            super::toggle_antigravity_models(&mut state, true);
            assert!(!state.providers.contains(ProviderId::Antigravity));
        }
        // As the only provider, the parent cannot be turned off.
        let mut state = state_for(ThemeDocument::starter(), placement("taskbar"));
        state.providers = ProviderSet::from_enabled([ProviderId::Antigravity]);
        state.show_antigravity_gemini = true;
        super::toggle_antigravity_models(&mut state, true);
        assert!(state.providers.contains(ProviderId::Antigravity));
    }

    #[test]
    fn agy_external_widget_mode_is_saved_and_old_settings_load_unchanged() {
        let key = crate::theme_engine::AGY_EXTERNAL_KEY;
        let legacy =
            serde_json::from_str::<crate::app_settings::SettingsFile>(r#"{"view_mode":2}"#)
                .unwrap();
        assert_eq!(legacy.widget_mode(key), 2);
        let modes = super::widget_modes_from_settings(&legacy);
        assert!(modes.iter().all(|&(mode, _)| mode == 2));
        let loaded = serde_json::from_str::<crate::app_settings::SettingsFile>(
            r#"{"view_mode":1,"widget_modes":{"agy_external":3}}"#,
        )
        .unwrap();
        assert_eq!(loaded.widget_mode(key), 3);
        assert_eq!(loaded.widget_mode("antigravity"), 1);
        // Changing it moves only its own slot.
        let mut state = state_for(ThemeDocument::starter(), placement("taskbar"));
        let index = crate::app_settings::widget_mode_keys()
            .position(|candidate| candidate == key)
            .unwrap();
        assert!(super::change_widget_mode(&mut state, index, false, None));
        for (slot, &(mode, _)) in state.widget_modes.iter().enumerate() {
            assert_eq!(mode, if slot == index { 2 } else { 1 }, "slot {slot}");
        }
    }
}
