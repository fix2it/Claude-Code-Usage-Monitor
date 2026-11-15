use super::*;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicIsize, Ordering};

/// Sentinel for "no submenu anchor captured yet".
const NO_ANCHOR: i64 = i64::MIN;
/// The root HMENU of the menu currently on screen, and the screen position of
/// the last opened submenu. Both are owned by the menu session on this UI
/// thread only; the handles are cleared before `DestroyMenu`.
static ROOT_MENU_HANDLE: AtomicIsize = AtomicIsize::new(0);
static SUBMENU_ANCHOR: AtomicI64 = AtomicI64::new(NO_ANCHOR);
static CAPTURE_SUBMENU_ANCHOR: AtomicBool = AtomicBool::new(false);

fn submenu_anchor() -> Option<(i32, i32)> {
    let packed = SUBMENU_ANCHOR.load(Ordering::Relaxed);
    (packed != NO_ANCHOR).then(|| ((packed >> 32) as i32, packed as i32))
}

fn store_anchor(rect: &RECT) {
    let packed = ((rect.left as i64) << 32) | (rect.top as i64 & 0xFFFF_FFFF);
    SUBMENU_ANCHOR.store(packed, Ordering::Relaxed);
}

/// Called from `WM_MENUSELECT` while a menu is on screen. `lParam` is the HMENU
/// that owns the highlighted item; when it is a submenu we can query that
/// submenu's own item rects. This gives the REAL screen position of the
/// submenu, whether Win32 opened it to the right or flipped it to the left
/// near the screen edge. Using the parent row's right edge (the old approach)
/// was wrong for the flipped case and made the re-opened menu jump right.
pub(super) unsafe fn note_menu_select(wparam: WPARAM, lparam: LPARAM) {
    if !CAPTURE_SUBMENU_ANCHOR.load(Ordering::Relaxed) {
        return;
    }
    let root = ROOT_MENU_HANDLE.load(Ordering::Relaxed);
    if root == 0 {
        return;
    }
    // HIWORD == 0xFFFF means the menu is closing / nothing is selected.
    if (wparam.0 >> 16) & 0xFFFF == 0xFFFF {
        return;
    }
    let menu = HMENU(lparam.0 as *mut _);
    if menu.is_invalid() || menu.0 as isize == root {
        return; // highlighted item belongs to the root menu, not a submenu
    }
    let index = (wparam.0 as u32) & 0xFFFF;
    let mut rect = RECT::default();
    // Item 0 is the submenu's top-left content row; fall back to the
    // highlighted item's own rect if the first row cannot be measured.
    if GetMenuItemRect(None, menu, 0, &mut rect).is_ok()
        || GetMenuItemRect(None, menu, index, &mut rect).is_ok()
    {
        store_anchor(&rect);
    }
}

/// Fallback for `WM_INITMENUPOPUP`: the submenu window may not exist yet, but
/// the root menu is on screen, so its parent row rect is queryable. This only
/// approximates the right-opening case; `note_menu_select` overwrites it with
/// the exact position as soon as an item inside the submenu is highlighted.
pub(super) unsafe fn note_menu_popup(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) {
    if !CAPTURE_SUBMENU_ANCHOR.load(Ordering::Relaxed) {
        return;
    }
    let root = ROOT_MENU_HANDLE.load(Ordering::Relaxed);
    if root == 0 || wparam.0 as isize == root {
        return; // the root popup itself, or no active menu session
    }
    if SUBMENU_ANCHOR.load(Ordering::Relaxed) != NO_ANCHOR {
        return; // an exact position was already captured
    }
    let parent_index = (lparam.0 as u32) & 0xFFFF;
    let root_menu = HMENU(root as *mut _);
    let mut rect = RECT::default();
    // Popup-menu item rects are queryable with the owner HWND on some Windows
    // builds and only with NULL on others; the repo's VM harness uses NULL.
    let found = GetMenuItemRect(Some(hwnd), root_menu, parent_index, &mut rect).is_ok()
        || GetMenuItemRect(None, root_menu, parent_index, &mut rect).is_ok();
    if found {
        store_anchor(&rect);
    }
}

/// A native `TrackPopupMenu` always dismisses after one command. To allow
/// several provider toggles in one session we re-open the menu at the same
/// point until Escape, an outside click, or a non-toggle action. The flag also
/// stops a second right-click from nesting another modal menu loop.
static MENU_SESSION_ACTIVE: AtomicBool = AtomicBool::new(false);
/// A native popup is on screen (inside `TrackPopupMenu`). Shell-facing work
/// that re-parents, re-orders or re-shows the owner surface can dismiss the
/// menu, so it is deferred until the popup closes.
static MENU_TRACKING: AtomicBool = AtomicBool::new(false);
static DEFERRED_FULL_RENDER: AtomicBool = AtomicBool::new(false);
static DEFERRED_RECONCILE: AtomicBool = AtomicBool::new(false);

/// A context-menu session is running (between rounds too).
pub(super) fn menu_session_active() -> bool {
    MENU_SESSION_ACTIVE.load(Ordering::Acquire)
}

/// Shell-facing work that must wait for the popup to close.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DeferredMenuWork {
    FullRender,
    Reconcile,
}

