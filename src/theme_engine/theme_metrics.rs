use super::*;

pub(super) const COMPACT_PERCENT_WIDTH: f64 = 30.0;
pub(super) const COMPACT_CURSOR_TEXT_WIDTH: f64 = 80.0;
pub(super) const DISPLAY_METRICS_CACHE_CAPACITY: usize = 8;

/// One provider's adaptive column. `text_slide` and `row_width` are already
/// resolved for the runtime's mode phase, so the theme never recomputes them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ProviderMetrics {
    pub text_box: f64,
    pub percent_box: f64,
    pub marker_box: f64,
    pub origin: f64,
    pub text_slide: f64,
    pub row_width: f64,
}

/// Measured Compact geometry for the current data snapshot and mode phase.
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayMetrics {
    providers: [ProviderMetrics; 8],
    pub providers_width: f64,
    /// Providers plus the Fable column when shown; sizes every bar.
    pub widget_count: usize,
    /// The Fable column, measured like a provider. Its `text_box` is 0 when
    /// the line cannot be measured, so the theme's `max(bar, text_box.fable)`
    /// falls back to the bar width.
    pub(super) fable: ProviderMetrics,
    /// The agy external-models widget is shown.
    agy_split: bool,
    /// The agy external-models widget, measured and moded on its own. It is
    /// only shown with pool O, so it always has a marker column.
    agy_external: ProviderMetrics,
    claude_x: ProviderMetrics,
    codex_x: ProviderMetrics,
    antigravity_o: ProviderMetrics,
    agy_external_o: ProviderMetrics,
    claude_separate: bool,
    codex_separate: bool,
    agy_separate: bool,
}

impl DisplayMetrics {
    #[cfg(test)]
    pub fn provider(&self, key: &str) -> Option<ProviderMetrics> {
        DISPLAY_PROVIDER_KEYS
            .iter()
            .position(|candidate| *candidate == key)
            .map(|index| self.providers[index])
    }

    /// Legacy reserves, used whenever a line cannot be measured and as the
    /// base context that theme validation and Studio build before data exists.
    pub fn legacy(runtime: ThemeRuntime) -> Self {
        Self::legacy_with_widgets(runtime, runtime.provider_count())
    }

    pub(crate) fn legacy_with_widgets(runtime: ThemeRuntime, widget_count: usize) -> Self {
        let bar = compact_bar_full_width(widget_count);
        let mut metrics = Self {
            providers: [ProviderMetrics::default(); 8],
            providers_width: 0.0,
            widget_count,
            fable: ProviderMetrics::default(),
            agy_split: antigravity_split(None, runtime),
            agy_external: ProviderMetrics::default(),
            claude_x: ProviderMetrics::default(),
            codex_x: ProviderMetrics::default(),
            antigravity_o: ProviderMetrics::default(),
            agy_external_o: ProviderMetrics::default(),
            claude_separate: pool_separate(runtime, ProviderId::Claude),
            codex_separate: pool_separate(runtime, ProviderId::Codex),
            agy_separate: agy_separate(None, runtime),
        };
        let dual = claude_dual(runtime);
        metrics.fable = fable_column(
            bar,
            if dual { COMPACT_TEXT_WIDTH } else { 0.0 },
            COMPACT_PERCENT_WIDTH,
            COMPACT_MARKER_WIDTH,
            COMPACT_ORIGIN_LEGACY,
            decompose_mode_phase(runtime.widget_mode_phase(FABLE_METRICS_INDEX)),
            dual,
        );
        for (index, key) in DISPLAY_PROVIDER_KEYS.iter().enumerate() {
            let phases = decompose_mode_phase(runtime.widget_mode_phase(index));
            let text_box = if *key == "cursor" {
                COMPACT_CURSOR_TEXT_WIDTH
            } else {
                COMPACT_TEXT_WIDTH
            };
            let (marker_box, origin) = match *key {
                "claude" | "codex" | "opencode" | "qoder" => {
                    (COMPACT_MARKER_WIDTH, COMPACT_ORIGIN_LEGACY)
                }
                _ => (0.0, 0.0),
            };
            metrics.providers[index] = ProviderMetrics {
                text_box,
                percent_box: COMPACT_PERCENT_WIDTH,
                marker_box,
                origin,
                text_slide: -text_box * phases.text_collapse,
                row_width: compact_row_width(bar, text_box, COMPACT_PERCENT_WIDTH, origin, phases),
            };
        }
        metrics.agy_external = agy_external_column(
            bar,
            COMPACT_TEXT_WIDTH,
            COMPACT_PERCENT_WIDTH,
            COMPACT_ORIGIN_LEGACY,
            decompose_mode_phase(runtime.widget_mode_phase(AGY_EXTERNAL_INDEX)),
            COMPACT_MARKER_WIDTH,
        );
        metrics.claude_x = metrics.providers[1];
        metrics.codex_x = metrics.providers[2];
        metrics.antigravity_o = metrics.providers[3];
        metrics.agy_external_o = metrics.agy_external;
        metrics.claude_x.origin = 0.0;
        metrics.codex_x.origin = 0.0;
        metrics.antigravity_o.origin = 0.0;
        metrics = metrics_for_phase(metrics, runtime);
        metrics.providers_width = metrics.providers_width(runtime);
        metrics
    }

