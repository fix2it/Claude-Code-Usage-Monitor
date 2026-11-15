use super::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, TME_NONCLIENT, TME_QUERY, VK_SHIFT,
};

pub(super) fn surface_index_for_window(state: &AppState, hwnd: HWND) -> Option<usize> {
    if state.hwnd.to_hwnd() == hwnd {
        return Some(0);
    }
    if let Some(index) = state
        .mirror_hwnds
        .iter()
        .position(|window| window.to_hwnd() == hwnd)
    {
        return Some(index + 1);
    }
    state
        .desktop_hwnds
        .iter()
        .position(|window| window.is_some_and(|window| window.to_hwnd() == hwnd))
}

pub(super) fn mouse_client_point(lparam: LPARAM) -> (f64, f64) {
    let packed = lparam.0 as u32;
    let x = (packed as u16 as i16) as f64;
    let y = ((packed >> 16) as u16 as i16) as f64;
    (x, y)
}

pub(super) fn mouse_target_at(hwnd: HWND, lparam: LPARAM) -> Option<(usize, String)> {
    let mut state = lock_state();
    let state = state.as_mut()?;
    let surface_index = surface_index_for_window(state, hwnd)?;
    let theme = effective_theme_from_state(state)?;
    let scale = theme_surface_scale(&theme, surface_index).max(0.01);
    let runtime = theme_runtime_for_surface(&theme, surface_index, theme_runtime_from_state(state));
    let primary_runtime = theme_runtime_for_surface(&theme, 0, theme_runtime_from_state(state));
    let metrics =
        theme_engine::metrics_for_phase(prepared_metrics(state, &theme, primary_runtime), runtime);
    let (x, y) = mouse_client_point(lparam);
    let object_id = theme_engine::hit_test_mouse_event_with_metrics(
        &theme,
        surface_index,
        x / scale,
        y / scale,
        state.data.as_ref(),
        runtime,
        &metrics,
    )?;
    Some((surface_index, object_id))
}

/// Resolve the nearest <key>-provider ancestor in the selected surface.
fn widget_key_for_object<'a>(
    surface: &theme_engine::SceneObject,
    object_id: &str,
) -> Option<&'a str> {
    let mut current = Some(object_id);
    for _ in 0..=surface.children.len() {
        let id = current?;
        // The agy external-models widget has its own mode slot.
        if id == theme_engine::ANTIGRAVITY_EXTERNAL_WIDGET {
            return Some(theme_engine::AGY_EXTERNAL_KEY);
        }
        if let Some(key) =
            app_settings::widget_mode_keys().find(|key| id == format!("{key}-provider"))
        {
            return Some(key);
        }
        current = surface
            .children
            .iter()
            .find(|object| object.id == id)
            .and_then(|object| object.parent.as_deref());
    }
    None
}

/// True when the surface has at least one `<key>-provider` container.
fn surface_has_widgets(surface: &theme_engine::SceneObject) -> bool {
    surface.children.iter().any(|object| {
        object.id == theme_engine::ANTIGRAVITY_EXTERNAL_WIDGET
            || app_settings::widget_mode_keys().any(|key| object.id == format!("{key}-provider"))
    })
}

/// Which widgets an input changes: `(source slot, all widgets)`. A theme
/// without `<key>-provider` containers keeps the old rule: any hit changes
/// every widget, with slot 0 as the source.
fn mode_target_at(lparam: LPARAM, surface_index: usize) -> Option<(usize, bool)> {
    let mut guard = lock_state();
    let state = guard.as_mut()?;
    let theme = effective_theme_from_state(state)?;
    let surface = theme.surfaces.get(surface_index)?;
    if !surface_has_widgets(surface) {
        return Some((0, true));
    }
    // One rule for every theme: the topmost visible provider container
    // under the cursor owns the input, whatever object the event hit.
    let scale = theme_surface_scale(&theme, surface_index).max(0.01);
    let runtime = theme_runtime_for_surface(&theme, surface_index, theme_runtime_from_state(state));
    let primary = theme_runtime_for_surface(&theme, 0, theme_runtime_from_state(state));
    let metrics =
        theme_engine::metrics_for_phase(prepared_metrics(state, &theme, primary), runtime);
    let (x, y) = mouse_client_point(lparam);
    let id = theme_engine::hit_test_widget_with_metrics(
        &theme,
        surface_index,
        x / scale,
        y / scale,
        state.data.as_ref(),
        runtime,
        &metrics,
    )?;
    let key = widget_key_for_object(surface, &id)?;
    app_settings::widget_mode_keys()
        .position(|candidate| candidate == key)
        .map(|index| (index, false))
}

fn shift_held() -> bool {
    unsafe { GetKeyState(i32::from(VK_SHIFT.0)) < 0 }
}

pub(super) fn mouse_handler_exists(
    surface_index: usize,
    object_id: &str,
    event: MouseEventKind,
) -> bool {
    let state = lock_state();
    state.as_ref().is_some_and(|state| {
        state.active_theme.as_ref().is_some_and(|theme| {
            theme_engine::mouse_event_script(theme, surface_index, object_id, event).is_some()
        })
    })
}

pub(super) fn dispatch_mouse_event(
    surface_index: usize,
    object_id: &str,
    event: MouseEventKind,
) -> bool {
    let source = {
        let state = lock_state();
        let Some(state) = state.as_ref() else {
            return false;
        };
        let Some(theme) = state.active_theme.as_ref() else {
            return false;
        };
        let Some(source) = theme_engine::mouse_event_script(theme, surface_index, object_id, event)
            .map(str::to_string)
        else {
            return false;
        };
        source
    };
    execute_mouse_action_source(surface_index, object_id, &source)
}

