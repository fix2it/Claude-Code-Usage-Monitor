//! Render every theme surface and hand it to its presenter.

use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum RenderTrigger {
    /// Geometry, host, theme or DPI may have changed.
    Full,
    /// Only hover-phase pixels changed; canvas size and placement are fixed.
    Animation,
}

/// Render every theme surface, then dispatch it to the presenter selected by
/// its nest: DirectComposition for desktop and layered windows elsewhere.
pub(super) fn render_layered() {
    render_layered_impl(RenderTrigger::Full);
}

/// A frame produced by an animation tick. Skips host enumeration, window
/// reconciliation and the positioning cache invalidation that a full render
/// performs; the positioning cache itself still misses on any geometry change.
pub(super) fn render_layered_animation() {
    render_layered_impl(RenderTrigger::Animation);
}

/// Everything one frame reads from `AppState`, copied out under one lock.
struct FrameInput {
    hwnd: HWND,
    theme: std::sync::Arc<ThemeDocument>,
    usage_data: Option<AppUsageData>,
    runtime: ThemeRuntime,
    metrics: theme_engine::DisplayMetrics,
    mirror_hwnds: Vec<SendHwnd>,
    desktop_hwnds: Vec<Option<SendHwnd>>,
}

pub(super) fn render_layered_impl(trigger: RenderTrigger) {
    // While the context menu is on screen a full pass (surface sync, shell
    // re-parenting and z-order) can dismiss it; paint as an animation frame
    // now and run the full pass when the menu closes.
    let trigger = if trigger == RenderTrigger::Full
        && window_context_menu::defer_while_menu_open(
            window_context_menu::DeferredMenuWork::FullRender,
        ) {
        RenderTrigger::Animation
    } else {
        trigger
    };
    if trigger == RenderTrigger::Full {
        // Shell/DPI/theme changes must force one full positioning pass before
        // subsequent animation frames are allowed to reuse the cached result.
        positioning::invalidate_position_cache();
    }
    refresh_dpi();
    if trigger == RenderTrigger::Full {
        sync_surface_windows();
    }
    let Some(frame) = frame_input(trigger) else {
        return;
    };

    // Theme rendering is the widget renderer. Startup and theme changes always
    // install Classic in memory when a selected theme cannot be loaded.
    let hwnd = frame.hwnd;
    if trigger == RenderTrigger::Full {
        set_window_state_timer(hwnd, theme_has_floating_surface(&frame.theme));
    }
    let target_count = frame.theme.surfaces.len();
    for surface_index in 0..target_count {
        render_surface(&frame, trigger, surface_index);
    }

    for target in std::iter::once(hwnd)
        .chain(frame.mirror_hwnds.iter().map(|mirror| mirror.to_hwnd()))
        .skip(target_count)
    {
        unsafe {
            let _ = ShowWindow(target, SW_HIDE);
        }
    }
}

fn frame_input(trigger: RenderTrigger) -> Option<FrameInput> {
    let mut state = lock_state();
    let state = state.as_mut()?;
    let theme = effective_theme_for_frame(state, trigger);
    let runtime = theme_runtime_from_state(state);
    let metrics = prepared_metrics(state, &theme, theme_runtime_for_surface(&theme, 0, runtime));
    Some(FrameInput {
        hwnd: state.hwnd.to_hwnd(),
        theme,
        usage_data: state.data.clone(),
        runtime,
        metrics,
        mirror_hwnds: state.mirror_hwnds.clone(),
        desktop_hwnds: state.desktop_hwnds.clone(),
    })
}

/// Rebuild the tile hit-test rects of one surface while they are missing.
///
/// Each rect is rebuilt only while it is `None`. A theme without a provider
/// object keeps that rect `None` forever, so hover-only animation frames skip
/// the lookup; full renders and view-mode frames (whose geometry changes)
/// still rebuild missing rects. Every surface has its own rects: a mirror of a
/// custom theme can place the tiles elsewhere.
fn refresh_tile_rects(
    frame: &FrameInput,
    trigger: RenderTrigger,
    surface_index: usize,
    surface_runtime: ThemeRuntime,
    surface_metrics: &theme_engine::DisplayMetrics,
) {
    let mut state = lock_state();
    if let Some(state) = state.as_mut().filter(|state| {
        trigger == RenderTrigger::Full || state.widget_animations.iter().any(Option::is_some)
    }) {
        if state.tiles.rects_incomplete(surface_index) {
            let bounds = theme_engine::object_bounds_for_ids_with_metrics(
                &frame.theme,
                surface_index,
                frame.usage_data.as_ref(),
                surface_runtime,
                &tiles::Tiles::object_ids(),
                surface_metrics,
            );
            state.tiles.set_rects(surface_index, &bounds);
        }
    }
}