    /// The agy external-models widget's geometry.
    #[cfg(test)]
    pub fn agy_external(&self) -> ProviderMetrics {
        self.agy_external
    }

    /// Enabled provider columns plus the agy external-models widget, sized
    /// by its own measurement and mode.
    fn providers_width(&self, runtime: ThemeRuntime) -> f64 {
        let agy = self.agy_external.row_width;
        compact_providers_width(
            DISPLAY_PROVIDER_KEYS
                .iter()
                .enumerate()
                .filter_map(|(index, key)| {
                    runtime
                        .provider_enabled(display_provider_id(key))
                        .then_some(self.providers[index].row_width)
                })
                .chain(self.agy_split.then_some(agy))
                .chain(self.claude_separate.then_some(self.claude_x.row_width))
                .chain(self.codex_separate.then_some(self.codex_x.row_width))
                .chain(self.agy_separate.then_some(self.antigravity_o.row_width)),
            runtime.provider_count()
                + self.agy_split as usize
                + self.claude_separate as usize
                + self.codex_separate as usize
                + self.agy_separate as usize,
        )
    }
}

pub(super) const COMPACT_ORIGIN_LEGACY: f64 = 12.0;

/// The agy external-models widget: a provider-style column with a pool
/// marker column (`a`/`o`) before the bars.
pub(super) fn agy_external_column(
    bar_full_width: f64,
    text_box: f64,
    percent_box: f64,
    origin: f64,
    phases: ModePhases,
    marker_box: f64,
) -> ProviderMetrics {
    ProviderMetrics {
        text_box,
        percent_box,
        marker_box,
        origin,
        text_slide: -text_box * phases.text_collapse,
        row_width: compact_row_width(bar_full_width, text_box, percent_box, origin, phases),
    }
}

pub(super) fn display_provider_id(key: &str) -> ProviderId {
    match key {
        "claude" => ProviderId::Claude,
        "codex" => ProviderId::Codex,
        "antigravity" => ProviderId::Antigravity,
        "opencode" => ProviderId::OpenCode,
        "cursor" => ProviderId::Cursor,
        "grok" => ProviderId::Grok,
        "copilot" => ProviderId::Copilot,
        "qoder" => ProviderId::Qoder,
        other => {
            debug_assert!(false, "unknown Compact provider key {other}");
            ProviderId::Claude
        }
    }
}

/// `display.row_width.<key>` for one row: the visible bar plus whatever text,
/// percent badge and marker column the current mode phase keeps on screen.
pub fn compact_row_width(
    bar_full_width: f64,
    text_box: f64,
    percent_box: f64,
    origin: f64,
    phases: ModePhases,
) -> f64 {
    bar_full_width * (1.0 - phases.bar_collapse)
        + COMPACT_ROW_GAP * (1.0 - phases.text_collapse)
        + text_box * (1.0 - phases.text_collapse)
        + percent_box * phases.bar_collapse
        + origin
}

/// The Fable column: a marker, then the bar stacked above its text line. The
/// bar still sets the minimum width, so the stacked text extent is expressed
/// through `compact_row_width` as `max(bar, text) - bar - gap`.
pub(super) fn fable_column(
    bar_full_width: f64,
    text_box: f64,
    percent_box: f64,
    marker_box: f64,
    origin: f64,
    phases: ModePhases,
    dual: bool,
) -> ProviderMetrics {
    if dual {
        // Two pool rows, each with its text beside the bar like a provider.
        return ProviderMetrics {
            text_box,
            percent_box,
            marker_box,
            origin,
            text_slide: -text_box * phases.text_collapse,
            row_width: compact_row_width(bar_full_width, text_box, percent_box, origin, phases),
        };
    }
    let stacked_text = bar_full_width.max(text_box) - bar_full_width - COMPACT_ROW_GAP;
    ProviderMetrics {
        text_box,
        percent_box,
        marker_box,
        origin,
        text_slide: -text_box * phases.text_collapse,
        row_width: compact_row_width(bar_full_width, stacked_text, percent_box, origin, phases),
    }
}