pub(super) fn execute_mouse_action_source(
    surface_index: usize,
    object_id: &str,
    source: &str,
) -> bool {
    let result = {
        let mut state = lock_state();
        let Some(state) = state.as_mut() else {
            return false;
        };
        let Some(theme) = state.active_theme.clone() else {
            return false;
        };
        let data = state.data.clone();
        let runtime =
            theme_runtime_for_surface(&theme, surface_index, theme_runtime_from_state(state));
        let metrics = state.prepared_metrics.clone();
        let overrides_before = state.mouse_action_overrides.clone();
        let result = theme_engine::execute_mouse_actions_with_metrics(
            &theme,
            surface_index,
            object_id,
            source,
            data.as_ref(),
            runtime,
            &mut state.mouse_action_overrides,
            metrics.as_ref(),
        );
        let changed = state.mouse_action_overrides != overrides_before;
        let needs_render = result
            .as_ref()
            .map_or(changed, |effects| action_needs_render(effects, changed));
        if needs_render {
            invalidate_metrics(state);
        }
        (result, needs_render)
    };
    let (result, needs_render) = result;
    match result {
        Ok(effects) => {
            let owner = lock_state()
                .as_ref()
                .map(|state| state.hwnd.to_hwnd())
                .unwrap_or_default();
            // A plain right-click only opens the menu: re-measuring and
            // re-rendering first only delays it.
            if needs_render {
                render_layered();
                sync_tray_icon(owner);
            }
            for effect in effects {
                match effect {
                    MouseActionEffect::ShowDashboard => {
                        crate::dashboard::show(controller_for(owner))
                    }
                    MouseActionEffect::ToggleDashboard => {
                        crate::dashboard::toggle(controller_for(owner))
                    }
                    MouseActionEffect::OpenUrl(url) => {
                        open_web_url(owner, &url, "mouse action URL could not be opened")
                    }
                    MouseActionEffect::ShowContextMenu(menu) => show_context_menu_document(
                        owner,
                        menu.as_deref(),
                        Some((surface_index, object_id.to_string())),
                    ),
                }
            }
            true
        }
        Err(error) => {
            diagnose::log(format!(
                "mouse action failed surface={surface_index} layer={object_id}: {error}"
            ));
            false
        }
    }
}

/// An action changed what the widget shows unless it only opened a menu and
/// left every override as it was.
pub(super) fn action_needs_render(effects: &[MouseActionEffect], overrides_changed: bool) -> bool {
    overrides_changed
        || effects
            .iter()
            .any(|effect| !matches!(effect, MouseActionEffect::ShowContextMenu(_)))
        || effects.is_empty()
}

pub(super) fn schedule_or_dispatch_click(hwnd: HWND, surface_index: usize, object_id: String) {
    let has_click = mouse_handler_exists(surface_index, &object_id, MouseEventKind::Click);
    if !has_click {
        return;
    }
    if mouse_handler_exists(surface_index, &object_id, MouseEventKind::DoubleClick) {
        let owner = {
            let mut state = lock_state();
            let Some(state) = state.as_mut() else {
                return;
            };
            state.pending_mouse_click = Some(PendingMouseClick {
                surface_index,
                object_id,
            });
            state.hwnd.to_hwnd()
        };
        unsafe {
            let _ = kill_controller_timer(Some(owner), TIMER_MOUSE_CLICK);
            set_controller_timer(
                Some(owner),
                TIMER_MOUSE_CLICK,
                GetDoubleClickTime().max(1),
                None,
            );
        }
    } else {
        let _ = dispatch_mouse_event(surface_index, &object_id, MouseEventKind::Click);
    }
    let _ = hwnd;
}

pub(super) fn dispatch_double_click(hwnd: HWND, surface_index: usize, object_id: String) {
    if !mouse_handler_exists(surface_index, &object_id, MouseEventKind::DoubleClick) {
        return;
    }
    let owner = {
        let mut state = lock_state();
        let Some(state) = state.as_mut() else {
            return;
        };
        state.pending_mouse_click = None;
        state.suppress_next_left_up = true;
        state.hwnd.to_hwnd()
    };
    unsafe {
        let _ = kill_controller_timer(Some(owner), TIMER_MOUSE_CLICK);
    }
    let _ = dispatch_mouse_event(surface_index, &object_id, MouseEventKind::DoubleClick);
    let _ = hwnd;
}

/// A theme-authored `click` action keeps its old routing (including the
/// double-click delay). Only a click over an object with no click action
/// changes the mode of the widget under the cursor. Keeping this decision pure makes the priority rule testable.
pub(super) fn click_changes_view_mode(has_click_handler: bool) -> bool {
    !has_click_handler
}

/// Route a left-button release that did not start a drag. Both the primary
/// surface and every mirror/desktop surface call this, so a click on any
/// widget advances that widget. A point outside all widgets changes nothing.
pub(super) fn handle_view_mode_click(hwnd: HWND, lparam: LPARAM) {
    let Some((surface, object)) = mouse_target_at(hwnd, lparam) else {
        return;
    };
    let has_click = mouse_handler_exists(surface, &object, MouseEventKind::Click);
    if click_changes_view_mode(has_click) {
        if let Some((index, all)) = mode_target_at(lparam, surface) {
            super::apply_widget_mode(hwnd, index, all || shift_held(), None);
        }
    } else {
        schedule_or_dispatch_click(hwnd, surface, object);
    }
}

/// Win32 `WHEEL_DELTA`: one physical wheel notch is 120 units. High-resolution
/// wheels split that notch into several smaller `WM_MOUSEWHEEL` messages.
const WHEEL_DELTA: i32 = 120;

/// Accumulates partial wheel deltas so one physical notch advances the view
/// mode exactly once. The accumulator is shared across surfaces.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct WheelAccumulator {
    remainder: i32,
}

impl WheelAccumulator {
    /// A partial remainder expires after this gap, so a stale scroll cannot
    /// combine with a later gesture.
    const RESET_AFTER: Duration = Duration::from_millis(250);

