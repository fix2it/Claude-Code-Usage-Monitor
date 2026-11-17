use super::*;

pub(super) const FIVE_HOURS: std::time::Duration = std::time::Duration::from_secs(5 * 3600);
pub(super) const WEEK: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 3600);
/// Fallback length for the 30-day billing window when a cached reading has
/// no period start. A live reading uses the real `startsAt..endsAt` length.
pub(super) const MONTH: std::time::Duration = std::time::Duration::from_secs(30 * 24 * 3600);

/// Percent of a window of `length` that has passed by now, given when it
/// resets; `None` when the reset is not within one window from now.
/// Long name for a one-letter pool marker (Qoder `u`/`c`), for themes with
/// room to spell it out; any other label is returned as it is.
pub(super) fn pool_name(label: &str) -> String {
    match label {
        "u" => "Ultima".into(),
        "c" => "Credits".into(),
        other => other.into(),
    }
}

pub(super) fn expected_used(
    resets_at: std::time::SystemTime,
    length: std::time::Duration,
) -> Option<f64> {
    let left = resets_at
        .duration_since(std::time::SystemTime::now())
        .ok()?;
    // A minute of slack for poll latency right after a reset.
    (!length.is_zero() && left <= length + std::time::Duration::from_secs(60))
        .then(|| (1.0 - left.min(length).as_secs_f64() / length.as_secs_f64()) * 100.0)
}

/// Computes asymmetric crossfade opacities `(normal_opacity, hover_opacity)`
/// for hover transitions.
///
/// Progress `p` runs from 0.0 (unhovered/normal state) to 1.0 (hovered state).
/// - The outgoing normal layer fades out over the first 45%:
///   `normal_opacity = (1.0 - p / 0.45).clamp(0.0, 1.0)`.
/// - The incoming hover layer starts at 10% and reaches full opacity at 65%:
///   `hover_opacity = ((p - 0.10) / 0.55).clamp(0.0, 1.0)`.
///
/// The original ramp (0.35 out / 0.25 in) left the middle of the transition
/// almost blank: combined opacity bottomed out near 0.21 and the incoming
/// text was only at ~33% at `p=0.5`. Labels stay at fixed coordinates and
/// substitute in place, so a wider overlap only blends the same text box and
/// does not produce a positional ghost. Combined opacity now stays above
/// ~0.63, and the two layers are still never both fully opaque (their sum
/// never exceeds 1.0).
pub fn asymmetric_crossfade_opacities(progress_to_hover: f64) -> (f64, f64) {
    let p = progress_to_hover.clamp(0.0, 1.0);
    let normal_opacity = (1.0 - p / 0.45).clamp(0.0, 1.0);
    let hover_opacity = ((p - 0.10) / 0.55).clamp(0.0, 1.0);
    (normal_opacity, hover_opacity)
}

/// Dimensionless decomposition of the continuous mode phase.
/// `text_collapse` drives the `1 -> 2` transition (the numeric text leaves),
/// `bar_collapse` drives `2 -> 3` (the bar shuts towards the centre and the
/// percent badge appears). Pure so the choreography is unit-testable without
/// Win32 or a theme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModePhases {
    pub text_collapse: f64,
    pub bar_collapse: f64,
}

pub fn decompose_mode_phase(phase: f64) -> ModePhases {
    let p = phase.clamp(0.0, 2.0);
    ModePhases {
        text_collapse: p.min(1.0),
        bar_collapse: (p - 1.0).clamp(0.0, 1.0),
    }
}

/// Row geometry mirrored from the Compact theme. Stage 2 replaces the theme's
/// own expressions with the `display.*` bindings below, which makes these the
/// single source of truth; until then the bindings are simply unused.
pub(super) const COMPACT_BAR_SEGMENTS: f64 = 10.0;
/// A bar never shows more cubes than this, whatever the widget count.
pub(super) const COMPACT_MAX_CUBES: f64 = 5.0;
pub(super) const COMPACT_TEXT_WIDTH: f64 = 62.0;
pub(super) const COMPACT_TEXT_WIDTH_WIDE: f64 = 90.0;
pub(super) const COMPACT_MARKER_WIDTH: f64 = 10.0;
pub(super) const COMPACT_ROW_GAP: f64 = 4.0;

/// Cubes per bar: `min(5, ceil(10 / max(1, widgets)))`. The theme's
/// `segments_expression` uses the same formula over `display.widget_count`.
pub(super) fn compact_bar_cubes(widget_count: usize) -> f64 {
    (COMPACT_BAR_SEGMENTS / widget_count.max(1) as f64)
        .ceil()
        .min(COMPACT_MAX_CUBES)
}