/// Sum of the enabled rows plus the 6 px inter-widget gap. `count` is already
/// clamped to one row exactly like `providers.count`.
pub fn compact_providers_width(rows: impl IntoIterator<Item = f64>, count: usize) -> f64 {
    let rows: f64 = rows.into_iter().sum();
    rows + count.saturating_sub(1) as f64 * 6.0
}

/// Container of the agy external-models widget shown next to the agy one.
pub(crate) const ANTIGRAVITY_EXTERNAL_WIDGET: &str = "antigravity-external-provider";

/// Slot after the eight providers: the Fable column's text measurement.
pub(super) const FABLE_METRICS_INDEX: usize = DISPLAY_PROVIDER_KEYS.len();
pub(super) const FABLE_KEY: &str = "fable";
/// Widget key (mode slot, settings key and `display.*.<key>` bindings) of the
/// agy external-models widget; its container is `ANTIGRAVITY_EXTERNAL_WIDGET`.
pub(crate) const AGY_EXTERNAL_KEY: &str = "agy_external";
/// Slot after the Fable column: the agy external-models widget.
pub(super) const AGY_EXTERNAL_INDEX: usize = FABLE_METRICS_INDEX + 1;
/// Providers, the Fable column and the agy external-models widget.
pub(crate) const WIDGET_SLOTS: usize = AGY_EXTERNAL_INDEX + 5;

pub(super) fn display_provider_key(provider_id: &str) -> Option<&'static str> {
    // The agy external-models widget is measured as its own widget.
    if provider_id.eq_ignore_ascii_case(ANTIGRAVITY_EXTERNAL_WIDGET) {
        return Some(AGY_EXTERNAL_KEY);
    }
    widget_keys().find(|key| provider_id.eq_ignore_ascii_case(&format!("{key}-provider")))
}

pub(super) fn scene_provider_key<'a>(
    object: &'a SceneObject,
    by_id: &HashMap<&'a str, &'a SceneObject>,
) -> Option<&'static str> {
    let mut current = object.parent.as_deref();
    while let Some(parent_id) = current {
        if let Some(key) = display_provider_key(parent_id) {
            return Some(key);
        }
        current = by_id
            .get(parent_id)
            .and_then(|parent| parent.parent.as_deref());
    }
    None
}

pub(super) fn scene_object_renders(
    object: &SceneObject,
    by_id: &HashMap<&str, &SceneObject>,
    context: &DataContext,
) -> bool {
    let mut current = Some(object);
    while let Some(candidate) = current {
        if !evaluate(&candidate.render.0, context)
            .is_ok_and(|value| value.is_finite() && value != 0.0)
        {
            return false;
        }
        current = candidate
            .parent
            .as_deref()
            .and_then(|parent_id| by_id.get(parent_id).copied());
    }
    true
}

/// Mirrors `format_template`'s token walk and reports whether a usage
/// substitution produced a placeholder. Cursor prefixes (`int `, `ext `) mean
/// the whole string must never be compared against `--`.
pub(super) fn template_has_placeholder_usage(template: &str, context: &DataContext) -> bool {
    let chars: Vec<char> = template.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '{' && index + 1 < chars.len() && chars[index + 1] == '{' {
            index += 2;
            continue;
        }
        if chars[index] != '{' {
            index += 1;
            continue;
        }
        let Some(relative_end) = chars[index + 1..].iter().position(|c| *c == '}') else {
            index += 1;
            continue;
        };
        let end = index + 1 + relative_end;
        let token: String = chars[index + 1..end].iter().collect();
        let (expression, format) = split_template_token(&token);
        let expression = expression.trim();
        let format = format.trim();
        let value = if !format.eq_ignore_ascii_case("usage_line")
            && !format.eq_ignore_ascii_case("usage_badge")
        {
            None
        } else if let Some(value) = context.get_string(expression) {
            Some(value.to_string())
        } else if format.eq_ignore_ascii_case("usage_line") {
            Some(format_usage_line(expression, context).unwrap_or_else(|| "--".into()))
        } else if format.eq_ignore_ascii_case("usage_badge") {
            Some(format_usage_badge(expression, context).unwrap_or_else(|| "--".into()))
        } else {
            None
        };
        if let Some(value) = value {
            if value.is_empty() || value == "--" || value == "!" {
                return true;
            }
        }
        index = end + 1;
    }
    false
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum MeasuredLineKind {
    Text,
    Percent,
    Marker,
}