    /// Fold one wheel message. Returns true when the message contains at least
    /// one complete notch. A direction reversal or an expired remainder drops
    /// the partial delta. All complete notches in the message are consumed, so
    /// one message advances at most one mode and cannot run away.
    pub(super) fn push(&mut self, delta: i32, elapsed: Duration) -> bool {
        if delta == 0 {
            return false;
        }
        if elapsed >= Self::RESET_AFTER || delta.signum() != self.remainder.signum() {
            self.remainder = 0;
        }
        self.remainder += delta;
        let notches = self.remainder.abs() / WHEEL_DELTA;
        if notches == 0 {
            return false;
        }
        self.remainder -= self.remainder.signum() * notches * WHEEL_DELTA;
        true
    }
}

static WHEEL_ACCUM: OnceLock<Mutex<(WheelAccumulator, Option<Instant>)>> = OnceLock::new();

fn wheel_notch_ready(delta: i32) -> bool {
    let cache = WHEEL_ACCUM.get_or_init(|| Mutex::new((WheelAccumulator::default(), None)));
    let mut entry = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let now = Instant::now();
    let elapsed = entry
        .1
        .map(|last| now.duration_since(last))
        .unwrap_or(WheelAccumulator::RESET_AFTER);
    entry.1 = Some(now);
    entry.0.push(delta, elapsed)
}

/// Wheel down (negative delta) drives towards mode 3, wheel up towards mode 1.
/// The wheel sets the direction explicitly and is ignored at a boundary. It is
/// gated on the cursor being over the widget via the common hit-test, so a
/// wheel delivered to this window while the pointer is elsewhere is ignored.
pub(super) fn handle_mouse_wheel(hwnd: HWND, wparam: WPARAM) {
    let delta = ((wparam.0 >> 16) & 0xffff) as u16 as i16;
    if delta == 0 {
        return;
    }
    let Some(lparam) = cursor_client_lparam(hwnd) else {
        return;
    };
    let Some((surface, _)) = mouse_target_at(hwnd, lparam) else {
        return;
    };
    let Some((index, all)) = mode_target_at(lparam, surface) else {
        return;
    };
    let shift = all || shift_held();
    if !wheel_notch_ready(i32::from(delta)) {
        return;
    }
    let toward = if delta > 0 { -1 } else { 1 };
    super::apply_widget_mode(hwnd, index, shift, Some(toward));
}

/// Rebuild a client-space `LPARAM` from the live cursor. `WM_MOUSEWHEEL`
/// carries its position in screen coordinates, so its own `lParam` cannot be
/// handed to `mouse_target_at`; the cursor position is authoritative anyway.
fn cursor_client_lparam(hwnd: HWND) -> Option<LPARAM> {
    let mut point = POINT::default();
    if unsafe { GetCursorPos(&mut point) }.is_err() {
        return None;
    }
    let mut client = [point];
    unsafe { MapWindowPoints(None, Some(hwnd), &mut client) };
    let packed = ((client[0].y as u32 & 0xffff) << 16) | (client[0].x as u32 & 0xffff);
    Some(LPARAM(packed as isize))
}

fn cursor_over_widget(hwnd: HWND) -> bool {
    cursor_client_lparam(hwnd)
        .and_then(|point| mouse_target_at(hwnd, point))
        .is_some()
}

/// A view-mode transition keeps the widget's right screen edge pinned and
/// moves its left edge, so a stationary cursor can end up outside the window.
/// Win32 does not post `WM_MOUSELEAVE` when the window itself moves away, which
/// would leave the tooltip anchor and the four hover crossfades frozen until
/// the next physical mouse move. Re-evaluate hover against the settled
/// geometry: tear the stale state down when the cursor left, or re-arm
/// tracking and recompute the provider hover when it stayed over the widget.
pub(super) fn revalidate_mouse_hover(hwnd: HWND) {
    reset_mouse_move_throttle();
    let Some(lparam) = cursor_client_lparam(hwnd) else {
        clear_mouse_hover(hwnd);
        return;
    };
    if mouse_target_at(hwnd, lparam).is_some() {
        update_mouse_hover(hwnd, lparam);
    } else {
        clear_mouse_hover(hwnd);
    }
}