/// Render one theme surface and present it into its window, or hide that
/// window when the surface does not render.
fn render_surface(frame: &FrameInput, trigger: RenderTrigger, surface_index: usize) {
    let regular_hwnd = if surface_index == 0 {
        frame.hwnd
    } else if let Some(mirror) = frame.mirror_hwnds.get(surface_index - 1) {
        mirror.to_hwnd()
    } else {
        return;
    };
    let surface = &frame.theme.surfaces[surface_index];
    let surface_runtime = theme_runtime_for_surface(&frame.theme, surface_index, frame.runtime);
    let surface_metrics = if surface_index == 0 {
        std::borrow::Cow::Borrowed(&frame.metrics)
    } else {
        std::borrow::Cow::Owned(theme_engine::metrics_for_surface(
            &frame.theme,
            frame.usage_data.as_ref(),
            &frame.metrics,
            surface_runtime,
        ))
    };
    refresh_tile_rects(
        frame,
        trigger,
        surface_index,
        surface_runtime,
        &surface_metrics,
    );
    let nest = surface
        .placement
        .nest
        .resolve(surface.placement.reference.region);
    let desktop_nested = nest == SurfaceNest::Desktop;
    let target_hwnd = if desktop_nested {
        unsafe {
            let _ = ShowWindow(regular_hwnd, SW_HIDE);
        }
        frame
            .desktop_hwnds
            .get(surface_index)
            .and_then(|window| *window)
            .map(SendHwnd::to_hwnd)
            .unwrap_or_default()
    } else {
        regular_hwnd
    };
    if target_hwnd.is_invalid() {
        return;
    }
    if nest == SurfaceNest::TrayIcon {
        unsafe {
            let _ = ShowWindow(target_hwnd, SW_HIDE);
        }
        return;
    }
    if !theme_engine::surface_should_render_with_metrics(
        &frame.theme,
        surface_index,
        frame.usage_data.as_ref(),
        surface_runtime,
        &surface_metrics,
    ) {
        unsafe {
            let _ = ShowWindow(target_hwnd, SW_HIDE);
        }
        return;
    }

    // A hover frame skips `sync_surface_windows`, so never present into a
    // surface that disappeared since the last full render.
    if trigger == RenderTrigger::Animation && unsafe { !IsWindow(Some(target_hwnd)).as_bool() } {
        return;
    }
    let scale = theme_surface_scale(&frame.theme, surface_index);
    let rendered = theme_engine::render_theme_surface_with_metrics_at_scale(
        &frame.theme,
        surface_index,
        frame.usage_data.as_ref(),
        surface_runtime,
        scale,
        &surface_metrics,
    );
    let mut positioned = theme_for_surface(&frame.theme, surface_index);
    let ((logical_width, logical_height), placement) =
        theme_engine::resolve_surface_geometry_with_metrics(
            &frame.theme,
            surface_index,
            frame.usage_data.as_ref(),
            surface_runtime,
            &surface_metrics,
        );
    positioned.canvas.width = logical_width;
    positioned.canvas.height = logical_height;
    positioned.placement.offset_x = placement.offset_x;
    positioned.placement.offset_y = placement.offset_y;
    let hosted = position_custom_theme(target_hwnd, &positioned, scale);
    if !hosted {
        theme_engine::recycle_pixel_buffer(rendered.pixels);
        return;
    }
    if desktop_nested {
        unsafe {
            let _ = ShowWindow(target_hwnd, SW_SHOWNOACTIVATE);
        }
    }
    render_custom_window(target_hwnd, &rendered, desktop_nested);
    theme_engine::recycle_pixel_buffer(rendered.pixels);
    unsafe {
        let show = nest != SurfaceNest::Floating
            || !foreground_is_fullscreen_on_display(positioned.placement.reference.display);
        let _ = ShowWindow(target_hwnd, if show { SW_SHOWNOACTIVATE } else { SW_HIDE });
    }
}

fn theme_for_surface(theme: &ThemeDocument, surface_index: usize) -> ThemeDocument {
    // Positioning reads only canvas dimensions and placement, not the scene graph.
    ThemeDocument {
        schema_version: theme.schema_version,
        id: String::new(),
        name: String::new(),
        canvas: theme.canvas.clone(),
        placement: theme.surfaces.get(surface_index).map_or_else(
            || theme.placement.clone(),
            |surface| surface.placement.clone(),
        ),
        children: Vec::new(),
        surfaces: Vec::new(),
    }
}