pub(super) struct MeasuredLine {
    provider_index: usize,
    kind: MeasuredLineKind,
    text: String,
    family: String,
    font_size: f64,
    weight: i32,
}

/// Build the eight extremes the theme can show: normal/hover and the text and
/// percent mode phases, each in both system themes. `visibility` is ignored
/// because it follows the mode animations; `render` is not.
pub(super) fn measurement_contexts(
    data: Option<&AppUsageData>,
    runtime: ThemeRuntime,
) -> Vec<DataContext> {
    let mut contexts = Vec::with_capacity(8);
    for dark in [0u8, 1] {
        for (phase, view_mode) in [(0.0, 1), (2.0, 3)] {
            for hover in [false, true] {
                let mut variant = runtime;
                variant.mode_phase = phase;
                variant.widget_mode_phases = [None; WIDGET_SLOTS];
                variant.view_mode = view_mode;
                let (claude, codex, antigravity, opencode, qoder) = if hover {
                    (1.0, 1.0, 0.0, 1.0, 1.0)
                } else {
                    (0.0, 0.0, 1.0, 0.0, 0.0)
                };
                variant.claude_hovered = hover;
                variant.codex_hovered = hover;
                variant.claude_crossfade_opacity = claude;
                variant.codex_crossfade_opacity = codex;
                variant.crossfade_opacity = antigravity;
                variant.opencode_crossfade_opacity = opencode;
                variant.qoder_crossfade_opacity = qoder;
                variant.agy_external_crossfade_opacity = claude;
                variant.split_hover_phases = [claude; SplitPool::ALL.len()];
                let mut context =
                    DataContext::from_usage_with_runtime(data, &Canvas::default(), variant);
                context.insert("system.dark", f64::from(dark));
                contexts.push(context);
            }
        }
    }
    contexts
}

pub(super) fn display_metrics_cache_key(
    theme: &ThemeDocument,
    runtime: ThemeRuntime,
    data: Option<&AppUsageData>,
    surface: &SceneObject,
    fingerprint: &str,
) -> String {
    let base = DataContext::from_usage_with_runtime(data, &Canvas::default(), runtime);
    let mut key = format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|",
        theme.id,
        runtime.countdown,
        runtime.use_windows_default_font,
        base.get("display.widget_count").unwrap_or(0.0),
        DISPLAY_PROVIDER_KEYS
            .iter()
            .filter(|key| runtime.provider_enabled(display_provider_id(key)))
            .copied()
            .collect::<Vec<_>>()
            .join(","),
        base.get("claude.dual_mode").unwrap_or(0.0),
        base.get("codex.dual_mode").unwrap_or(0.0),
        base.get("qoder.session.available").unwrap_or(0.0),
        base.get("qoder.week.available").unwrap_or(0.0),
        base.get("antigravity.pool_o").unwrap_or(0.0),
        base.get("display.agy_split").unwrap_or(0.0),
    );
    key.push_str(&surface.id);
    key.push('|');
    key.push_str(fingerprint);
    key
}

pub(super) fn cached_display_metrics(key: &str) -> Option<DisplayMetrics> {
    let cache = DISPLAY_METRICS_CACHE.get_or_init(|| Mutex::new(VecDeque::new()));
    let cache = cache.lock().ok()?;
    cache
        .iter()
        .find(|(candidate, _)| candidate == key)
        .map(|(_, metrics)| metrics.clone())
}

pub(super) fn store_display_metrics(key: String, metrics: &DisplayMetrics) {
    let cache = DISPLAY_METRICS_CACHE.get_or_init(|| Mutex::new(VecDeque::new()));
    let Ok(mut cache) = cache.lock() else {
        return;
    };
    if cache.len() >= DISPLAY_METRICS_CACHE_CAPACITY {
        cache.pop_front();
    }
    cache.push_back((key, metrics.clone()));
}

pub(super) static DISPLAY_METRICS_CACHE: OnceLock<Mutex<VecDeque<(String, DisplayMetrics)>>> =
    OnceLock::new();