/// `widget_count` is providers plus one when the Fable column is shown.
pub(super) fn compact_bar_full_width(widget_count: usize) -> f64 {
    compact_bar_cubes(widget_count) * 11.0 - 1.0
}

/// Count enabled providers and the separate two-pool Fable column.
pub(crate) fn widget_count(data: Option<&AppUsageData>, runtime: ThemeRuntime) -> usize {
    runtime.provider_count()
        + (fable_visible(data, runtime) && claude_dual(runtime)) as usize
        + antigravity_split(data, runtime) as usize
        + pool_separate(runtime, ProviderId::Claude) as usize
        + pool_separate(runtime, ProviderId::Codex) as usize
        + agy_separate(data, runtime) as usize
}

/// Which providers' second pools the active surface can show as their own
/// tiles. A provider without its split objects keeps one widget (F68).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoolSplitSupport {
    pub claude: bool,
    pub codex: bool,
    /// Needs both Antigravity tiles: one setting splits pools O and external O.
    pub antigravity: bool,
}

impl PoolSplitSupport {
    pub const ALL: Self = Self::uniform(true);
    pub const NONE: Self = Self::uniform(false);

    pub const fn uniform(supported: bool) -> Self {
        Self {
            claude: supported,
            codex: supported,
            antigravity: supported,
        }
    }

    /// From a predicate telling whether a split tile's object exists.
    pub(crate) fn from_pools(has: impl Fn(SplitPool) -> bool) -> Self {
        Self {
            claude: has(SplitPool::ClaudeX),
            codex: has(SplitPool::CodexX),
            antigravity: has(SplitPool::AntigravityO) && has(SplitPool::AgyExternalO),
        }
    }

    pub const fn all(self) -> bool {
        self.claude && self.codex && self.antigravity
    }

    /// Both sides must allow a provider's split.
    pub const fn and(self, other: Self) -> Self {
        Self {
            claude: self.claude && other.claude,
            codex: self.codex && other.codex,
            antigravity: self.antigravity && other.antigravity,
        }
    }
}

/// A second pool that gets its own tile while its provider's pools are split
/// ("Pools on one widget" off). The table drives the tile's theme object, its
/// binding namespace, its hover phase and its hit-test rect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SplitPool {
    ClaudeX,
    AntigravityO,
    AgyExternalO,
    CodexX,
}

impl SplitPool {
    /// In `split_hover_phases` order.
    pub(crate) const ALL: [SplitPool; 4] = [
        SplitPool::ClaudeX,
        SplitPool::AntigravityO,
        SplitPool::AgyExternalO,
        SplitPool::CodexX,
    ];

    pub(crate) const fn index(self) -> usize {
        self as usize
    }

    /// Namespace of the first pool, which the second pool's namespace copies
    /// while the pools share one widget.
    pub(super) const fn base_key(self) -> &'static str {
        match self {
            SplitPool::ClaudeX => "claude",
            SplitPool::AntigravityO => "antigravity",
            SplitPool::AgyExternalO => "agy_external",
            SplitPool::CodexX => "codex",
        }
    }

    /// Binding namespace of the tile (`claude_x.session.display`, ...).
    pub(crate) const fn key(self) -> &'static str {
        match self {
            SplitPool::ClaudeX => "claude_x",
            SplitPool::AntigravityO => "antigravity_o",
            SplitPool::AgyExternalO => "agy_external_o",
            SplitPool::CodexX => "codex_x",
        }
    }

    /// Theme object of the tile: its hover and hit-test area.
    pub(crate) const fn object_id(self) -> &'static str {
        match self {
            SplitPool::ClaudeX => "claude_x-provider",
            SplitPool::AntigravityO => "antigravity_o-provider",
            SplitPool::AgyExternalO => "agy_external_o-provider",
            SplitPool::CodexX => "codex_x-provider",
        }
    }

    pub(super) fn hover_phase(self, runtime: ThemeRuntime) -> f64 {
        runtime.split_hover_phases[self.index()].clamp(0.0, 1.0)
    }
}

/// Both Claude (or both Codex) pools are on and shown as separate widgets.
pub(super) fn pool_separate(runtime: ThemeRuntime, provider: ProviderId) -> bool {
    let (main, x, one_widget) = match provider {
        ProviderId::Claude => (
            runtime.claude_main_enabled,
            runtime.claude_x_enabled,
            runtime.claude_pools_one_widget,
        ),
        ProviderId::Codex => (
            runtime.codex_main_enabled,
            runtime.codex_x_enabled,
            runtime.codex_pools_one_widget,
        ),
        _ => return false,
    };
    runtime.provider_enabled(provider) && main && x && !one_widget
}