/// Returns true (and remembers the work) while a popup is on screen; the
/// caller then skips it. The work runs once when the popup closes.
pub(super) fn defer_while_menu_open(work: DeferredMenuWork) -> bool {
    if !MENU_TRACKING.load(Ordering::Acquire) {
        return false;
    }
    match work {
        DeferredMenuWork::FullRender => DEFERRED_FULL_RENDER.store(true, Ordering::Release),
        DeferredMenuWork::Reconcile => DEFERRED_RECONCILE.store(true, Ordering::Release),
    }
    true
}

/// `--diagnose` trace of the messages that can dismiss an open popup, so a
/// menu that closes by itself shows what closed it.
pub(super) fn trace_menu_message(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) {
    if !MENU_TRACKING.load(Ordering::Acquire) || !diagnose::is_enabled() {
        return;
    }
    let name = match msg {
        WM_CANCELMODE => "WM_CANCELMODE",
        WM_ACTIVATE => "WM_ACTIVATE",
        WM_ACTIVATEAPP => "WM_ACTIVATEAPP",
        WM_KILLFOCUS => "WM_KILLFOCUS",
        WM_SETFOCUS => "WM_SETFOCUS",
        WM_CAPTURECHANGED => "WM_CAPTURECHANGED",
        WM_WINDOWPOSCHANGING => "WM_WINDOWPOSCHANGING",
        WM_SHOWWINDOW => "WM_SHOWWINDOW",
        WM_UNINITMENUPOPUP => "WM_UNINITMENUPOPUP",
        WM_EXITMENULOOP => "WM_EXITMENULOOP",
        WM_ENTERMENULOOP => "WM_ENTERMENULOOP",
        WM_TIMER => "WM_TIMER",
        _ => return,
    };
    diagnose::log(format!(
        "menu trace hwnd={:#x} {name} wparam={:#x} lparam={:#x} fg={:#x}",
        hwnd.0 as usize,
        wparam.0,
        lparam.0,
        unsafe { GetForegroundWindow() }.0 as usize
    ));
}

/// Mark the popup open or closed. Closing hands back the deferred work.
fn set_menu_tracking(open: bool) -> (bool, bool) {
    MENU_TRACKING.store(open, Ordering::Release);
    if open {
        return (false, false);
    }
    (
        DEFERRED_FULL_RENDER.swap(false, Ordering::AcqRel),
        DEFERRED_RECONCILE.swap(false, Ordering::AcqRel),
    )
}

/// Run what was deferred while the popup was on screen.
fn run_deferred_menu_work((render, reconcile): (bool, bool)) {
    if reconcile {
        lifecycle::reconcile_surfaces();
    }
    if render {
        render_layered();
    }
}

struct MenuSessionGuard;
impl Drop for MenuSessionGuard {
    fn drop(&mut self) {
        CAPTURE_SUBMENU_ANCHOR.store(false, Ordering::Relaxed);
        SUBMENU_ANCHOR.store(NO_ANCHOR, Ordering::Relaxed);
        ROOT_MENU_HANDLE.store(0, Ordering::Relaxed);
        MENU_SESSION_ACTIVE.store(false, Ordering::Release);
    }
}

/// Provider rows are checkboxes: they may be applied repeatedly in one menu
/// session. Every other action keeps the original one-shot behaviour.
fn is_provider_toggle(action: &ContextMenuAction) -> bool {
    matches!(
        action,
        ContextMenuAction::ToggleProvider { .. }
            | ContextMenuAction::ToggleAntigravityGemini
            | ContextMenuAction::ToggleAntigravityExternal
            | ContextMenuAction::ToggleAntigravityA
            | ContextMenuAction::ToggleAntigravityX
            | ContextMenuAction::ToggleAntigravityExternalX
            | ContextMenuAction::ToggleClaudeMain
            | ContextMenuAction::ToggleClaudeX
            | ContextMenuAction::ToggleCodexMain
            | ContextMenuAction::ToggleCodexX
            | ContextMenuAction::ToggleShowFable
            | ContextMenuAction::ToggleShowFableX
            | ContextMenuAction::ToggleClaudePoolsOneWidget
            | ContextMenuAction::ToggleCodexPoolsOneWidget
            | ContextMenuAction::ToggleAntigravityPoolsOneWidget
    )
}

/// Find the menu (root or a nested popup) that owns a command id. Win32 allows
/// `TrackPopupMenu` on a `GetSubMenu` handle, so the Providers submenu can be
/// re-opened directly instead of collapsing back to the root menu.
unsafe fn menu_for_command(menu: HMENU, command: u32) -> Option<HMENU> {
    let count = GetMenuItemCount(Some(menu)).max(0);
    for index in 0..count {
        if GetMenuItemID(menu, index) == command {
            return Some(menu);
        }
        let submenu = GetSubMenu(menu, index);
        if !submenu.is_invalid() {
            if let Some(found) = menu_for_command(submenu, command) {
                return Some(found);
            }
        }
    }
    None
}