/// Measure every Full/percent/marker line the Compact surface can show.
/// Repeated calls in one frame reuse the cached result through the theme,
/// formatted-string and runtime fingerprint.
pub(crate) fn display_metrics(
    theme: &ThemeDocument,
    data: Option<&AppUsageData>,
    runtime: ThemeRuntime,
) -> DisplayMetrics {
    #[cfg(test)]
    METRICS_COMPUTATION_COUNT.with(|count| count.set(count.get() + 1));
    let widgets = widget_count(data, runtime);
    let Some(surface) = theme.surfaces.first() else {
        return DisplayMetrics::legacy_with_widgets(runtime, widgets);
    };
    let (lines, fingerprint) = measured_lines(surface, &measurement_contexts(data, runtime));
    let key = display_metrics_cache_key(theme, runtime, data, surface, &fingerprint);
    if let Some(metrics) = cached_display_metrics(&key) {
        return metrics_for_phase(metrics, runtime);
    }
    let advances = LineAdvances::measure(&lines);
    let metrics = measured_display_metrics(&advances, data, runtime, widgets);
    store_display_metrics(key, &metrics);
    metrics_for_phase(metrics, runtime)
}

/// Every text line each provider can show across the measurement contexts,
/// plus a fingerprint of everything that affects their measured width.
fn measured_lines(surface: &SceneObject, contexts: &[DataContext]) -> (Vec<MeasuredLine>, String) {
    let by_id: HashMap<&str, &SceneObject> = surface
        .children
        .iter()
        .map(|object| (object.id.as_str(), object))
        .collect();
    let mut fingerprint = String::new();
    let mut lines = Vec::new();
    for object in &surface.children {
        let SceneContent::Text {
            template,
            font_family,
            font_size,
            weight,
            ..
        } = &object.content
        else {
            continue;
        };
        let Some(key) = scene_provider_key(object, &by_id) else {
            continue;
        };
        let provider_index = widget_mode_index(key).unwrap_or(0);
        let kind = if object.id.contains("-marker-") {
            MeasuredLineKind::Marker
        } else if object.id.ends_with("-pct") {
            MeasuredLineKind::Percent
        } else {
            MeasuredLineKind::Text
        };
        fingerprint.push_str(&format!(
            "{}|{}|{}|{}|{}|{}|",
            object.id,
            template,
            font_family,
            font_size.0,
            weight.gdi_weight(),
            object.render.0
        ));
        for context in contexts {
            if !scene_object_renders(object, &by_id, context) {
                fingerprint.push_str("0;");
                continue;
            }
            let text = format_template(template, context);
            if kind != MeasuredLineKind::Marker && template_has_placeholder_usage(template, context)
            {
                fingerprint.push_str("p;");
                continue;
            }
            fingerprint.push_str("1:");
            fingerprint.push_str(&text);
            fingerprint.push(';');
            if text.is_empty() {
                continue;
            }
            let font_size = evaluate(&font_size.0, context).unwrap_or(13.0);
            let family = if context.force_segoe_ui {
                crate::widget_font::segoe_ui_family(weight.gdi_weight()).to_string()
            } else {
                font_family.clone()
            };
            fingerprint.push_str(&format!("font:{family}:{font_size};"));
            lines.push(MeasuredLine {
                provider_index,
                kind,
                text,
                family,
                font_size,
                weight: weight.gdi_weight(),
            });
        }
    }
    (lines, fingerprint)
}

/// Widest measured advance per widget slot and line kind.
struct LineAdvances {
    text: [Option<f64>; WIDGET_SLOTS],
    percent: [Option<f64>; WIDGET_SLOTS],
    marker: [Option<f64>; WIDGET_SLOTS],
}

impl LineAdvances {
    fn measure(lines: &[MeasuredLine]) -> Self {
        let mut advances = Self {
            text: [None; WIDGET_SLOTS],
            percent: [None; WIDGET_SLOTS],
            marker: [None; WIDGET_SLOTS],
        };
        let mut measured = HashMap::new();
        for line in lines {
            let advance = *measured
                .entry((
                    &line.text,
                    &line.family,
                    line.font_size.to_bits(),
                    line.weight,
                ))
                .or_insert_with(|| {
                    #[cfg(test)]
                    MEASUREMENT_COUNT.with(|count| count.set(count.get() + 1));
                    text_measure::measure_text(
                        &line.text,
                        &line.family,
                        line.font_size,
                        line.weight,
                    )
                });
            let Some(advance) = advance else {
                continue;
            };
            let slot = match line.kind {
                MeasuredLineKind::Text => &mut advances.text[line.provider_index],
                MeasuredLineKind::Percent => &mut advances.percent[line.provider_index],
                MeasuredLineKind::Marker => &mut advances.marker[line.provider_index],
            };
            *slot = Some((*slot).map_or(advance, |current: f64| current.max(advance)));
        }
        advances
    }

