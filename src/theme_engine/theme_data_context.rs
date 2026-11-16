use super::*;
use crate::models::{UsageData, UsageSection};
use std::time::SystemTime;

impl DataContext {
    /// Evaluate a surface render expression only when it does not require its
    /// resolved size. Unknown values leave the decision to the normal renderer.
    pub fn surface_render_without_size(
        expression: &Expression,
        data: Option<&AppUsageData>,
        canvas: &Canvas,
        runtime: ThemeRuntime,
    ) -> Option<bool> {
        let mut context = Self::from_usage_with_runtime(data, canvas, runtime);
        context.display_metrics = None;
        for name in [
            "canvas.width",
            "canvas.height",
            "parent.width",
            "parent.height",
        ] {
            Arc::make_mut(&mut context.values).remove(name);
            context.geometry_values.remove(name);
        }
        evaluate(&expression.0, &context)
            .ok()
            .map(|value| value.is_finite() && value != 0.0)
    }

    pub fn from_usage(data: Option<&AppUsageData>, canvas: &Canvas) -> Self {
        Self::from_usage_with_runtime(data, canvas, ThemeRuntime::default())
    }

    pub fn from_usage_with_runtime(
        data: Option<&AppUsageData>,
        canvas: &Canvas,
        runtime: ThemeRuntime,
    ) -> Self {
        let mut context = Self {
            force_segoe_ui: !runtime.use_windows_default_font,
            ..Self::default()
        };
        context.insert_environment(canvas, runtime);
        context.insert_i18n(runtime);
        context.insert_clock();
        context.insert_provider_flags(data, runtime);
        context.insert_display_flags(data, runtime);
        insert_animation_bindings(&mut context, &runtime, widget_count(data, runtime));
        let pool_o = context.insert_antigravity_display(data, runtime);
        context.insert_hover_states(data, runtime);
        context.insert_usage(data, runtime, pool_o);
        context.insert_split_pools(data, runtime);
        context
    }

    /// Canvas size, math constants, app version and poll status.
    fn insert_environment(&mut self, canvas: &Canvas, runtime: ThemeRuntime) {
        self.insert("canvas.width", canvas.width as f64);
        self.insert("canvas.height", canvas.height as f64);
        self.insert("parent.width", canvas.width as f64);
        self.insert("parent.height", canvas.height as f64);
        self.insert("host.width", runtime.host_width as f64);
        self.insert("host.height", runtime.host_height as f64);
        self.insert("pi", std::f64::consts::PI);
        self.insert("e", std::f64::consts::E);
        self.insert("true", 1.0);
        self.insert("false", 0.0);
        self.insert_string("app.version", env!("CARGO_PKG_VERSION"));
        let mut version_parts = env!("CARGO_PKG_VERSION")
            .split(['.', '-', '+'])
            .filter_map(|part| part.parse::<f64>().ok());
        self.insert("app.version.major", version_parts.next().unwrap_or(0.0));
        self.insert("app.version.minor", version_parts.next().unwrap_or(0.0));
        self.insert("app.version.patch", version_parts.next().unwrap_or(0.0));
        self.insert(
            "system.dark",
            crate::theme::is_dark_mode_cached() as u8 as f64,
        );
        self.insert("data.poll_ok", runtime.poll_ok as u8 as f64);
        self.insert("data.has_error", runtime.has_error as u8 as f64);
        self.insert(
            "data.loading",
            (!runtime.poll_ok && !runtime.has_error) as u8 as f64,
        );
    }

    fn insert_i18n(&mut self, runtime: ThemeRuntime) {
        let strings = runtime.language.strings();
        self.insert_string("i18n.locale", runtime.language.code());
        self.insert_string("i18n.session_window", strings.session_window);
        self.insert_string("i18n.weekly_window", strings.weekly_window);
        self.insert_string("i18n.cursor_auto_window", strings.cursor_auto_window);
        self.insert_string("i18n.cursor_api_window", strings.cursor_api_window);
        self.insert_string("i18n.now", strings.now);
        self.insert_string("i18n.day_suffix", strings.day_suffix);
        self.insert_string("i18n.hour_suffix", strings.hour_suffix);
        self.insert_string("i18n.minute_suffix", strings.minute_suffix);
        self.insert_string("i18n.second_suffix", strings.second_suffix);
    }