pub(super) fn show_context_menu_document(
    hwnd: HWND,
    reference: Option<&str>,
    origin: Option<(usize, String)>,
) {
    if MENU_SESSION_ACTIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let _guard = MenuSessionGuard;

    let document = match context_menu::resolve_context_menu(reference) {
        Ok(document) => document,
        Err(error) => {
            diagnose::log(format!("context menu load failed: {error}"));
            context_menu::classic_context_menu()
        }
    };
    let language = lock_state()
        .as_ref()
        .map(|state| state.language)
        .unwrap_or_else(localization::detect_system_language);
    let mut point = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut point);
    }
    // Capture the real on-screen position of whichever submenu the user opens
    // during this first (root) menu. Only round one has a visible root menu.
    SUBMENU_ANCHOR.store(NO_ANCHOR, Ordering::Relaxed);
    CAPTURE_SUBMENU_ANCHOR.store(true, Ordering::Relaxed);
    // After a provider toggle, re-open the submenu that held the item so the
    // provider list stays put for the next selection.
    let mut reopen_submenu: Option<ContextMenuAction> = None;

    loop {
        // Rebuild every round: conditional rows and checkmarks must reflect
        // the toggles applied so far.
        let data_context = context_menu_data_context(origin.as_ref());
        let mut actions = Vec::new();
        unsafe {
            let Ok(menu) = CreatePopupMenu() else {
                break;
            };
            ROOT_MENU_HANDLE.store(menu.0 as isize, Ordering::Relaxed);
            append_context_menu_items(
                menu,
                &document.items,
                language,
                &data_context,
                origin.as_ref(),
                &mut actions,
            );
            // Locate the same action again by identity (indices can shift if
            // conditional rows changed), then show its containing popup.
            let display_menu = reopen_submenu
                .as_ref()
                .and_then(|action| actions.iter().position(|candidate| candidate == action))
                .map(|index| (1_000 + index) as u32)
                .and_then(|id| menu_for_command(menu, id))
                .unwrap_or(menu);
            // Round one shows the root menu at the cursor; later rounds show
            // the provider submenu at its own captured screen position.
            let (menu_x, menu_y) = if reopen_submenu.is_some() {
                submenu_anchor().unwrap_or((point.x, point.y))
            } else {
                (point.x, point.y)
            };
            let foreground_before = GetForegroundWindow();
            // A taskbar-hosted surface is a child window: it can never be
            // foreground, so SetForegroundWindow(hwnd) fails. Right after start
            // nothing is foreground; the taskbar then activates a moment later
            // and that activation cancels the first menu. Activate the top-level
            // host now, so the menu opens with the foreground already settled.
            let mut foreground_ok = SetForegroundWindow(hwnd).as_bool();
            if !foreground_ok {
                let root = GetAncestor(hwnd, GA_ROOT);
                if !root.is_invalid() && root != hwnd {
                    foreground_ok = SetForegroundWindow(root).as_bool();
                }
            }
            diagnose::log(format!(
                "menu open hwnd={:#x} top={:#x} fg_before={:#x} set_fg={foreground_ok} fg_after={:#x}",
                hwnd.0 as usize,
                GetAncestor(hwnd, GA_ROOT).0 as usize,
                foreground_before.0 as usize,
                GetForegroundWindow().0 as usize
            ));
            let opened_at = std::time::Instant::now();
            set_menu_tracking(true);
            let selected = TrackPopupMenu(
                display_menu,
                TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NOANIMATION,
                menu_x,
                menu_y,
                None,
                hwnd,
                None,
            )
            .0 as usize;
            // KB135788: let the owner's queue run once so the next menu
            // dismisses on an outside click instead of lingering.
            let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            diagnose::log(format!(
                "menu closed selected={selected} after_ms={} fg={:#x}",
                opened_at.elapsed().as_millis(),
                GetForegroundWindow().0 as usize
            ));
            run_deferred_menu_work(set_menu_tracking(false));
            ROOT_MENU_HANDLE.store(0, Ordering::Relaxed);
            let _ = DestroyMenu(menu);

            // 0 = Escape or a click outside the menu: end the session.
            if selected < 1_000 {
                break;
            }
            let Some(action) = actions.get(selected - 1_000).cloned() else {
                break;
            };
            if is_provider_toggle(&action) {
                // Apply synchronously. PostMessageW would dispatch after the
                // next menu was built, so its checkmark would still be stale.
                if let Some(command) = context_menu_static_command(&action) {
                    let _ = SendMessageW(
                        controller_for(hwnd),
                        WM_COMMAND,
                        Some(WPARAM(command as usize)),
                        Some(LPARAM(0)),
                    );
                }
                // The root menu is gone after this round; stop capturing so
                // later WM_INITMENUPOPUP events cannot overwrite the anchor.
                CAPTURE_SUBMENU_ANCHOR.store(false, Ordering::Relaxed);
                reopen_submenu = Some(action);
                continue;
            }
            execute_context_menu_action(hwnd, action, origin);
            break;
        }
    }
}

pub(super) fn context_menu_data_context(origin: Option<&(usize, String)>) -> DataContext {
    let mut state = lock_state();
    let Some(state) = state.as_mut() else {
        return DataContext::from_usage(None, &Canvas::default());
    };
    let mut runtime = theme_runtime_from_state(state);
    let mut canvas = Canvas::default();
    if let Some(theme) = effective_theme_from_state(state) {
        let surface_index = origin.map_or(0, |(surface_index, _)| *surface_index);
        if theme.surfaces.get(surface_index).is_some() {
            runtime = theme_runtime_for_surface(&theme, surface_index, runtime);
            let primary_runtime =
                theme_runtime_for_surface(&theme, 0, theme_runtime_from_state(state));
            let metrics = theme_engine::metrics_for_phase(
                prepared_metrics(state, &theme, primary_runtime),
                runtime,
            );
            let (width, height) = theme_engine::resolve_surface_content_size_with_metrics(
                &theme,
                surface_index,
                state.data.as_ref(),
                runtime,
                &metrics,
            );
            canvas.width = width;
            canvas.height = height;
        }
    }
    DataContext::from_usage_with_runtime(state.data.as_ref(), &canvas, runtime)
}