    /// Measured text column (plus a 2 px reserve) or `fallback`.
    fn text_box(&self, index: usize, fallback: f64) -> f64 {
        self.text[index].map_or(fallback, |advance| advance + 2.0)
    }

    fn percent_box(&self, index: usize, fallback: f64) -> f64 {
        self.percent[index].map_or(fallback, |advance| advance + 2.0)
    }
}

/// Which providers currently show a pool marker column.
struct PoolMarkers {
    claude_dual: bool,
    codex_dual: bool,
    qoder_pool: bool,
    agy_pool_o: bool,
}

fn measured_display_metrics(
    advances: &LineAdvances,
    data: Option<&AppUsageData>,
    runtime: ThemeRuntime,
    widgets: usize,
) -> DisplayMetrics {
    let base = DataContext::from_usage_with_runtime(data, &Canvas::default(), runtime);
    let flag = |name: &str| base.get(name).unwrap_or(0.0) != 0.0;
    let markers = PoolMarkers {
        claude_dual: flag("claude.dual_mode"),
        codex_dual: flag("codex.dual_mode"),
        qoder_pool: flag("qoder.session.available") || flag("qoder.week.available"),
        agy_pool_o: flag("antigravity.pool_o"),
    };
    let legacy = DisplayMetrics::legacy_with_widgets(runtime, widgets);
    let bar = compact_bar_full_width(widgets);
    let phases = |index: usize| decompose_mode_phase(runtime.widget_mode_phase(index));
    let mut metrics = legacy.clone();
    metrics.agy_split = flag("display.agy_split");
    metrics.agy_separate = agy_separate(data, runtime);
    let (fable_marker_box, fable_origin) =
        marker_column(advances.marker[FABLE_METRICS_INDEX], legacy.fable);
    metrics.fable = fable_column(
        bar,
        advances.text_box(FABLE_METRICS_INDEX, legacy.fable.text_box),
        advances.percent_box(FABLE_METRICS_INDEX, COMPACT_PERCENT_WIDTH),
        fable_marker_box,
        fable_origin,
        phases(FABLE_METRICS_INDEX),
        markers.claude_dual,
    );
    let (external_marker_box, external_origin) =
        marker_column(advances.marker[AGY_EXTERNAL_INDEX], legacy.agy_external);
    metrics.agy_external = agy_external_column(
        bar,
        advances.text_box(AGY_EXTERNAL_INDEX, legacy.agy_external.text_box),
        advances.percent_box(AGY_EXTERNAL_INDEX, legacy.agy_external.percent_box),
        external_origin,
        phases(AGY_EXTERNAL_INDEX),
        external_marker_box,
    );
    for (index, key) in DISPLAY_PROVIDER_KEYS.iter().enumerate() {
        let fallback = legacy.providers[index];
        let text_box = advances.text_box(index, fallback.text_box);
        let percent_box = advances.percent_box(index, fallback.percent_box);
        let (marker_box, origin) =
            provider_marker_column(key, advances.marker[index], fallback, &markers);
        let phases = phases(index);
        metrics.providers[index] = ProviderMetrics {
            text_box,
            percent_box,
            marker_box,
            origin,
            text_slide: -text_box * phases.text_collapse,
            row_width: compact_row_width(bar, text_box, percent_box, origin, phases),
        };
    }
    for (index, target) in [
        (AGY_EXTERNAL_INDEX + 1, &mut metrics.claude_x),
        (AGY_EXTERNAL_INDEX + 2, &mut metrics.antigravity_o),
        (AGY_EXTERNAL_INDEX + 3, &mut metrics.agy_external_o),
        (AGY_EXTERNAL_INDEX + 4, &mut metrics.codex_x),
    ] {
        let phases = phases(index);
        target.text_box = advances.text_box(index, target.text_box);
        target.percent_box = advances.percent_box(index, target.percent_box);
        if index != AGY_EXTERNAL_INDEX + 3 {
            target.origin = 0.0;
        }
        target.text_slide = -target.text_box * phases.text_collapse;
        target.row_width = compact_row_width(
            bar,
            target.text_box,
            target.percent_box,
            target.origin,
            phases,
        );
    }
    metrics.providers_width = metrics.providers_width(runtime);
    metrics
}