    /// Current time as unix seconds plus local and UTC calendar parts.
    fn insert_clock(&mut self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_secs_f64())
            .unwrap_or(0.0);
        self.insert("time.now.unix", now);
        self.insert("time.now.milliseconds", now * 1000.0);
        for (zone, local) in [("local", true), ("utc", false)] {
            if let Some(value) = timestamp_parts(now, local) {
                for (component, value) in [
                    ("year", value.year),
                    ("month", value.month),
                    ("day", value.day),
                    ("weekday", value.weekday),
                    ("hour", value.hour),
                    ("minute", value.minute),
                    ("second", value.second),
                ] {
                    self.insert(&format!("time.{zone}.{component}"), f64::from(value));
                }
            }
        }
    }

    fn insert_provider_flags(&mut self, data: Option<&AppUsageData>, runtime: ThemeRuntime) {
        self.insert(
            "providers.count",
            (runtime.provider_count()
                + pool_separate(runtime, ProviderId::Claude) as usize
                + pool_separate(runtime, ProviderId::Codex) as usize
                + agy_separate(data, runtime) as usize) as f64,
        );
        for descriptor in PROVIDER_DESCRIPTORS {
            self.insert_string(&format!("{}.account.name", descriptor.key), "");
            self.insert(
                &format!("providers.{}.enabled", descriptor.key),
                runtime.provider_enabled(descriptor.id) as u8 as f64,
            );
        }
    }

    /// Display settings and the Fable / pool-split row switches.
    fn insert_display_flags(&mut self, data: Option<&AppUsageData>, runtime: ThemeRuntime) {
        self.insert("display.countdown", runtime.countdown as u8 as f64);
        self.insert("display.show_pace", runtime.show_pace as u8 as f64);
        self.insert(
            "display.pool_split_supported",
            runtime.pool_split_supported as u8 as f64,
        );
        self.insert("display.show_fable", runtime.show_fable as u8 as f64);
        let (fable_main, fable_x) = fable_pools(runtime);
        self.insert("display.fable_main", fable_main as u8 as f64);
        self.insert("display.fable_x", fable_x as u8 as f64);
        self.insert(
            "display.fable_visible",
            fable_visible(data, runtime) as u8 as f64,
        );
        self.insert(
            "display.fable_separate",
            (fable_visible(data, runtime) && claude_dual(runtime)) as u8 as f64,
        );
        self.insert(
            "display.fable_row",
            (if pool_separate(runtime, ProviderId::Claude) {
                runtime.show_fable
            } else {
                fable_visible(data, runtime) && !claude_dual(runtime)
            }) as u8 as f64,
        );
        self.insert(
            "display.fable_x_row",
            (pool_separate(runtime, ProviderId::Claude) && runtime.show_fable_x) as u8 as f64,
        );
        self.insert(
            "display.claude_split",
            pool_separate(runtime, ProviderId::Claude) as u8 as f64,
        );
        self.insert(
            "display.codex_split",
            pool_separate(runtime, ProviderId::Codex) as u8 as f64,
        );
        self.insert(
            "display.agy_pool_split",
            agy_separate(data, runtime) as u8 as f64,
        );
    }

    /// Antigravity pool switches and crossfade opacities; returns whether
    /// pool O shares the Antigravity widget.
    fn insert_antigravity_display(
        &mut self,
        data: Option<&AppUsageData>,
        runtime: ThemeRuntime,
    ) -> bool {
        self.insert(
            "antigravity.gemini.enabled",
            runtime.antigravity_gemini_enabled as u8 as f64,
        );
        self.insert(
            "antigravity.external.enabled",
            runtime.antigravity_external_enabled as u8 as f64,
        );
        let pool_o = antigravity_pool_o(data, runtime) && !agy_separate(data, runtime);
        self.insert(
            "antigravity.x.enabled",
            runtime.antigravity_x_enabled as u8 as f64,
        );
        self.insert("antigravity.pool_o", pool_o as u8 as f64);
        self.insert(
            "antigravity.a.enabled",
            runtime.antigravity_a_enabled as u8 as f64,
        );
        self.insert(
            "display.agy_split",
            antigravity_split(data, runtime) as u8 as f64,
        );
        self.insert("display.agy_o_external", 0.0);
        // Two rows: Gemini and external models (pool A), or pools A and O.
        let agy_dual_mode =
            (runtime.antigravity_gemini_enabled && runtime.antigravity_external_enabled) || pool_o;
        self.insert("antigravity.dual_mode", agy_dual_mode as u8 as f64);
        let crossfade = runtime.crossfade_opacity.clamp(0.0, 1.0);
        self.insert("antigravity.crossfade_opacity", crossfade);
        let (agy_normal_opacity, agy_hover_opacity) =
            asymmetric_crossfade_opacities(1.0 - crossfade);
        self.insert("antigravity.normal.opacity", agy_normal_opacity);
        self.insert("antigravity.hover.opacity", agy_hover_opacity);
        // crossfade 1.0 shows the primary pool: Gemini in dual mode, otherwise
        // the single enabled pool; hover fades toward the other one.
        let (gemini_opacity, external_opacity) = match (
            runtime.antigravity_gemini_enabled,
            runtime.antigravity_external_enabled,
        ) {
            (false, true) => (agy_hover_opacity, agy_normal_opacity),
            (false, false) => (0.0, 0.0),
            _ => (agy_normal_opacity, agy_hover_opacity),
        };
        let only_gemini_a = agy_separate(data, runtime) && !runtime.antigravity_external_enabled;
        self.insert(
            "antigravity.gemini.opacity",
            if only_gemini_a { 1.0 } else { gemini_opacity },
        );
        self.insert(
            "antigravity.external.opacity",
            if only_gemini_a { 0.0 } else { external_opacity },
        );
        pool_o
    }

    /// Hover state and crossfades of the dual-pool provider widgets.
    fn insert_hover_states(&mut self, data: Option<&AppUsageData>, runtime: ThemeRuntime) {
        self.insert("claude.hovered", runtime.claude_hovered as u8 as f64);
        self.insert_hover_crossfade("claude", runtime.claude_crossfade_opacity);
        self.insert(
            "claude.main.enabled",
            runtime.claude_main_enabled as u8 as f64,
        );
        self.insert("claude.x.enabled", runtime.claude_x_enabled as u8 as f64);
        self.insert("claude.dual_mode", claude_dual(runtime) as u8 as f64);
        self.insert("codex.hovered", runtime.codex_hovered as u8 as f64);
        self.insert_hover_crossfade("codex", runtime.codex_crossfade_opacity);
        self.insert(
            "codex.main.enabled",
            runtime.codex_main_enabled as u8 as f64,
        );
        self.insert("codex.x.enabled", runtime.codex_x_enabled as u8 as f64);
        self.insert(
            "codex.dual_mode",
            (runtime.codex_main_enabled
                && runtime.codex_x_enabled
                && runtime.codex_pools_one_widget) as u8 as f64,
        );
        self.insert_hover_crossfade("opencode", runtime.opencode_crossfade_opacity);
        // Qoder's hover only swaps in a second pool. If a poll removes that
        // pool mid-hover, rest at phase 0 so the single bottom row stays drawn.
        let qoder_phase = if data
            .and_then(|data| data.get(ProviderId::Qoder))
            .is_some_and(|usage| usage.week.is_some())
        {
            runtime.qoder_crossfade_opacity.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.insert("qoder.hover_phase", qoder_phase);
        self.insert_hover_crossfade("qoder", qoder_phase);
        self.insert_hover_crossfade(AGY_EXTERNAL_KEY, runtime.agy_external_crossfade_opacity);
    }

    /// Per-account, per-provider, Antigravity pool and `active` readings.
    /// Before the first poll every reading binding exists but is empty.
    fn insert_usage(&mut self, data: Option<&AppUsageData>, runtime: ThemeRuntime, pool_o: bool) {
        if let Some(data) = data {
            self.insert_accounts(data, runtime.countdown);
        }
        for descriptor in PROVIDER_DESCRIPTORS {
            if let Some(data) = data {
                if let Some(account) = data
                    .accounts
                    .iter()
                    .find(|account| account.provider == descriptor.id && account.selected)
                {
                    self.insert(
                        &format!("{}.has_error", descriptor.key),
                        account.error.is_some() as u8 as f64,
                    );
                }
                self.insert_string(
                    &format!("{}.account.name", descriptor.key),
                    data.selected_account_name(descriptor.id).unwrap_or(""),
                );
            }
            self.insert_provider(
                descriptor.key,
                data.and_then(|data| data.get(descriptor.id)),
                descriptor.id == ProviderId::Codex,
                runtime.countdown,
            );
        }
        let pool_o_usage = data
            .and_then(|data| data.antigravity_x())
            .and_then(|account| account.usage.as_ref());
        // With pool A off, pool O fills the single-pool layout alone.
        let pool_a = if runtime.antigravity_o_only() {
            pool_o_usage
        } else {
            // Never pool O's stand-in reading under pool A's label.
            data.and_then(|data| data.antigravity_pool_a_reading())
        };
        self.insert_antigravity_rows(pool_a, pool_o_usage, pool_o, runtime);
        if pool_a.is_some() {
            // Antigravity answered this poll; pool-level gaps are "no limit
            // data" ("--"), not a provider error.
            self.insert("antigravity.reported", 1.0);
        }
        self.insert_provider("antigravity.gemini", pool_a, false, runtime.countdown);
        // A missing external pool stays empty ("--"); never show Gemini figures in its place.
        self.insert_provider(
            "antigravity.external",
            pool_a.and_then(|agy| agy.external_pool.as_deref()),
            false,
            runtime.countdown,
        );
        let active = ProviderId::ALL.into_iter().find_map(|provider| {
            data.and_then(|data| data.get(provider))
                .map(|usage| (provider, usage))
        });
        self.insert_provider(
            "active",
            active.map(|(_, usage)| usage),
            active.is_some_and(|(provider, _)| provider == ProviderId::Codex),
            runtime.countdown,
        );
    }

    fn insert_accounts(&mut self, data: &AppUsageData, countdown: bool) {
        for account in &data.accounts {
            let key = format!(
                "accounts.{}.{}",
                account.provider.descriptor().key,
                account.profile.id
            );
            self.insert_string(&format!("{key}.name"), &account.profile.name);
            self.insert(&format!("{key}.selected"), account.selected as u8 as f64);
            self.insert(
                &format!("{key}.has_error"),
                account.error.is_some() as u8 as f64,
            );
            self.insert_provider(
                &key,
                account.usage.as_ref(),
                account.provider == ProviderId::Codex,
                countdown,
            );
        }
        self.insert_missing_pool_accounts(data, countdown);
    }

    /// Theme validation evaluates every expression even when its parent is
    /// hidden, so the split namespaces must exist in single-pool contexts.
    fn insert_split_pools(&mut self, data: Option<&AppUsageData>, runtime: ThemeRuntime) {
        for pool in SplitPool::ALL {
            self.copy_prefix(
                &format!("{}.", pool.base_key()),
                &format!("{}.", pool.key()),
            );
        }
        if pool_separate(runtime, ProviderId::Claude) {
            // Claude rebinds even before the first poll (the tiles then read
            // nothing); Codex keeps its empty provider bindings until data.
            self.bind_account_pools(SplitPool::ClaudeX, &[], true, runtime);
        }
        if pool_separate(runtime, ProviderId::Codex) {
            self.bind_account_pools(SplitPool::CodexX, &["credits."], data.is_some(), runtime);
        }
        if agy_separate(data, runtime) {
            self.insert_antigravity_pool_o(data, runtime);
        }
    }

    /// Account bindings for pools that have no account entry yet: the
    /// default pools fall back to the provider reading, a second pool that
    /// has not been polled shows "--" (the provider reading is the first
    /// pool's).
    fn insert_missing_pool_accounts(&mut self, data: &AppUsageData, countdown: bool) {
        let has_account = |provider: ProviderId, id: &str| {
            data.accounts
                .iter()
                .any(|a| a.provider == provider && a.profile.id == id)
        };
        if !has_account(ProviderId::Claude, "default") {
            // Without any Claude account the legacy entry is the default
            // profile. Otherwise it may belong to another profile, so the
            // fallback keeps its windows but never borrows its limits.
            let has_claude_account = data
                .accounts
                .iter()
                .any(|a| a.provider == ProviderId::Claude);
            let claude_usage = data.get(ProviderId::Claude).map(|usage| {
                let mut usage = usage.clone();
                if has_claude_account {
                    usage.limits.clear();
                }
                usage
            });
            self.insert_provider(
                "accounts.claude.default",
                claude_usage.as_ref(),
                false,
                countdown,
            );
        }
        if !has_account(ProviderId::Claude, "claude_x") {
            self.insert_provider("accounts.claude.claude_x", None, false, countdown);
        }
        if !has_account(ProviderId::Codex, "default") {
            let codex_usage = data.get(ProviderId::Codex);
            self.insert_provider("accounts.codex.default", codex_usage, true, countdown);
        }
        if !has_account(ProviderId::Codex, "codex_x") {
            self.insert_provider("accounts.codex.codex_x", None, true, countdown);
        }
    }

    /// Split Claude or Codex: the main tile reads the default account and the
    /// X tile its own account, replacing the provider-level quota bindings.
    fn bind_account_pools(
        &mut self,
        pool: SplitPool,
        extra_fields: &[&str],
        rebind: bool,
        runtime: ThemeRuntime,
    ) {
        const QUOTA_FIELDS: [&str; 6] = [
            "session.",
            "five_hour.",
            "weekly.",
            "model.",
            "limits.",
            "scoped.",
        ];
        let (main, x) = (pool.base_key(), pool.key());
        if rebind {
            let prefixes: Vec<String> = [main, x]
                .iter()
                .flat_map(|owner| {
                    QUOTA_FIELDS
                        .iter()
                        .chain(extra_fields)
                        .map(move |field| format!("{owner}.{field}"))
                })
                .collect();
            let replaced = |key: &String| prefixes.iter().any(|prefix| key.starts_with(prefix));
            Arc::make_mut(&mut self.values).retain(|key, _| !replaced(key));
            Arc::make_mut(&mut self.strings).retain(|key, _| !replaced(key));
            self.copy_prefix(&format!("accounts.{main}.default."), &format!("{main}."));
            self.copy_prefix(&format!("accounts.{main}.{x}."), &format!("{x}."));
        }
        let phase = self.insert_split_hover(pool, runtime).0;
        self.insert(&format!("{x}.hovered"), (phase > 0.0) as u8 as f64);
    }

    /// `hover_phase` and the crossfade pair of a split tile. Returns the
    /// phase and the (normal, hover) opacities.
    fn insert_split_hover(&mut self, pool: SplitPool, runtime: ThemeRuntime) -> (f64, (f64, f64)) {
        let phase = pool.hover_phase(runtime);
        self.insert(&format!("{}.hover_phase", pool.key()), phase);
        (phase, self.insert_hover_crossfade(pool.key(), phase))
    }

    /// agy pool O in its own tile, built through the same provider insertion
    /// path as pool A, without rebuilding unrelated provider and animation
    /// bindings. The external models of pool O get their own tile too.
    fn insert_antigravity_pool_o(&mut self, data: Option<&AppUsageData>, runtime: ThemeRuntime) {
        let account = data.and_then(AppUsageData::antigravity_x);
        let pool = account.and_then(|account| account.usage.as_ref());
        let external = pool.and_then(|usage| usage.external_pool.as_deref());
        for (key, usage) in [
            ("antigravity_o", pool),
            ("antigravity_o.gemini", pool),
            ("antigravity_o.external", external),
            ("antigravity_o.row1h", pool),
            ("antigravity_o.row2", external),
            ("antigravity_o.row2h", external),
        ] {
            self.insert_provider(key, usage, false, runtime.countdown);
        }
        self.insert("antigravity_o.reported", pool.is_some() as u8 as f64);
        Arc::make_mut(&mut self.values).remove("antigravity_o.has_error");
        if let Some(account) = account {
            self.insert(
                "antigravity_o.has_error",
                account.error.is_some() as u8 as f64,
            );
        }
        let (_, (normal, hover)) = self.insert_split_hover(SplitPool::AntigravityO, runtime);
        let external_x = runtime.antigravity_external_x_enabled;
        self.insert("antigravity_o.external.enabled", external_x as u8 as f64);
        self.insert(
            "antigravity_o.gemini.opacity",
            if external_x { normal } else { 1.0 },
        );
        self.insert(
            "antigravity_o.external.opacity",
            if external_x { hover } else { 0.0 },
        );
        self.insert("antigravity_o.dual_mode", external_x as u8 as f64);
        self.insert(
            "antigravity.external_a.shown",
            runtime.antigravity_external_enabled as u8 as f64,
        );
        self.insert("antigravity.external_o.shown", 0.0);
        self.insert("antigravity_o.external_a.shown", external_x as u8 as f64);
        self.insert("antigravity_o.external_o.shown", 0.0);
        self.copy_prefix("agy_external.", "agy_external_o.");
        self.insert_split_hover(SplitPool::AgyExternalO, runtime);
    }

    fn copy_prefix(&mut self, source: &str, target: &str) {
        let values: Vec<_> = self
            .values
            .iter()
            .filter_map(|(key, value)| {
                key.strip_prefix(source)
                    .map(|suffix| (format!("{target}{suffix}"), *value))
            })
            .collect();
        let strings: Vec<_> = self
            .strings
            .iter()
            .filter_map(|(key, value)| {
                key.strip_prefix(source)
                    .map(|suffix| (format!("{target}{suffix}"), value.clone()))
            })
            .collect();
        Arc::make_mut(&mut self.values).extend(values);
        Arc::make_mut(&mut self.strings).extend(strings);
    }

    /// Sources of the agy two-row layout. Row 1 is always pool A's Gemini
    /// (`antigravity.gemini`). Without pool O row 2 is pool A's external
    /// models, exactly as before; with pool O it is pool O's Gemini, and the
    /// external models move to their own widget (`display.agy_split`).
    fn insert_antigravity_rows(
        &mut self,
        pool_a: Option<&crate::models::UsageData>,
        pool_o: Option<&crate::models::UsageData>,
        pool_o_on: bool,
        runtime: ThemeRuntime,
    ) {
        let a_external = pool_a.and_then(|usage| usage.external_pool.as_deref());
        let o_external = pool_o.and_then(|usage| usage.external_pool.as_deref());
        let row2 = if pool_o_on { pool_o } else { a_external };
        self.insert_provider("antigravity.row2", row2, false, runtime.countdown);
        self.insert_provider("antigravity.row1h", pool_a, false, runtime.countdown);
        self.insert_provider("antigravity.row2h", row2, false, runtime.countdown);
        // Pool O's row in the external-models widget. A missing or failed
        // reading leaves the row in place with no data ("--", empty gauge),
        // like any provider without data, so the widget is never empty.
        self.insert_provider(
            "antigravity.external_o",
            o_external,
            false,
            runtime.countdown,
        );
        // One external-models row per pool whose own toggle is on.
        let split = pool_o_on
            && (runtime.antigravity_external_enabled || runtime.antigravity_external_x_enabled);
        self.insert(
            "antigravity.external_a.shown",
            (split && runtime.antigravity_external_enabled) as u8 as f64,
        );
        self.insert(
            "antigravity.external_o.shown",
            (split && runtime.antigravity_external_x_enabled) as u8 as f64,
        );
    }

    pub fn insert_hover_crossfade(&mut self, prefix: &str, progress_to_hover: f64) -> (f64, f64) {
        let p = progress_to_hover.clamp(0.0, 1.0);
        self.insert(&format!("{prefix}.crossfade_opacity"), p);
        let (normal, hover) = asymmetric_crossfade_opacities(p);
        self.insert(&format!("{prefix}.normal.opacity"), normal);
        self.insert(&format!("{prefix}.hover.opacity"), hover);
        (normal, hover)
    }

    pub(super) fn insert_provider(
        &mut self,
        name: &str,
        usage: Option<&UsageData>,
        codex_compatibility: bool,
        countdown: bool,
    ) {
        self.insert_limits(name, usage, countdown);
        self.insert_window_labels(name, usage);
        // Before Codex windows were classified by duration, a weekly-only
        // response arrived in the primary slot and was exposed to themes as
        // `codex.session`. Keep that established binding working for existing
        // custom themes, while `codex.five_hour` always means the real window.
        let codex_session_fallback = codex_compatibility
            && usage.is_some_and(|usage| !usage.session.available && usage.weekly.available);
        // Each window reads the clock once; `session` reuses the reading of
        // the window it mirrors so their countdowns stay identical.
        let five_hour_now = SystemTime::now();
        let weekly_now = SystemTime::now();
        for (window, binding) in quota_windows(name, usage, codex_session_fallback) {
            let now = match window {
                "session" if codex_session_fallback => weekly_now,
                "session" | "five_hour" => five_hour_now,
                "weekly" => weekly_now,
                _ => SystemTime::now(),
            };
            self.insert_window(name, window, binding, countdown, now);
        }
        self.insert(&format!("{name}.available"), usage.is_some() as u8 as f64);
        // Carried over from an earlier poll: real figures, not current ones.
        self.insert(
            &format!("{name}.stale"),
            usage.is_some_and(|usage| usage.stale) as u8 as f64,
        );
        self.insert_credits_and_headline(name, usage, countdown);
    }

    fn insert_window_labels(&mut self, name: &str, usage: Option<&UsageData>) {
        let weekly_label = usage
            .and_then(|usage| usage.weekly_label.as_deref())
            .or_else(|| self.get_string("i18n.weekly_window"))
            .unwrap_or("7d")
            .to_string();
        self.insert_string(&format!("{name}.weekly.label"), weekly_label);
        let session_label = usage
            .and_then(|usage| usage.session_label.as_deref())
            .or_else(|| self.get_string("i18n.session_window"))
            .unwrap_or("5h")
            .to_string();
        self.insert_string(&format!("{name}.session.name"), pool_name(&session_label));
        self.insert_string(&format!("{name}.session.label"), session_label);
        let week_label = usage
            .and_then(|usage| usage.week_label.as_deref())
            .unwrap_or("7d")
            .to_string();
        self.insert_string(&format!("{name}.week.name"), pool_name(&week_label));
        self.insert_string(&format!("{name}.week.label"), week_label);
        self.insert_string(&format!("{name}.monthly.label"), "30d");
    }

    /// `percentage` stays the share that has been spent so warning thresholds
    /// keep working, while `display` follows the countdown setting.
    fn insert_window(
        &mut self,
        name: &str,
        window: &str,
        binding: WindowBinding,
        countdown: bool,
        now: SystemTime,
    ) {
        let percentage = binding.percentage;
        self.insert(
            &format!("{name}.{window}.available"),
            binding.available as u8 as f64,
        );
        self.insert(&format!("{name}.{window}.percentage"), percentage);
        self.insert(&format!("{name}.{window}.remaining"), 100.0 - percentage);
        let shown = if binding.empty_gauge {
            0.0
        } else {
            display_percentage(percentage, countdown)
        };
        self.insert(&format!("{name}.{window}.display"), shown);
        self.insert(
            &format!("{name}.{window}.expected_available"),
            binding.expected.is_some() as u8 as f64,
        );
        let expected = binding.expected.unwrap_or(0.0);
        self.insert(&format!("{name}.{window}.expected"), expected);
        self.insert(
            &format!("{name}.{window}.expected_display"),
            display_percentage(expected, countdown),
        );
        let unix = binding
            .resets_at
            .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|value| value.as_secs_f64())
            .unwrap_or(0.0);
        let seconds = binding
            .resets_at
            .and_then(|value| value.duration_since(now).ok())
            .map(|value| value.as_secs_f64())
            .unwrap_or(0.0);
        self.insert(&format!("{name}.{window}.reset.unix"), unix);
        self.insert(&format!("{name}.{window}.reset.seconds"), seconds);
        self.insert(&format!("{name}.{window}.reset.minutes"), seconds / 60.0);
        self.insert(&format!("{name}.{window}.reset.hours"), seconds / 3600.0);
        self.insert(&format!("{name}.{window}.reset.days"), seconds / 86400.0);
    }

    fn insert_credits_and_headline(
        &mut self,
        name: &str,
        usage: Option<&UsageData>,
        countdown: bool,
    ) {
        // Credits are absent for most accounts, so `credits.available` is what
        // a theme should gate the overlay on rather than `available`.
        let credits = usage.and_then(|usage| usage.credits.as_ref());
        let credits_percentage = credits.map(|credits| credits.percentage).unwrap_or(0.0);
        self.insert(&format!("{name}.credits.percentage"), credits_percentage);
        self.insert(
            &format!("{name}.credits.remaining"),
            100.0 - credits_percentage,
        );
        self.insert(
            &format!("{name}.credits.display"),
            display_percentage(credits_percentage, countdown),
        );
        // Currency, unlike the percentages either side of it.
        self.insert(
            &format!("{name}.credits.balance"),
            credits.map(|credits| credits.remaining).unwrap_or(0.0),
        );
        self.insert(
            &format!("{name}.credits.total"),
            credits.map(|credits| credits.total).unwrap_or(0.0),
        );
        self.insert(
            &format!("{name}.credits.available"),
            credits.is_some() as u8 as f64,
        );
        // The single figure a badge should show: whatever is closest to its
        // limit. A provider can switch a window off entirely -- Codex has its
        // five-hour window disabled -- so binding a badge to one window alone
        // leaves it reporting 0% while another allowance is spent.
        let headline = match credits {
            Some(credits) => credits.percentage,
            None => usage
                .map(|usage| usage.session.percentage.max(usage.weekly.percentage))
                .unwrap_or(0.0),
        };
        self.insert(&format!("{name}.headline.percentage"), headline);
        self.insert(&format!("{name}.headline.remaining"), 100.0 - headline);
        self.insert(
            &format!("{name}.headline.display"),
            display_percentage(headline, countdown),
        );
    }

    pub fn insert(&mut self, name: &str, value: f64) {
        let key = name.to_ascii_lowercase();
        if key.starts_with("parent.") || key.starts_with("this.") || key.starts_with("object.") {
            self.geometry_values.insert(key, value);
        } else {
            Arc::make_mut(&mut self.values).insert(key, value);
        }
    }

    pub fn get(&self, name: &str) -> Option<f64> {
        if let Some(value) = self.geometry_values.get(name).copied() {
            return Some(value);
        }
        if let Some(value) = self.values.get(name).copied() {
            return Some(value);
        }
        if let Some(value) = self
            .display_metrics
            .as_ref()
            .and_then(|metrics| display_metric(metrics, name))
        {
            return Some(value);
        }
        let name = name.to_ascii_lowercase();
        self.geometry_values
            .get(&name)
            .copied()
            .or_else(|| self.values.get(&name).copied())
            .or_else(|| self.limit_default(&name))
            .or_else(|| {
                let key = Self::account_default_key(&name)?;
                let value = *Self::account_defaults().values.get(&key)?;
                Some(
                    if key.ends_with(".display")
                        && self.values.get("display.countdown").copied().unwrap_or(0.0) != 0.0
                    {
                        100.0 - value
                    } else {
                        value
                    },
                )
            })
    }

    pub fn insert_string(&mut self, name: &str, value: impl Into<String>) {
        Arc::make_mut(&mut self.strings).insert(name.to_ascii_lowercase(), value.into());
    }

    pub fn get_string(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.strings
            .get(&name)
            .map(String::as_str)
            .or_else(|| Self::limit_string_default(&name))
            .or_else(|| {
                let key = Self::account_default_key(&name)?;
                Self::account_defaults()
                    .strings
                    .get(&key)
                    .map(String::as_str)
            })
    }

    // Account IDs are dynamic, but their fields use the same schema as provider
    // bindings. Supply typed defaults during validation, startup and removal;
    // unknown fields must still report typos instead of silently becoming zero.
    fn account_default_key(name: &str) -> Option<String> {
        let (provider, rest) = name.strip_prefix("accounts.")?.split_once('.')?;
        let (id, field) = rest.split_once('.')?;
        (matches!(provider, "claude" | "codex")
            && !id.is_empty()
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'))
        .then(|| format!("account.{field}"))
    }

    fn account_defaults() -> &'static Self {
        static DEFAULTS: OnceLock<DataContext> = OnceLock::new();
        DEFAULTS.get_or_init(|| {
            let mut context = Self::default();
            context.insert_provider("account", None, false, false);
            context.insert_string("account.name", "");
            context.insert("account.selected", 0.0);
            context.insert("account.has_error", 0.0);
            context
        })
    }

    pub fn with_object(mut self, object: &ResolvedObject<'_>) -> Self {
        self.insert("object.x", object.x);
        self.insert("object.y", object.y);
        self.insert("object.width", object.width);
        self.insert("object.height", object.height);
        self.insert("this.x", object.x);
        self.insert("this.y", object.y);
        self.insert("this.width", object.width);
        self.insert("this.height", object.height);
        self.insert("this.gap", object.gap);
        self.insert("parent.width", object.parent_width);
        self.insert("parent.height", object.parent_height);
        self
    }
}