unsafe fn append_context_menu_items(
    menu: HMENU,
    items: &[ContextMenuItem],
    language: LanguageId,
    context: &DataContext,
    origin: Option<&(usize, String)>,
    actions: &mut Vec<ContextMenuAction>,
) {
    for item in items {
        if !item.should_render(context) {
            continue;
        }
        match &item.kind {
            ContextMenuItemKind::Separator => {
                let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            }
            ContextMenuItemKind::Text => {
                let label = native_interop::wide_str(&context_menu::rendered_label(
                    language,
                    &item.label,
                    context,
                ));
                let _ = AppendMenuW(menu, MF_GRAYED, 0, PCWSTR::from_raw(label.as_ptr()));
            }
            ContextMenuItemKind::Submenu { items } => {
                let Ok(submenu) = CreatePopupMenu() else {
                    continue;
                };
                append_context_menu_items(submenu, items, language, context, origin, actions);
                let label = native_interop::wide_str(&context_menu::rendered_label(
                    language,
                    &item.label,
                    context,
                ));
                let _ = AppendMenuW(
                    menu,
                    MF_POPUP,
                    submenu.0 as usize,
                    PCWSTR::from_raw(label.as_ptr()),
                );
            }
            ContextMenuItemKind::Action { action } => {
                let id = 1_000 + actions.len();
                let label = native_interop::wide_str(&context_menu::rendered_label(
                    language,
                    &item.label,
                    context,
                ));
                let flags = context_menu_action_flags(action, origin);
                let _ = AppendMenuW(menu, flags, id, PCWSTR::from_raw(label.as_ptr()));
                actions.push(action.clone());
            }
        }
    }
}

pub(super) fn context_menu_action_flags(
    action: &ContextMenuAction,
    origin: Option<&(usize, String)>,
) -> MENU_ITEM_FLAGS {
    let state = lock_state();
    let Some(state) = state.as_ref() else {
        return MENU_ITEM_FLAGS(0);
    };
    let checked = context_menu_checked_with_origin(state, action, origin);
    let disabled = matches!(
        action,
        ContextMenuAction::CheckForUpdates
            if matches!(state.update_status, UpdateStatus::Checking | UpdateStatus::Applying)
    ) || matches!(
        action,
        ContextMenuAction::LayerActions { .. } | ContextMenuAction::ToggleLayerRender { .. }
    ) && origin.is_none()
        && state.active_theme.is_none();
    match (checked, disabled) {
        (true, true) => MF_CHECKED | MF_GRAYED,
        (true, false) => MF_CHECKED,
        (false, true) => MF_GRAYED,
        (false, false) => MENU_ITEM_FLAGS(0),
    }
}