/// Marker box and text origin of a main provider row.
fn provider_marker_column(
    key: &str,
    advance: Option<f64>,
    fallback: ProviderMetrics,
    markers: &PoolMarkers,
) -> (f64, f64) {
    match key {
        "claude" if !markers.claude_dual => (0.0, 0.0),
        "codex" if !markers.codex_dual => (0.0, 0.0),
        "claude" | "codex" => marker_column(advance, fallback),
        "opencode" => (fallback.marker_box, fallback.origin),
        "qoder" if markers.qoder_pool => marker_column(advance, fallback),
        "qoder" => (fallback.marker_box, fallback.origin),
        "antigravity" if markers.agy_pool_o => marker_column(
            advance,
            ProviderMetrics {
                marker_box: COMPACT_MARKER_WIDTH,
                origin: COMPACT_ORIGIN_LEGACY,
                ..fallback
            },
        ),
        _ => (0.0, 0.0),
    }
}

pub(crate) fn metrics_for_phase(
    mut metrics: DisplayMetrics,
    runtime: ThemeRuntime,
) -> DisplayMetrics {
    let bar = compact_bar_full_width(metrics.widget_count);
    for (index, provider) in metrics.providers.iter_mut().enumerate() {
        let phases = decompose_mode_phase(runtime.widget_mode_phase(index));
        provider.text_slide = -provider.text_box * phases.text_collapse;
        provider.row_width = compact_row_width(
            bar,
            provider.text_box,
            provider.percent_box,
            provider.origin,
            phases,
        );
    }
    metrics.fable = fable_column(
        bar,
        metrics.fable.text_box,
        metrics.fable.percent_box,
        metrics.fable.marker_box,
        metrics.fable.origin,
        decompose_mode_phase(runtime.widget_mode_phase(FABLE_METRICS_INDEX)),
        claude_dual(runtime),
    );
    let external = metrics.agy_external;
    metrics.agy_external = agy_external_column(
        bar,
        external.text_box,
        external.percent_box,
        external.origin,
        decompose_mode_phase(runtime.widget_mode_phase(AGY_EXTERNAL_INDEX)),
        external.marker_box,
    );
    for (index, target) in [
        (AGY_EXTERNAL_INDEX + 1, &mut metrics.claude_x),
        (AGY_EXTERNAL_INDEX + 2, &mut metrics.antigravity_o),
        (AGY_EXTERNAL_INDEX + 3, &mut metrics.agy_external_o),
        (AGY_EXTERNAL_INDEX + 4, &mut metrics.codex_x),
    ] {
        let phases = decompose_mode_phase(runtime.widget_mode_phase(index));
        target.text_slide = -target.text_box * phases.text_collapse;
        target.row_width = compact_row_width(
            bar,
            target.text_box,
            target.percent_box,
            target.origin,
            phases,
        );
    }
    metrics.providers_width = metrics.providers_width(runtime);
    metrics
}

/// Measured marker column: widest available glyph plus a 1 px frame and a
/// 2 px gap to the bar. Falls back to the legacy 10/12 when GDI refused.
pub(super) fn marker_column(advance: Option<f64>, fallback: ProviderMetrics) -> (f64, f64) {
    match advance {
        Some(advance) => {
            let marker_box = advance + 1.0;
            (marker_box, marker_box + 2.0)
        }
        None => (fallback.marker_box, fallback.origin),
    }
}

pub(super) fn insert_display_metrics(context: &mut DataContext, metrics: &DisplayMetrics) {
    context.display_metrics = Some(metrics.clone());
}

pub(super) fn display_metric(metrics: &DisplayMetrics, name: &str) -> Option<f64> {
    if name.eq_ignore_ascii_case("display.providers_width") {
        return Some(metrics.providers_width);
    }
    let (prefix, key) = name.rsplit_once('.')?;
    let provider = if key.eq_ignore_ascii_case(FABLE_KEY) {
        metrics.fable
    } else if key.eq_ignore_ascii_case(AGY_EXTERNAL_KEY) {
        metrics.agy_external
    } else if key.eq_ignore_ascii_case("claude_x") {
        metrics.claude_x
    } else if key.eq_ignore_ascii_case("codex_x") {
        metrics.codex_x
    } else if key.eq_ignore_ascii_case("antigravity_o") {
        metrics.antigravity_o
    } else if key.eq_ignore_ascii_case("agy_external_o") {
        metrics.agy_external_o
    } else {
        let index = DISPLAY_PROVIDER_KEYS
            .iter()
            .position(|candidate| key.eq_ignore_ascii_case(candidate))?;
        metrics.providers[index]
    };
    if prefix.eq_ignore_ascii_case("display.text_box") {
        Some(provider.text_box)
    } else if prefix.eq_ignore_ascii_case("display.percent_box") {
        Some(provider.percent_box)
    } else if prefix.eq_ignore_ascii_case("display.marker_box") {
        Some(provider.marker_box)
    } else if prefix.eq_ignore_ascii_case("display.origin") {
        Some(provider.origin)
    } else if prefix.eq_ignore_ascii_case("display.text_slide") {
        Some(provider.text_slide)
    } else if prefix.eq_ignore_ascii_case("display.row_width") {
        Some(provider.row_width)
    } else {
        None
    }
}