fn display_percentage(percentage: f64, countdown: bool) -> f64 {
    if countdown {
        100.0 - percentage
    } else {
        percentage
    }
}

/// One quota window's bindings under `<provider>.<window>`.
struct WindowBinding {
    percentage: f64,
    available: bool,
    /// Draw an empty gauge in either usage direction instead of a full
    /// "remaining" one, for a window the provider did not report.
    empty_gauge: bool,
    /// Pace: the share the window would have spent by now at an even rate.
    expected: Option<f64>,
    resets_at: Option<SystemTime>,
}

/// Length of a window from its start to its reset, or `fallback`.
fn window_length(
    start: Option<SystemTime>,
    end: Option<SystemTime>,
    fallback: Duration,
) -> Duration {
    start
        .zip(end)
        .and_then(|(start, end)| end.duration_since(start).ok())
        .filter(|length| !length.is_zero())
        .unwrap_or(fallback)
}

fn section_expected(section: &UsageSection, length: Duration) -> Option<f64> {
    section
        .available
        .then(|| expected_used(section.resets_at?, length))
        .flatten()
}

/// The session, five-hour, weekly, 7-day and 30-day windows of a reading.
fn quota_windows(
    name: &str,
    usage: Option<&UsageData>,
    codex_session_fallback: bool,
) -> [(&'static str, WindowBinding); 5] {
    let binding = |section: Option<&UsageSection>, length, available, empty_gauge| WindowBinding {
        percentage: section.map_or(0.0, |section| section.percentage),
        available,
        empty_gauge,
        expected: section.and_then(|section| section_expected(section, length)),
        resets_at: section.and_then(|section| section.resets_at),
    };
    // An Antigravity pool window that was not reported draws an empty gauge.
    let quota = |section: Option<&UsageSection>, length| {
        let available = section.is_some_and(|section| section.available);
        binding(
            section,
            length,
            available,
            name.starts_with("antigravity") && !available,
        )
    };
    // Providers with a non-standard session slot (Qoder credit packs) give
    // its start; everyone else uses the 5-hour window.
    let session = if codex_session_fallback {
        quota(usage.map(|usage| &usage.weekly), WEEK)
    } else {
        let length = window_length(
            usage.and_then(|usage| usage.session_started_at),
            usage.and_then(|usage| usage.session.resets_at),
            FIVE_HOURS,
        );
        quota(usage.map(|usage| &usage.session), length)
    };
    // Optional windows count as available whenever they are present; a
    // missing OpenCode window draws an empty gauge.
    let optional = |section: Option<&UsageSection>, start, fallback| {
        let length = window_length(
            start,
            section.and_then(|section| section.resets_at),
            fallback,
        );
        let present = section.is_some();
        binding(section, length, present, name == "opencode" && !present)
    };
    [
        ("session", session),
        (
            "five_hour",
            quota(usage.map(|usage| &usage.session), FIVE_HOURS),
        ),
        ("weekly", quota(usage.map(|usage| &usage.weekly), WEEK)),
        // The true 7-day window (OpenCode Go). `weekly` may instead hold the
        // more constrained 30-day window, so its pace comes from `week`.
        (
            "week",
            optional(
                usage.and_then(|usage| usage.week.as_ref()),
                usage.and_then(|usage| usage.week_started_at),
                WEEK,
            ),
        ),
        // The 30-day window's real length is the billing period; cached
        // readings without `startsAt` fall back to a fixed 30 days.
        (
            "monthly",
            optional(
                usage.and_then(|usage| usage.monthly.as_ref()),
                usage.and_then(|usage| usage.monthly_started_at),
                MONTH,
            ),
        ),
    ]
}