/// Check mark of a menu action for the current state (STATE already locked).
pub(super) fn context_menu_checked_with_origin(
    state: &AppState,
    action: &ContextMenuAction,
    origin: Option<&(usize, String)>,
) -> bool {
    match action {
        ContextMenuAction::SetUpdateFrequency { seconds } => {
            state.poll_interval_ms == seconds.saturating_mul(1_000)
        }
        ContextMenuAction::ToggleProvider { provider } => state.providers.contains(*provider),
        // The parent shows agy itself: unchecked whenever agy is off, so one
        // click always turns it back on.
        ContextMenuAction::ToggleAntigravityGemini => {
            state.show_antigravity_gemini && state.providers.contains(ProviderId::Antigravity)
        }
        ContextMenuAction::ToggleAntigravityExternal => state.show_antigravity_external,
        ContextMenuAction::ToggleAntigravityA => {
            state.providers.contains(ProviderId::Antigravity)
                && state.accounts.antigravity_a_enabled()
        }
        ContextMenuAction::ToggleAntigravityX => {
            state.providers.contains(ProviderId::Antigravity)
                && state.accounts.antigravity_x().is_some()
        }
        ContextMenuAction::ToggleAntigravityExternalX => state.show_antigravity_external_x,
        ContextMenuAction::ToggleClaudeMain => {
            state.providers.contains(ProviderId::Claude)
                && state
                    .accounts
                    .claude
                    .profiles
                    .iter()
                    .find(|p| p.id == "default")
                    .map(|p| p.enabled)
                    .unwrap_or(true)
        }
        ContextMenuAction::ToggleClaudeX => {
            state.providers.contains(ProviderId::Claude)
                && state
                    .accounts
                    .claude
                    .profiles
                    .iter()
                    .find(|p| p.id == "claude_x")
                    .map(|p| p.enabled)
                    .unwrap_or(false)
        }
        ContextMenuAction::ToggleCodexMain => {
            state.providers.contains(ProviderId::Codex)
                && state
                    .accounts
                    .codex
                    .profiles
                    .iter()
                    .find(|p| p.id == "default")
                    .map(|p| p.enabled)
                    .unwrap_or(true)
        }
        ContextMenuAction::ToggleCodexX => {
            state.providers.contains(ProviderId::Codex)
                && state
                    .accounts
                    .codex
                    .profiles
                    .iter()
                    .find(|p| p.id == "codex_x")
                    .map(|p| p.enabled)
                    .unwrap_or(false)
        }
        ContextMenuAction::ToggleStartup => is_startup_enabled(),
        ContextMenuAction::ToggleTaskbarLock => state.lock_taskbar,
        ContextMenuAction::ToggleShowPace => state.show_pace,
        ContextMenuAction::ToggleShowFable => state.show_fable,
        ContextMenuAction::ToggleShowFableX => state.show_fable_x,
        ContextMenuAction::ToggleClaudePoolsOneWidget => state.claude_pools_one_widget,
        ContextMenuAction::ToggleCodexPoolsOneWidget => state.codex_pools_one_widget,
        ContextMenuAction::ToggleAntigravityPoolsOneWidget => state.antigravity_pools_one_widget,
        ContextMenuAction::ToggleWidget => state
            .active_theme
            .as_ref()
            .and_then(|theme| {
                let effective = theme_engine::apply_mouse_action_overrides(
                    theme,
                    &state.mouse_action_overrides,
                );
                context_menu_widget_origin(&effective).map(|(surface_index, _)| {
                    let runtime = theme_runtime_for_surface(
                        &effective,
                        surface_index,
                        theme_runtime_from_state(state),
                    );
                    let metrics = state.prepared_metrics.as_ref().map_or_else(
                        || theme_engine::display_metrics(&effective, state.data.as_ref(), runtime),
                        |metrics| theme_engine::metrics_for_phase(metrics.clone(), runtime),
                    );
                    theme_engine::surface_should_render_with_metrics(
                        &effective,
                        surface_index,
                        state.data.as_ref(),
                        runtime,
                        &metrics,
                    )
                })
            })
            .unwrap_or(false),
        ContextMenuAction::SetLanguage { language } => {
            if language.eq_ignore_ascii_case("system") {
                state.language_override.is_none()
            } else {
                state
                    .language_override
                    .is_some_and(|current| current.code().eq_ignore_ascii_case(language))
            }
        }
        ContextMenuAction::ToggleLayerRender { target } => state
            .active_theme
            .as_ref()
            .and_then(|theme| {
                let effective = theme_engine::apply_mouse_action_overrides(
                    theme,
                    &state.mouse_action_overrides,
                );
                // STATE is already locked here. Match the action's first-surface
                // fallback without calling the helper that locks STATE again.
                let origin_surface = origin.map_or(0, |(surface_index, _)| *surface_index);
                toggle_layer_render_state_with_metrics(
                    &effective,
                    origin_surface,
                    target,
                    state.data.as_ref(),
                    theme_runtime_from_state(state),
                    state.prepared_metrics.as_ref(),
                )
            })
            .unwrap_or(false),
        _ => false,
    }
}

#[cfg(test)]
pub(super) fn context_menu_checked(state: &AppState, action: &ContextMenuAction) -> bool {
    context_menu_checked_with_origin(state, action, None)
}

/// Whether a `toggle_layer_render` target currently draws, so its menu row can
/// carry a check mark. The target is a surface id, or a layer inside the
/// surface the menu was opened from, which is what the action itself toggles.
/// An unknown id has no state to report.
fn toggle_layer_render_state_with_metrics(
    theme: &ThemeDocument,
    origin_surface: usize,
    target: &str,
    data: Option<&AppUsageData>,
    base_runtime: ThemeRuntime,
    cached: Option<&theme_engine::DisplayMetrics>,
) -> Option<bool> {
    let metrics = cached
        .cloned()
        .unwrap_or_else(|| theme_engine::display_metrics(theme, data, base_runtime));
    if let Some(surface_index) = theme
        .surfaces
        .iter()
        .position(|surface| surface.id.eq_ignore_ascii_case(target))
    {
        let runtime = theme_runtime_for_surface(theme, surface_index, base_runtime);
        let metrics = theme_engine::metrics_for_phase(metrics, runtime);
        return Some(theme_engine::surface_should_render_with_metrics(
            theme,
            surface_index,
            data,
            runtime,
            &metrics,
        ));
    }
    let object_index = theme
        .surfaces
        .get(origin_surface)?
        .children
        .iter()
        .position(|object| object.id.eq_ignore_ascii_case(target))?;
    let runtime = theme_runtime_for_surface(theme, origin_surface, base_runtime);
    let metrics = theme_engine::metrics_for_phase(metrics, runtime);
    Some(
        theme_engine::resolve_object_bounds_with_metrics(
            theme,
            origin_surface,
            object_index,
            data,
            runtime,
            &metrics,
        )
        .is_some(),
    )
}

pub(super) fn context_menu_action_origin(
    origin: Option<(usize, String)>,
) -> Option<(usize, String)> {
    if origin.is_some() {
        return origin;
    }
    lock_state().as_ref().and_then(|state| {
        state
            .active_theme
            .as_ref()
            .and_then(|theme| theme.surfaces.first())
            .map(|surface| (0, surface.id.clone()))
    })
}