pub(super) fn agy_separate(data: Option<&AppUsageData>, runtime: ThemeRuntime) -> bool {
    antigravity_pool_o(data, runtime) && !runtime.antigravity_pools_one_widget
}

/// External models on with pool O shown: the external models get their own
/// widget next to the Gemini one, each with a row per pool (A and O).
pub(crate) fn antigravity_split(data: Option<&AppUsageData>, runtime: ThemeRuntime) -> bool {
    antigravity_pool_o(data, runtime)
        && runtime.antigravity_pools_one_widget
        && (runtime.antigravity_external_enabled || runtime.antigravity_external_x_enabled)
}

/// The OpenCode agy pool has its own row: it is switched on (with agy) and is
/// not the same login as pool A. Until the first poll the row is kept, so the
/// widget does not jump; a known duplicate hides it.
pub(crate) fn antigravity_pool_o(data: Option<&AppUsageData>, runtime: ThemeRuntime) -> bool {
    runtime.provider_enabled(ProviderId::Antigravity)
        && runtime.antigravity_gemini_enabled
        && runtime.antigravity_x_enabled
        && runtime.antigravity_a_enabled
        && !data
            .and_then(AppUsageData::antigravity_x)
            .is_some_and(|account| account.error == Some(crate::poller::PollError::NoCredentials))
}

/// Both Claude pools are on: the Fable column then shows one row per pool.
pub(super) fn claude_dual(runtime: ThemeRuntime) -> bool {
    runtime.claude_main_enabled && runtime.claude_x_enabled && runtime.claude_pools_one_widget
}

/// Whether the Fable column is drawn.
pub(crate) fn fable_visible(_data: Option<&AppUsageData>, runtime: ThemeRuntime) -> bool {
    let (main, x) = fable_pools(runtime);
    main || x
}

/// Per-pool Fable rows: each pool's own toggle, while Claude and that pool are on.
pub(super) fn fable_pools(runtime: ThemeRuntime) -> (bool, bool) {
    let claude = runtime.provider_enabled(ProviderId::Claude);
    (
        claude && runtime.show_fable && runtime.claude_main_enabled,
        claude && runtime.show_fable_x && runtime.claude_x_enabled,
    )
}

/// Geometry and visibility for one widget row at a given mode phase. Pure:
/// the caller supplies the row's own widths, so no theme constants are baked
/// into the formulas themselves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModeBindings {
    pub bar_width: f64,
    pub bar_x: f64,
    pub text_width: f64,
    pub text_slide_x: f64,
    pub text_opacity: f64,
    pub percent_opacity: f64,
    pub marker_opacity: f64,
    pub provider_width: f64,
}

pub fn mode_row_bindings(
    phase: f64,
    bar_full_width: f64,
    text_full_width: f64,
    marker_width: f64,
    gap: f64,
) -> ModeBindings {
    let phases = decompose_mode_phase(phase);
    let bar_width = bar_full_width * (1.0 - phases.bar_collapse);
    let text_width = text_full_width * (1.0 - phases.text_collapse);
    ModeBindings {
        bar_width,
        bar_x: bar_full_width * phases.bar_collapse / 2.0,
        text_width,
        text_slide_x: -text_full_width * phases.text_collapse,
        text_opacity: 100.0 * (1.0 - phases.text_collapse),
        percent_opacity: 100.0 * phases.bar_collapse,
        // Markers stay at their fixed positions in every mode.
        marker_opacity: 100.0,
        provider_width: bar_width + text_width + gap + marker_width,
    }
}

/// Provider keys that carry an adaptive Compact column, in the same order the
/// bindings are read by the theme.
pub const DISPLAY_PROVIDER_KEYS: [&str; 8] = [
    "cursor",
    "claude",
    "codex",
    "antigravity",
    "opencode",
    "qoder",
    "grok",
    "copilot",
];

pub(super) fn widget_mode_index(key: &str) -> Option<usize> {
    widget_keys().position(|candidate| candidate == key)
}

/// Every widget key in slot order: the providers, the Fable column, then the
/// agy external-models widget.
pub(crate) fn widget_keys() -> impl Iterator<Item = &'static str> {
    DISPLAY_PROVIDER_KEYS.into_iter().chain([
        FABLE_KEY,
        AGY_EXTERNAL_KEY,
        "claude_x",
        "antigravity_o",
        "agy_external_o",
        "codex_x",
    ])
}