thread_local! {
    static REVALIDATING_HOVER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn hover_needs_revalidation(provider_hovered: bool, layer_hovered: bool) -> bool {
    provider_hovered || layer_hovered
}

#[derive(Debug, PartialEq, Eq)]
enum HoverRevalidation<H> {
    Nothing,
    /// Recheck the hovered (or newly entered) surface in place.
    Revalidate(H),
    /// The cursor is no longer over the hovered surface. `old` is `None` when
    /// that surface is gone (it can never get WM_MOUSELEAVE); `enter` is
    /// another surface that now sits under the stationary cursor.
    Move {
        old: Option<H>,
        enter: Option<H>,
    },
}

/// Pure decision for `revalidate_active_hover`. `surfaces[i]` is the window of
/// surface `i`, or `None` when that surface is gone.
fn plan_hover_revalidation<H: Copy>(
    active: bool,
    hovered_index: Option<usize>,
    surfaces: &[Option<H>],
    cursor_over: impl Fn(H) -> bool,
) -> HoverRevalidation<H> {
    let mut under_cursor = surfaces
        .iter()
        .flatten()
        .copied()
        .filter(|hwnd| cursor_over(*hwnd));
    if active {
        let old = surfaces.get(hovered_index.unwrap_or(0)).copied().flatten();
        if let Some(old) = old {
            if cursor_over(old) {
                return HoverRevalidation::Revalidate(old);
            }
        }
        return HoverRevalidation::Move {
            old,
            enter: under_cursor.next(),
        };
    }
    // Nothing hovered yet: a surface that moved or was recreated under a
    // stationary cursor must still start its hover.
    under_cursor
        .next()
        .map_or(HoverRevalidation::Nothing, HoverRevalidation::Revalidate)
}

static HOVER_REVALIDATION_PENDING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Ask for a hover recheck after a surface moved, resized or was recreated.
/// Win32 posts no WM_MOUSELEAVE/WM_MOUSEMOVE when the window moves under a
/// stationary cursor. The recheck is posted, never run inline, so it cannot
/// re-enter a render or a theme mouse action; repeated requests coalesce.
pub(super) fn request_hover_revalidation(hwnd: HWND) {
    use std::sync::atomic::Ordering;
    if HOVER_REVALIDATION_PENDING.swap(true, Ordering::AcqRel) {
        return;
    }
    let posted = unsafe {
        PostMessageW(
            Some(controller_for(hwnd)),
            native_interop::WM_APP_REVALIDATE_HOVER,
            WPARAM(0),
            LPARAM(0),
        )
    };
    if posted.is_err() {
        HOVER_REVALIDATION_PENDING.store(false, Ordering::Release);
    }
}

pub(super) fn run_requested_hover_revalidation() {
    HOVER_REVALIDATION_PENDING.store(false, std::sync::atomic::Ordering::Release);
    revalidate_active_hover();
}

/// Recheck hover against the settled geometry of every visible surface.
fn revalidate_active_hover() {
    // A theme action run below may render and move a surface; that only
    // posts another request, handled after this pass returns.
    if REVALIDATING_HOVER.with(|guard| guard.replace(true)) {
        return;
    }
    revalidate_active_hover_once();
    REVALIDATING_HOVER.with(|guard| guard.set(false));
}

fn revalidate_active_hover_once() {
    let (active, hovered_index, surfaces, primary) = {
        let state = lock_state();
        let Some(state) = state.as_ref() else {
            return;
        };
        let provider_hovered = state.antigravity_hovered
            || state.claude_hovered
            || state.codex_hovered
            || state.opencode_hovered
            || state.qoder_hovered
            || state.agy_external_hovered
            || state.split_hovers.iter().any(|hover| hover.hovered);
        let active =
            hover_needs_revalidation(provider_hovered, state.hovered_mouse_layer.is_some());
        let count = (state.mirror_hwnds.len() + 1).max(state.desktop_hwnds.len());
        let surfaces: Vec<Option<HWND>> = (0..count)
            .map(|index| {
                state
                    .desktop_hwnds
                    .get(index)
                    .and_then(|hwnd| *hwnd)
                    .or_else(|| {
                        if index == 0 {
                            Some(state.hwnd)
                        } else {
                            state.mirror_hwnds.get(index - 1).copied()
                        }
                    })
                    .map(SendHwnd::to_hwnd)
                    // A hidden surface must neither keep nor gain hover.
                    .filter(|hwnd| unsafe {
                        IsWindow(Some(*hwnd)).as_bool() && IsWindowVisible(*hwnd).as_bool()
                    })
            })
            .collect();
        let hovered_index = state.hovered_mouse_layer.as_ref().map(|(index, _)| *index);
        // A tray-icon surface has no visible window; its own timer
        // (`clear_tray_mouse_hover_if_left`) tracks the cursor.
        let tray_hovered = hovered_index.is_some_and(|index| {
            state.active_theme.as_ref().is_some_and(|theme| {
                theme.surfaces.get(index).is_some_and(|surface| {
                    surface
                        .placement
                        .nest
                        .resolve(surface.placement.reference.region)
                        == SurfaceNest::TrayIcon
                })
            })
        });
        if tray_hovered {
            return;
        }
        (active, hovered_index, surfaces, state.hwnd.to_hwnd())
    };
    {
        match plan_hover_revalidation(active, hovered_index, &surfaces, cursor_over_widget) {
            HoverRevalidation::Nothing => {}
            HoverRevalidation::Revalidate(hwnd) => revalidate_mouse_hover(hwnd),
            HoverRevalidation::Move { old, enter } => {
                if let Some(old) = old {
                    clear_mouse_hover(old);
                } else {
                    clear_mouse_hover(primary);
                    // `clear_mouse_hover` keeps a layer that belongs to another
                    // surface; that surface is gone or hidden, so drop it here
                    // and still run its leave action.
                    let stale = lock_state()
                        .as_mut()
                        .and_then(|state| state.hovered_mouse_layer.take());
                    if let Some((surface, object)) = stale {
                        let _ = dispatch_mouse_event(surface, &object, MouseEventKind::MouseLeave);
                    }
                }
                if let Some(enter) = enter {
                    revalidate_mouse_hover(enter);
                }
            }
        }
    }
}

#[cfg(test)]
mod hover_revalidation_tests {
    use super::{hover_needs_revalidation, plan_hover_revalidation, HoverRevalidation};

    #[test]
    fn active_hover_requires_revalidation_after_geometry_changes() {
        assert!(!hover_needs_revalidation(false, false));
        assert!(hover_needs_revalidation(true, false));
        assert!(hover_needs_revalidation(false, true));
        // Every real move/resize/recreate goes through `window_pos_flags`;
        // the drag path posts too. Rendering itself must not revalidate.
        let positioning = include_str!("positioning.rs");
        assert!(positioning.contains("super::mouse::request_hover_revalidation(hwnd)"));
        let messages = include_str!("message_loop.rs");
        assert!(messages.contains("super::mouse::request_hover_revalidation(hwnd)"));
        assert!(messages.contains("super::mouse::run_requested_hover_revalidation()"));
        let render = include_str!("../window.rs");
        assert!(!render.contains("mouse::revalidate_active_hover"));
        assert!(render.contains("mouse::request_hover_revalidation(replacement)"));
    }

    #[test]
    fn moved_hovered_surface_is_rechecked() {
        let surfaces = [Some(10), Some(11)];
        assert_eq!(
            plan_hover_revalidation(true, Some(1), &surfaces, |hwnd| hwnd == 11),
            HoverRevalidation::Revalidate(11)
        );
        // Provider hover without a layer rechecks the primary surface.
        assert_eq!(
            plan_hover_revalidation(true, None, &surfaces, |hwnd| hwnd == 10),
            HoverRevalidation::Revalidate(10)
        );
        // The hovered surface moved away from the cursor.
        assert_eq!(
            plan_hover_revalidation(true, Some(1), &surfaces, |_| false),
            HoverRevalidation::Move {
                old: Some(11),
                enter: None
            }
        );
    }

    #[test]
    fn removed_hovered_surface_clears_stale_hover() {
        assert_eq!(
            plan_hover_revalidation(true, Some(1), &[Some(10), None], |_| false),
            HoverRevalidation::Move {
                old: None,
                enter: None
            }
        );
        assert_eq!(
            plan_hover_revalidation(true, Some(2), &[Some(10)], |_| true),
            HoverRevalidation::Move {
                old: None,
                enter: Some(10)
            }
        );
    }

    #[test]
    fn hover_moves_to_the_surface_now_under_the_cursor() {
        assert_eq!(
            plan_hover_revalidation(true, Some(0), &[Some(10), Some(11)], |hwnd| hwnd == 11),
            HoverRevalidation::Move {
                old: Some(10),
                enter: Some(11)
            }
        );
    }

    #[test]
    fn surface_moved_under_a_stationary_cursor_gains_hover() {
        let surfaces = [Some(10), None, Some(12)];
        assert_eq!(
            plan_hover_revalidation(false, None, &surfaces, |hwnd| hwnd == 12),
            HoverRevalidation::Revalidate(12)
        );
        assert_eq!(
            plan_hover_revalidation(false, None, &surfaces, |_| false),
            HoverRevalidation::Nothing
        );
    }
}

/// Theme object whose hover reveals the Antigravity Claude/GPT pool.
pub(super) const ANTIGRAVITY_HOVER_OBJECT: &str = "antigravity-provider";
pub(super) const CLAUDE_HOVER_OBJECT: &str = "claude-provider";
pub(super) const CODEX_HOVER_OBJECT: &str = "codex-provider";
pub(super) const OPENCODE_HOVER_OBJECT: &str = "opencode-provider";
pub(super) const QODER_HOVER_OBJECT: &str = "qoder-provider";
pub(super) const AGY_EXTERNAL_HOVER_OBJECT: &str = theme_engine::ANTIGRAVITY_EXTERNAL_WIDGET;

fn tile_hover_at(
    state: &super::AppState,
    hwnd: HWND,
    lparam: LPARAM,
    rect: Option<(f64, f64, f64, f64)>,
) -> bool {
    if surface_index_for_window(state, hwnd) != Some(0) {
        return false;
    }
    let Some(rect) = rect else {
        return false;
    };
    let Some(theme) = state.active_theme.as_ref() else {
        return false;
    };
    let scale = theme_surface_scale(theme, 0).max(0.01);
    let (x, y) = mouse_client_point(lparam);
    let lx = x / scale;
    let ly = y / scale;
    lx >= rect.0 && lx < rect.0 + rect.2 && ly >= rect.1 && ly < rect.1 + rect.3
}

/// A duplicate `WM_MOUSEMOVE` at the same client point cannot change any
/// hover state, but it used to re-run four hit-tests, a `TME_QUERY` and up to
/// four `mouse_target_at` passes. Coalescing them keeps the message queue
/// short so animation ticks arrive on time. `WM_MOUSELEAVE` clears the cache.
static LAST_MOUSE_MOVE: OnceLock<Mutex<Option<(Instant, isize)>>> = OnceLock::new();

pub(super) fn reset_mouse_move_throttle() {
    let cache = LAST_MOUSE_MOVE.get_or_init(|| Mutex::new(None));
    *cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
}

/// `now` is injected by the caller so the coalescing window is unit-testable
/// without depending on wall-clock timing.
fn duplicate_mouse_move(lparam: LPARAM, now: Instant) -> bool {
    let cache = LAST_MOUSE_MOVE.get_or_init(|| Mutex::new(None));
    let mut last = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let duplicate = last.as_ref().is_some_and(|(at, point)| {
        *point == lparam.0 && now.duration_since(*at) < Duration::from_millis(8)
    });
    *last = Some((now, lparam.0));
    duplicate
}

pub(super) fn update_mouse_hover(hwnd: HWND, lparam: LPARAM) {
    if duplicate_mouse_move(lparam, Instant::now()) {
        return;
    }
    track_mouse_hover_with(hwnd, widget_tooltip::visible_for(hwnd), |tracking| unsafe {
        TrackMouseEvent(tracking)
    });
    let (
        antigravity_hovered,
        claude_hovered,
        codex_hovered,
        opencode_hovered,
        qoder_hovered,
        agy_external_hovered,
    ) = {
        let state = lock_state();
        if let Some(state) = state.as_ref() {
            (
                tile_hover_at(state, hwnd, lparam, state.antigravity_tile_rect),
                tile_hover_at(state, hwnd, lparam, state.claude_tile_rect),
                tile_hover_at(state, hwnd, lparam, state.codex_tile_rect),
                tile_hover_at(state, hwnd, lparam, state.opencode_tile_rect),
                tile_hover_at(state, hwnd, lparam, state.qoder_tile_rect),
                tile_hover_at(state, hwnd, lparam, state.agy_external_tile_rect),
            )
        } else {
            (false, false, false, false, false, false)
        }
    };
    set_antigravity_hover(hwnd, antigravity_hovered);
    super::set_claude_hover(hwnd, claude_hovered);
    super::set_codex_hover(hwnd, codex_hovered);
    // `mouse_target_at` only returns objects with non-empty `mouse_events`,
    // and the provider wrappers have none, so the OpenCode tile is detected
    // with the same cached `object_bounds` rect as the other three providers.
    super::set_opencode_hover(hwnd, opencode_hovered);
    super::set_qoder_hover(hwnd, qoder_hovered);
    super::set_agy_external_hover(hwnd, agy_external_hovered);
    let split_hovered = {
        let state = lock_state();
        std::array::from_fn::<_, 4, _>(|i| {
            state
                .as_ref()
                .is_some_and(|state| tile_hover_at(state, hwnd, lparam, state.split_hovers[i].rect))
        })
    };
    for (index, hovered) in split_hovered.into_iter().enumerate() {
        super::set_split_hover(hwnd, index, hovered);
    }
    // No tick here: the animation timer drives every frame. Ticking on mouse
    // move could produce a second full render, and its throttle could swallow
    // real timer ticks.
    for _ in 0..4 {
        let target = mouse_target_at(hwnd, lparam);
        let previous = {
            let mut state = lock_state();
            let Some(state) = state.as_mut() else {
                return;
            };
            if state.hovered_mouse_layer == target {
                return;
            }
            std::mem::replace(&mut state.hovered_mouse_layer, target.clone())
        };
        if let Some((surface, object)) = previous {
            let _ = dispatch_mouse_event(surface, &object, MouseEventKind::MouseLeave);
        }
        if let Some((surface, object)) = &target {
            let _ = dispatch_mouse_event(*surface, object, MouseEventKind::MouseEnter);
        }
    }
    diagnose::log("mouse enter/leave actions did not stabilize after four transitions");
    if let Some(state) = lock_state().as_mut() {
        state.hovered_mouse_layer = None;
    }
}

fn track_mouse_hover_with(
    hwnd: HWND,
    tooltip_visible: bool,
    mut track: impl FnMut(&mut TRACKMOUSEEVENT) -> windows::core::Result<()>,
) {
    let mut tracking = TRACKMOUSEEVENT {
        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_QUERY,
        ..Default::default()
    };
    let active = if track(&mut tracking).is_ok()
        && tracking.hwndTrack == hwnd
        && !tracking.dwFlags.contains(TME_NONCLIENT)
    {
        tracking.dwFlags
    } else {
        Default::default()
    };
    let desired = if tooltip_visible {
        TME_LEAVE
    } else {
        TME_LEAVE | TME_HOVER
    };
    let missing = desired & !active;
    if missing.0 != 0 {
        // Re-requesting TME_HOVER resets its timer, even for synthetic mouse
        // moves caused by repainting/repositioning a stationary widget. Let
        // Windows keep the hover rectangle and deadline until hover or leave.
        tracking.dwFlags = missing;
        tracking.hwndTrack = hwnd;
        tracking.dwHoverTime = HOVER_DEFAULT;
        let _ = track(&mut tracking);
    }
}

pub(super) fn clear_mouse_hover(hwnd: HWND) {
    reset_mouse_move_throttle();
    widget_tooltip::hide_for(hwnd);
    set_antigravity_hover(hwnd, false);
    super::set_claude_hover(hwnd, false);
    super::set_codex_hover(hwnd, false);
    super::set_opencode_hover(hwnd, false);
    super::set_qoder_hover(hwnd, false);
    super::set_agy_external_hover(hwnd, false);
    for index in 0..4 {
        super::set_split_hover(hwnd, index, false);
    }
    let previous = {
        let mut state = lock_state();
        let Some(state) = state.as_mut() else {
            return;
        };
        let surface = surface_index_for_window(state, hwnd);
        if state
            .hovered_mouse_layer
            .as_ref()
            .is_none_or(|(hovered, _)| Some(*hovered) != surface)
        {
            return;
        }
        state.hovered_mouse_layer.take()
    };
    if let Some((surface, object)) = previous {
        let _ = dispatch_mouse_event(surface, &object, MouseEventKind::MouseLeave);
    }
}

pub(super) fn update_tray_mouse_hover(hwnd: HWND, surface_index: usize, object_id: String) {
    let target = Some((surface_index, object_id));
    let previous = {
        let mut state = lock_state();
        let Some(state) = state.as_mut() else {
            return;
        };
        if state.hovered_mouse_layer == target {
            unsafe {
                set_controller_timer(Some(hwnd), TIMER_TRAY_HOVER, 100, None);
            }
            return;
        }
        std::mem::replace(&mut state.hovered_mouse_layer, target.clone())
    };
    if let Some((surface, object)) = previous {
        let _ = dispatch_mouse_event(surface, &object, MouseEventKind::MouseLeave);
    }
    if let Some((surface, object)) = target {
        let _ = dispatch_mouse_event(surface, &object, MouseEventKind::MouseEnter);
    }
    unsafe {
        set_controller_timer(Some(hwnd), TIMER_TRAY_HOVER, 100, None);
    }
}

pub(super) fn clear_tray_mouse_hover_if_left(hwnd: HWND) {
    let hovered = {
        let state = lock_state();
        let Some(state) = state.as_ref() else {
            return;
        };
        state.hovered_mouse_layer.clone().map(|target| {
            let is_tray = state.active_theme.as_ref().is_some_and(|theme| {
                theme.surfaces.get(target.0).is_some_and(|surface| {
                    surface
                        .placement
                        .nest
                        .resolve(surface.placement.reference.region)
                        == SurfaceNest::TrayIcon
                })
            });
            (target, is_tray)
        })
    };
    let Some((target, true)) = hovered else {
        unsafe {
            let _ = kill_controller_timer(Some(hwnd), TIMER_TRAY_HOVER);
        }
        return;
    };
    if tray_icon::cursor_over_themed_icon(controller_for(hwnd), target.0) {
        return;
    }
    let previous = lock_state().as_mut().and_then(|state| {
        (state.hovered_mouse_layer.as_ref() == Some(&target))
            .then(|| state.hovered_mouse_layer.take())
            .flatten()
    });
    unsafe {
        let _ = kill_controller_timer(Some(hwnd), TIMER_TRAY_HOVER);
    }
    if let Some((surface, object)) = previous {
        let _ = dispatch_mouse_event(surface, &object, MouseEventKind::MouseLeave);
    }
}

/// Answer WM_SETCURSOR without DefWindowProc. For a child surface it first
/// sends WM_SETCURSOR to the parent, which for a taskbar-hosted surface is a
/// synchronous call into Explorer on every mouse move.
pub(super) unsafe fn set_surface_cursor(hwnd: HWND) {
    let cursor = if surface_cursor_is_clickable(hwnd) {
        IDC_HAND
    } else {
        IDC_ARROW
    };
    SetCursor(Some(LoadCursorW(None, cursor).unwrap_or_default()));
}

unsafe fn surface_cursor_is_clickable(hwnd: HWND) -> bool {
    let Some(lparam) = cursor_client_lparam(hwnd) else {
        return false;
    };
    let Some((surface, object)) = mouse_target_at(hwnd, lparam) else {
        return false;
    };
    [
        MouseEventKind::Click,
        MouseEventKind::DoubleClick,
        MouseEventKind::RightClick,
    ]
    .into_iter()
    .any(|event| mouse_handler_exists(surface, &object, event))
}

#[cfg(test)]
mod tests {
    #[test]
    fn opening_the_menu_alone_skips_the_re_render() {
        use super::action_needs_render;
        use crate::theme_engine::MouseActionEffect as Effect;
        let menu = [Effect::ShowContextMenu(Some("classic-v1".into()))];
        assert!(!action_needs_render(&menu, false));
        assert!(action_needs_render(&menu, true));
        assert!(action_needs_render(&[], false));
        assert!(action_needs_render(
            &[Effect::ShowContextMenu(None), Effect::ShowDashboard],
            false
        ));
    }

    #[test]
    fn built_in_widget_hit_test_finds_provider_containers() {
        use super::*;
        let mut theme: ThemeDocument =
            serde_json::from_str(include_str!("../themes/compact-fluent-quad.json")).unwrap();
        theme.prepare_runtime();
        let runtime = ThemeRuntime::from_providers(ProviderSet::from_enabled([
            ProviderId::Claude,
            ProviderId::Codex,
        ]));
        let metrics = theme_engine::display_metrics(&theme, None, runtime);
        for id in ["claude-provider", "codex-provider"] {
            let (x, y, width, height) =
                theme_engine::object_bounds_with_metrics(&theme, 0, None, runtime, id, &metrics)
                    .unwrap();
            assert_eq!(
                theme_engine::hit_test_widget_with_metrics(
                    &theme,
                    0,
                    x + width / 2.0,
                    y + height / 2.0,
                    None,
                    runtime,
                    &metrics
                )
                .as_deref(),
                Some(id)
            );
        }
        assert_eq!(
            theme_engine::hit_test_widget_with_metrics(
                &theme, 0, -100.0, -100.0, None, runtime, &metrics
            ),
            None
        );
    }

    #[test]
    fn widget_key_uses_nearest_provider_ancestor() {
        use super::*;
        let mut surface = theme_engine::SceneObject::object("main", "main");
        let mut provider = theme_engine::SceneObject::object("fable-provider", "Fable");
        provider.parent = Some("main".into());
        let mut child = theme_engine::SceneObject::object("fable-row", "row");
        child.parent = Some("fable-provider".into());
        let mut nested = theme_engine::SceneObject::object("label", "label");
        nested.parent = Some("fable-row".into());
        surface.children = vec![provider, child, nested];
        assert_eq!(widget_key_for_object(&surface, "label"), Some("fable"));
        assert_eq!(
            widget_key_for_object(&surface, "fable-provider"),
            Some("fable")
        );
        assert_eq!(widget_key_for_object(&surface, "main"), None);
        assert_eq!(widget_key_for_object(&surface, "unknown"), None);
        // The agy external-models widget changes only its own mode.
        let mut external = theme_engine::SceneObject::object(
            theme_engine::ANTIGRAVITY_EXTERNAL_WIDGET,
            "agy external models",
        );
        external.parent = Some("main".into());
        let mut gauge = theme_engine::SceneObject::object("antigravity-ext-a-5h", "gauge");
        gauge.parent = Some(theme_engine::ANTIGRAVITY_EXTERNAL_WIDGET.into());
        surface.children.extend([external, gauge]);
        assert_eq!(
            widget_key_for_object(&surface, "antigravity-ext-a-5h"),
            Some(theme_engine::AGY_EXTERNAL_KEY)
        );
        assert_ne!(
            app_settings::widget_mode_keys().position(|key| key == "antigravity"),
            app_settings::widget_mode_keys().position(|key| key == theme_engine::AGY_EXTERNAL_KEY)
        );
        assert!(surface_has_widgets(&surface));
        // A custom theme without provider containers keeps the shared mode.
        let mut legacy = theme_engine::SceneObject::object("main", "main");
        legacy.children = vec![theme_engine::SceneObject::object("bar", "bar")];
        assert!(!surface_has_widgets(&legacy));
    }

    use super::*;

    #[test]
    fn theme_click_actions_keep_priority_over_view_mode_switching() {
        assert!(
            click_changes_view_mode(false),
            "a plain click over a widget with no click action must bounce the mode"
        );
        assert!(
            !click_changes_view_mode(true),
            "an authored JSON click keeps its old routing; only the wheel changes the mode"
        );
    }

    #[test]
    fn a_standard_wheel_notch_advances_exactly_once() {
        let mut up = WheelAccumulator::default();
        assert!(up.push(120, Duration::from_millis(1)));
        assert_eq!(up.remainder, 0);
        assert!(!up.push(0, Duration::from_millis(1)));

        let mut down = WheelAccumulator::default();
        assert!(down.push(-120, Duration::from_millis(1)));
        assert_eq!(down.remainder, 0);
    }

    #[test]
    fn high_resolution_wheel_deltas_accumulate_to_one_notch() {
        let mut acc = WheelAccumulator::default();
        assert!(!acc.push(40, Duration::from_millis(1)));
        assert!(!acc.push(40, Duration::from_millis(1)));
        assert_eq!(acc.remainder, 80);
        assert!(
            acc.push(40, Duration::from_millis(1)),
            "three partial deltas complete exactly one notch"
        );
        assert_eq!(acc.remainder, 0);
        // A second physical notch again needs three partial messages.
        assert!(!acc.push(40, Duration::from_millis(1)));
        assert!(!acc.push(40, Duration::from_millis(1)));
        assert!(acc.push(40, Duration::from_millis(1)));

        // A partial remainder carries within one continuous gesture.
        let mut carry = WheelAccumulator::default();
        assert!(!carry.push(80, Duration::from_millis(1)));
        assert!(carry.push(120, Duration::from_millis(1)));
        assert_eq!(carry.remainder, 80);
        assert!(carry.push(40, Duration::from_millis(1)));
        assert_eq!(carry.remainder, 0);
    }

    #[test]
    fn wheel_accumulator_resets_on_direction_change_and_idle_gap() {
        let mut acc = WheelAccumulator::default();
        assert!(!acc.push(80, Duration::from_millis(1)));
        assert!(
            !acc.push(-40, Duration::from_millis(1)),
            "reversing direction drops the partial remainder"
        );
        assert_eq!(acc.remainder, -40);
        assert!(!acc.push(-40, Duration::from_millis(1)));
        assert!(acc.push(-40, Duration::from_millis(1)));
        assert_eq!(acc.remainder, 0);

        let mut stale = WheelAccumulator::default();
        assert!(!stale.push(80, Duration::from_millis(1)));
        assert!(
            !stale.push(40, Duration::from_millis(300)),
            "a stale partial must not complete a notch"
        );
        assert_eq!(stale.remainder, 40);
    }

    #[test]
    fn one_wheel_message_never_advances_more_than_one_mode() {
        let mut acc = WheelAccumulator::default();
        assert!(
            acc.push(360, Duration::from_millis(1)),
            "three coalesced notches still emit one step"
        );
        assert_eq!(acc.remainder, 0, "all complete notches are consumed");
        assert!(
            !acc.push(40, Duration::from_millis(1)),
            "no leftover notch carries into the next message"
        );
    }

    #[test]
    fn duplicate_mouse_moves_are_coalesced_but_real_moves_are_not() {
        reset_mouse_move_throttle();
        let start = Instant::now();
        let point = LPARAM(0x0001_0002);
        assert!(
            !duplicate_mouse_move(point, start),
            "the first move at a point must be processed"
        );
        assert!(
            duplicate_mouse_move(point, start + Duration::from_millis(1)),
            "a duplicate at the same point within 8 ms must be skipped"
        );
        assert!(
            !duplicate_mouse_move(point, start + Duration::from_millis(9)),
            "the same point after the 8 ms window must be processed"
        );
        let elsewhere = LPARAM(0x0003_0004);
        assert!(
            !duplicate_mouse_move(elsewhere, start + Duration::from_millis(10)),
            "a move to different coordinates must be processed"
        );
        // WM_MOUSELEAVE clears the throttle so re-entry at the same point works.
        reset_mouse_move_throttle();
        assert!(
            !duplicate_mouse_move(point, start + Duration::from_millis(11)),
            "re-entry after a leave must be processed"
        );
    }

    #[test]
    fn repeated_mouse_moves_preserve_the_pending_hover_deadline() {
        let hwnd = HWND(std::ptr::dangling_mut());
        let mut active = TRACKMOUSEEVENT::default();
        let mut deadline = None;
        // Model Win32's documented timer reset on each TME_HOVER request.
        // Repaints can generate mouse moves even with a stationary cursor.
        for elapsed in (0..=500).step_by(50) {
            track_mouse_hover_with(hwnd, false, |tracking| {
                if tracking.dwFlags == TME_QUERY {
                    tracking.hwndTrack = active.hwndTrack;
                    tracking.dwFlags = active.dwFlags;
                } else {
                    assert_eq!(tracking.dwHoverTime, HOVER_DEFAULT);
                    if tracking.dwFlags.contains(TME_HOVER) {
                        deadline = Some(elapsed + 400);
                    }
                    active.hwndTrack = tracking.hwndTrack;
                    active.dwFlags |= tracking.dwFlags;
                }
                Ok(())
            });
        }
        assert_eq!(deadline, Some(400), "mouse moves postponed the hover");
        assert_eq!(active.dwFlags, TME_HOVER | TME_LEAVE);
    }

    #[test]
    fn hover_tracking_rearms_after_hover_leave_and_window_changes() {
        let hwnd = HWND(std::ptr::dangling_mut());
        let mirror = HWND(2 as *mut _);
        for (tracked, active, visible, expected) in [
            // A visible tooltip needs leave notifications, without rearming hover.
            (hwnd, TME_LEAVE, true, None),
            (hwnd, TME_HOVER | TME_LEAVE, true, None),
            // Windows cancels hover after delivering WM_MOUSEHOVER. A dismissed
            // popup can be shown again on a later mouse move.
            (hwnd, TME_LEAVE, false, Some(TME_HOVER)),
            // Adding leave tracking must not reset an existing hover timer.
            (hwnd, TME_HOVER, false, Some(TME_LEAVE)),
            // WM_MOUSELEAVE cancels all tracking; re-entry needs both flags.
            (hwnd, Default::default(), false, Some(TME_HOVER | TME_LEAVE)),
            // Tracking another surface or its nonclient area is not reusable.
            (
                mirror,
                TME_HOVER | TME_LEAVE,
                false,
                Some(TME_HOVER | TME_LEAVE),
            ),
            (
                hwnd,
                TME_NONCLIENT | TME_HOVER | TME_LEAVE,
                false,
                Some(TME_HOVER | TME_LEAVE),
            ),
        ] {
            let mut requests = Vec::new();
            track_mouse_hover_with(hwnd, visible, |tracking| {
                if tracking.dwFlags == TME_QUERY {
                    tracking.hwndTrack = tracked;
                    tracking.dwFlags = active;
                } else {
                    assert_eq!(tracking.hwndTrack, hwnd);
                    requests.push(tracking.dwFlags);
                }
                Ok(())
            });
            assert_eq!(requests, expected.into_iter().collect::<Vec<_>>());
        }
    }

    #[test]
    fn failed_tracking_query_still_arms_hover() {
        let hwnd = HWND(std::ptr::dangling_mut());
        let mut requested = false;
        track_mouse_hover_with(hwnd, false, |tracking| {
            if tracking.dwFlags == TME_QUERY {
                return Err(windows::core::Error::from_hresult(E_FAIL));
            }
            assert_eq!(tracking.hwndTrack, hwnd);
            assert_eq!(tracking.dwFlags, TME_HOVER | TME_LEAVE);
            requested = true;
            Ok(())
        });
        assert!(requested);
    }
}