pub(super) fn context_menu_widget_origin(theme: &ThemeDocument) -> Option<(usize, String)> {
    theme
        .surfaces
        .iter()
        .enumerate()
        .find(|(_, surface)| surface.placement.nest != SurfaceNest::TrayIcon)
        .map(|(index, surface)| (index, surface.id.clone()))
}

/// Map an action to the WM_COMMAND id handled by `wnd_proc`. Kept separate so
/// the multi-select loop can dispatch a provider toggle synchronously.
fn context_menu_static_command(action: &ContextMenuAction) -> Option<u16> {
    match action {
        ContextMenuAction::OpenDashboard => Some(IDM_DASHBOARD),
        ContextMenuAction::Refresh => Some(1),
        ContextMenuAction::SetUpdateFrequency { seconds } => match *seconds {
            POLL_1_MIN_SECONDS => Some(IDM_FREQ_1MIN),
            POLL_5_MIN_SECONDS => Some(IDM_FREQ_5MIN),
            POLL_15_MIN_SECONDS => Some(IDM_FREQ_15MIN),
            POLL_1_HOUR_SECONDS => Some(IDM_FREQ_1HOUR),
            _ => None,
        },
        ContextMenuAction::ToggleProvider { provider } => {
            Some(provider.descriptor().native_menu_command_id)
        }
        ContextMenuAction::ToggleAntigravityGemini => Some(IDM_PROVIDER_ANTIGRAVITY_GEMINI),
        ContextMenuAction::ToggleAntigravityExternal => Some(IDM_PROVIDER_ANTIGRAVITY_EXTERNAL),
        ContextMenuAction::ToggleAntigravityA => Some(IDM_PROVIDER_ANTIGRAVITY_A),
        ContextMenuAction::ToggleAntigravityX => Some(IDM_PROVIDER_ANTIGRAVITY_X),
        ContextMenuAction::ToggleAntigravityExternalX => Some(IDM_PROVIDER_ANTIGRAVITY_EXTERNAL_X),
        ContextMenuAction::ToggleClaudeMain => Some(IDM_PROVIDER_CLAUDE_MAIN),
        ContextMenuAction::ToggleClaudeX => Some(IDM_PROVIDER_CLAUDE_X),
        ContextMenuAction::ToggleCodexMain => Some(IDM_PROVIDER_CODEX_MAIN),
        ContextMenuAction::ToggleCodexX => Some(IDM_PROVIDER_CODEX_X),
        ContextMenuAction::ToggleStartup => Some(IDM_START_WITH_WINDOWS),
        ContextMenuAction::ToggleTaskbarLock => Some(IDM_LOCK_TASKBAR),
        ContextMenuAction::ToggleShowPace => Some(IDM_SHOW_PACE),
        ContextMenuAction::ToggleShowFable => Some(IDM_SHOW_FABLE),
        ContextMenuAction::ToggleShowFableX => Some(IDM_SHOW_FABLE_X),
        ContextMenuAction::ToggleClaudePoolsOneWidget => Some(IDM_CLAUDE_POOLS_ONE_WIDGET),
        ContextMenuAction::ToggleCodexPoolsOneWidget => Some(IDM_CODEX_POOLS_ONE_WIDGET),
        ContextMenuAction::ToggleAntigravityPoolsOneWidget => {
            Some(IDM_ANTIGRAVITY_POOLS_ONE_WIDGET)
        }
        ContextMenuAction::SetLanguage { language } => {
            if language.eq_ignore_ascii_case("system") {
                Some(IDM_LANG_SYSTEM)
            } else {
                LanguageId::from_code(language).map(language_menu_command_id)
            }
        }
        ContextMenuAction::CheckForUpdates => Some(IDM_VERSION_ACTION),
        ContextMenuAction::Exit => Some(2),
        ContextMenuAction::ToggleWidget
        | ContextMenuAction::LegacyResetPosition
        | ContextMenuAction::ToggleLayerRender { .. }
        | ContextMenuAction::LayerActions { .. }
        | ContextMenuAction::OpenUrl { .. } => None,
    }
}