/// Insert every binding derived from the animation state. Called once per
/// render, so the theme never duplicates the formulas.
pub(super) fn insert_animation_bindings(
    context: &mut DataContext,
    runtime: &ThemeRuntime,
    widgets: usize,
) {
    let phase = runtime.mode_phase.clamp(0.0, 2.0);
    let phases = decompose_mode_phase(phase);
    context.insert("display.view_mode", f64::from(runtime.view_mode));
    context.insert("display.mode_phase", phase);
    context.insert("display.mode_text_progress", phases.text_collapse);
    context.insert("display.mode_bar_progress", phases.bar_collapse);

    for (index, key) in widget_keys().enumerate() {
        let phase = runtime.widget_mode_phase(index);
        let phases = decompose_mode_phase(phase);
        context.insert(&format!("display.mode_phase.{key}"), phase);
        context.insert(
            &format!("display.mode_text_progress.{key}"),
            phases.text_collapse,
        );
        context.insert(
            &format!("display.mode_bar_progress.{key}"),
            phases.bar_collapse,
        );
    }

    let bar = compact_bar_full_width(widgets);
    context.insert("display.widget_count", widgets as f64);
    context.insert("display.bar_full_width", bar);
    let narrow = mode_row_bindings(
        phase,
        bar,
        COMPACT_TEXT_WIDTH,
        COMPACT_MARKER_WIDTH,
        COMPACT_ROW_GAP,
    );
    let wide = mode_row_bindings(
        phase,
        bar,
        COMPACT_TEXT_WIDTH_WIDE,
        COMPACT_MARKER_WIDTH,
        COMPACT_ROW_GAP,
    );
    context.insert("display.bar_width", narrow.bar_width);
    context.insert("display.bar_x", narrow.bar_x);
    context.insert("display.text_width", narrow.text_width);
    context.insert("display.text_slide_x", narrow.text_slide_x);
    context.insert("display.text_opacity", narrow.text_opacity);
    context.insert("display.percent_opacity", narrow.percent_opacity);
    context.insert("display.marker_opacity", narrow.marker_opacity);
    context.insert("display.provider_width", narrow.provider_width);
    // Rows carrying a window label reserve the wider text block.
    context.insert("display.text_width_wide", wide.text_width);
    context.insert("display.text_slide_x_wide", wide.text_slide_x);
    context.insert("display.provider_width_wide", wide.provider_width);
    // Per-widget row bindings follow each widget's own mode phase.
    for (index, key) in widget_keys().enumerate() {
        let row = mode_row_bindings(
            runtime.widget_mode_phase(index),
            bar,
            COMPACT_TEXT_WIDTH,
            COMPACT_MARKER_WIDTH,
            COMPACT_ROW_GAP,
        );
        context.insert(&format!("display.bar_width.{key}"), row.bar_width);
        context.insert(&format!("display.text_opacity.{key}"), row.text_opacity);
        context.insert(
            &format!("display.percent_opacity.{key}"),
            row.percent_opacity,
        );
    }

    // Hover progress per provider. Antigravity's crossfade runs in the
    // opposite direction (1.0 = normal, 0.0 = hover), so invert it here to
    // give every provider the same 0..1 hover phase.
    let clamp = |value: f64| value.clamp(0.0, 1.0);
    context.insert(
        "claude.hover_phase",
        clamp(runtime.claude_crossfade_opacity),
    );
    context.insert("codex.hover_phase", clamp(runtime.codex_crossfade_opacity));
    context.insert(
        "antigravity.hover_phase",
        1.0 - clamp(runtime.crossfade_opacity),
    );
    context.insert(
        "opencode.hover_phase",
        clamp(runtime.opencode_crossfade_opacity),
    );
    context.insert("qoder.hover_phase", clamp(runtime.qoder_crossfade_opacity));
    context.insert(
        "agy_external.hover_phase",
        clamp(runtime.agy_external_crossfade_opacity),
    );

    // Adaptive Compact columns. Legacy reserves are the base every context
    // sees; theme-aware resolvers overwrite them with measured values.
    let metrics = DisplayMetrics::legacy_with_widgets(*runtime, widgets);
    insert_display_metrics(context, &metrics);
}