pub(super) fn execute_context_menu_action(
    hwnd: HWND,
    action: ContextMenuAction,
    origin: Option<(usize, String)>,
) {
    let static_command = context_menu_static_command(&action);
    if let Some(command) = static_command {
        unsafe {
            let _ = PostMessageW(
                Some(controller_for(hwnd)),
                WM_COMMAND,
                WPARAM(command as usize),
                LPARAM(0),
            );
        }
        return;
    }
    match action {
        ContextMenuAction::ToggleWidget => {
            let target = lock_state()
                .as_ref()
                .and_then(|state| state.active_theme.as_ref())
                .and_then(context_menu_widget_origin);
            if let Some((surface_index, root_id)) = target {
                let _ =
                    execute_mouse_action_source(surface_index, &root_id, "toggle(self, render)");
            }
        }
        ContextMenuAction::ToggleLayerRender { target } => {
            let Some((surface_index, self_id)) = context_menu_action_origin(origin) else {
                return;
            };
            let target = target.replace('\\', "\\\\").replace('"', "\\\"");
            let source = format!("toggle(\"{target}\", render)");
            let _ = execute_mouse_action_source(surface_index, &self_id, &source);
        }
        ContextMenuAction::LayerActions { actions } => {
            let Some((surface_index, self_id)) = context_menu_action_origin(origin) else {
                return;
            };
            let _ = execute_mouse_action_source(surface_index, &self_id, &actions);
        }
        ContextMenuAction::OpenUrl { url } => {
            open_web_url(hwnd, &url, "context menu URL could not be opened")
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shell_work_waits_for_the_open_popup_and_runs_once_after_it() {
        use super::{defer_while_menu_open, set_menu_tracking, DeferredMenuWork};
        assert!(!defer_while_menu_open(DeferredMenuWork::FullRender));
        assert_eq!(set_menu_tracking(false), (false, false));
        set_menu_tracking(true);
        assert!(defer_while_menu_open(DeferredMenuWork::FullRender));
        assert!(defer_while_menu_open(DeferredMenuWork::FullRender));
        assert!(defer_while_menu_open(DeferredMenuWork::Reconcile));
        assert_eq!(set_menu_tracking(false), (true, true));
        assert_eq!(set_menu_tracking(false), (false, false));
        assert!(!defer_while_menu_open(DeferredMenuWork::Reconcile));
    }

    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{GetMenuItemCount, GetMenuItemID, GetSubMenu};

    /// Every row of the Providers submenu keeps the menu open, so a new row
    /// cannot be forgotten in `is_provider_toggle`.
    #[test]
    fn every_providers_submenu_action_is_repeatable() {
        fn walk(items: &[crate::context_menu::ContextMenuItem], out: &mut Vec<ContextMenuAction>) {
            for item in items {
                match &item.kind {
                    ContextMenuItemKind::Action { action } => out.push(action.clone()),
                    ContextMenuItemKind::Submenu { items } => walk(items, out),
                    _ => {}
                }
            }
        }
        let document = crate::context_menu::classic_context_menu();
        let providers = document
            .items
            .iter()
            .find(|item| item.id == "providers")
            .expect("providers submenu");
        let mut actions = Vec::new();
        walk(std::slice::from_ref(providers), &mut actions);
        assert!(actions.len() > 10);
        for action in &actions {
            assert!(is_provider_toggle(action), "{action:?}");
            assert!(context_menu_static_command(action).is_some(), "{action:?}");
        }
    }

    #[test]
    fn provider_toggles_are_repeatable_but_other_actions_close_the_menu() {
        use crate::providers::ProviderId;

        let repeatable = [
            ContextMenuAction::ToggleProvider {
                provider: ProviderId::OpenCode,
            },
            ContextMenuAction::ToggleProvider {
                provider: ProviderId::Grok,
            },
            ContextMenuAction::ToggleAntigravityGemini,
            ContextMenuAction::ToggleAntigravityExternal,
            ContextMenuAction::ToggleAntigravityA,
            ContextMenuAction::ToggleAntigravityX,
            ContextMenuAction::ToggleAntigravityExternalX,
            ContextMenuAction::ToggleClaudeMain,
            ContextMenuAction::ToggleClaudeX,
            ContextMenuAction::ToggleCodexMain,
            ContextMenuAction::ToggleCodexX,
        ];
        for action in &repeatable {
            assert!(is_provider_toggle(action), "{action:?}");
            assert!(
                context_menu_static_command(action).is_some(),
                "{action:?} needs a synchronous WM_COMMAND id"
            );
        }
        assert_eq!(
            context_menu_static_command(&ContextMenuAction::ToggleShowFable),
            Some(super::IDM_SHOW_FABLE)
        );
        assert_eq!(
            context_menu_static_command(&ContextMenuAction::ToggleShowFableX),
            Some(super::IDM_SHOW_FABLE_X)
        );
        for action in [
            ContextMenuAction::Refresh,
            ContextMenuAction::Exit,
            ContextMenuAction::OpenDashboard,
            ContextMenuAction::ToggleShowPace,
            ContextMenuAction::ToggleStartup,
            ContextMenuAction::ToggleTaskbarLock,
            ContextMenuAction::CheckForUpdates,
        ] {
            assert!(!is_provider_toggle(&action), "{action:?}");
        }
    }

    #[test]
    fn menu_for_command_finds_nested_provider_submenus() {
        let mut conditional = vec![
            ContextMenuItem::action("root", "Refresh", ContextMenuAction::Refresh),
            ContextMenuItem::submenu(
                "providers",
                "Providers",
                vec![ContextMenuItem::action(
                    "nested",
                    "Exit",
                    ContextMenuAction::Exit,
                )],
            ),
            ContextMenuItem::action("last", "Exit", ContextMenuAction::Exit),
        ];
        for item in &mut conditional {
            item.render = theme_engine::Expression("1".into());
        }
        let context = DataContext::from_usage(None, &Canvas::default());
        let mut actions = Vec::new();
        unsafe {
            let menu = CreatePopupMenu().unwrap();
            append_context_menu_items(
                menu,
                &conditional,
                localization::detect_system_language(),
                &context,
                None,
                &mut actions,
            );
            let submenu = GetSubMenu(menu, 1);
            assert!(!submenu.is_invalid());
            // root action 1000, nested 1001, last 1002.
            assert_eq!(menu_for_command(menu, 1000).map(|m| m.0), Some(menu.0));
            assert_eq!(menu_for_command(menu, 1001).map(|m| m.0), Some(submenu.0));
            assert_eq!(menu_for_command(menu, 1002).map(|m| m.0), Some(menu.0));
            assert!(menu_for_command(menu, 9999).is_none());
            DestroyMenu(menu).unwrap();
        }
    }

    #[test]
    fn conditional_native_rows_and_subtrees_preserve_action_ids_on_each_open() {
        let mut conditional = vec![
            ContextMenuItem::text("usage", "Weekly usage"),
            ContextMenuItem::separator("divider"),
            ContextMenuItem::action("refresh", "Refresh", ContextMenuAction::Refresh),
            ContextMenuItem::submenu(
                "provider",
                "Provider",
                vec![ContextMenuItem::action(
                    "nested",
                    "Exit",
                    ContextMenuAction::Exit,
                )],
            ),
        ];
        for item in &mut conditional {
            item.render = theme_engine::Expression("providers.codex.enabled".into());
        }
        // This row stays hidden even when its parent becomes visible.
        if let ContextMenuItemKind::Submenu { items } = &mut conditional[3].kind {
            let mut hidden = ContextMenuItem::text("hidden", "Hidden child");
            hidden.render = theme_engine::Expression("0".into());
            items.insert(0, hidden);
        }
        conditional.push(ContextMenuItem::action(
            "exit",
            "Exit",
            ContextMenuAction::Exit,
        ));
        let mut context = DataContext::from_usage(None, &Canvas::default());
        for enabled in [false, true, false] {
            context.insert("providers.codex.enabled", enabled as u8 as f64);
            let mut actions = Vec::new();
            unsafe {
                let menu = CreatePopupMenu().unwrap();
                append_context_menu_items(
                    menu,
                    &conditional,
                    localization::detect_system_language(),
                    &context,
                    None,
                    &mut actions,
                );
                let count = GetMenuItemCount(Some(menu));
                let last_id = GetMenuItemID(menu, count - 1);
                let child_count = if enabled {
                    let submenu = GetSubMenu(menu, 3);
                    Some((GetMenuItemCount(Some(submenu)), GetMenuItemID(submenu, 0)))
                } else {
                    None
                };
                DestroyMenu(menu).unwrap();
                assert_eq!(count, if enabled { 5 } else { 1 });
                assert_eq!(last_id, if enabled { 1002 } else { 1000 });
                if enabled {
                    assert_eq!(child_count, Some((1, 1001)));
                    assert_eq!(
                        actions,
                        vec![
                            ContextMenuAction::Refresh,
                            ContextMenuAction::Exit,
                            ContextMenuAction::Exit
                        ]
                    );
                } else {
                    assert_eq!(actions, vec![ContextMenuAction::Exit]);
                }
            }
        }
    }

    #[test]
    fn a_layer_toggle_row_reports_the_layer_state_a_surface_row_already_reported() {
        let mut theme = ThemeDocument::starter();
        let runtime = ThemeRuntime::new(true, true, true);
        let surface_id = theme.surfaces[0].id.clone();
        let layer_id = theme.surfaces[0].children[0].id.clone();
        theme.surfaces[0].children[0].render = 1.0.into();
        theme.prepare_runtime();

        // A surface id keeps reporting the surface, as it always did.
        assert_eq!(
            toggle_layer_render_state_with_metrics(&theme, 0, &surface_id, None, runtime, None),
            Some(true)
        );
        // A layer id used to report nothing, leaving the row unchecked even
        // while the layer was drawing.
        assert_eq!(
            toggle_layer_render_state_with_metrics(&theme, 0, &layer_id, None, runtime, None),
            Some(true)
        );
        theme.surfaces[0].children[0].render = 0.0.into();
        theme.prepare_runtime();
        assert_eq!(
            toggle_layer_render_state_with_metrics(&theme, 0, &layer_id, None, runtime, None),
            Some(false)
        );
        // An id that matches nothing has no state to report.
        assert_eq!(
            toggle_layer_render_state_with_metrics(
                &theme,
                0,
                "no-such-object",
                None,
                runtime,
                None
            ),
            None
        );
    }

    #[test]
    fn layer_toggle_state_uses_origin_surface_and_live_overrides() {
        let mut theme = ThemeDocument::starter();
        let runtime = ThemeRuntime::new(true, true, true);
        let layer_id = theme.surfaces[0].children[0].id.clone();
        theme.surfaces[0].children[0].render = 1.0.into();
        let mut second = theme.surfaces[0].clone();
        second.id = "second".into();
        second.children[0].render = 0.0.into();
        theme.surfaces.push(second);
        let second_index = theme.surfaces.len() - 1;
        theme.prepare_runtime();
        assert_eq!(
            toggle_layer_render_state_with_metrics(
                &theme,
                second_index,
                &layer_id,
                None,
                runtime,
                None
            ),
            Some(false)
        );
        let mut overrides = HashMap::new();
        theme_engine::execute_mouse_actions(
            &theme,
            0,
            &layer_id,
            "toggle(self, render)",
            None,
            runtime,
            &mut overrides,
        )
        .unwrap();
        let effective = theme_engine::apply_mouse_action_overrides(&theme, &overrides);
        assert_eq!(
            toggle_layer_render_state_with_metrics(
                &effective,
                0,
                &layer_id.to_uppercase(),
                None,
                runtime,
                None
            ),
            Some(false)
        );
        assert_eq!(
            toggle_layer_render_state_with_metrics(&theme, 0, &layer_id, None, runtime, None),
            Some(true)
        );
    }
}
