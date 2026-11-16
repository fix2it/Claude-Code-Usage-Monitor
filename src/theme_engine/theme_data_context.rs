use super::*;

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
        context.insert("canvas.width", canvas.width as f64);
        context.insert("canvas.height", canvas.height as f64);
        context.insert("parent.width", canvas.width as f64);
        context.insert("parent.height", canvas.height as f64);
        context.insert("host.width", runtime.host_width as f64);
        context.insert("host.height", runtime.host_height as f64);
        context.insert("pi", std::f64::consts::PI);
        context.insert("e", std::f64::consts::E);
        context.insert("true", 1.0);
        context.insert("false", 0.0);
        context.insert_string("app.version", env!("CARGO_PKG_VERSION"));
        let mut version_parts = env!("CARGO_PKG_VERSION")
            .split(['.', '-', '+'])
            .filter_map(|part| part.parse::<f64>().ok());
        context.insert("app.version.major", version_parts.next().unwrap_or(0.0));
        context.insert("app.version.minor", version_parts.next().unwrap_or(0.0));
        context.insert("app.version.patch", version_parts.next().unwrap_or(0.0));
        context.insert(
            "system.dark",
            crate::theme::is_dark_mode_cached() as u8 as f64,
        );
        context.insert("data.poll_ok", runtime.poll_ok as u8 as f64);
        context.insert("data.has_error", runtime.has_error as u8 as f64);
        context.insert(
            "data.loading",
            (!runtime.poll_ok && !runtime.has_error) as u8 as f64,
        );
        let strings = runtime.language.strings();
        context.insert_string("i18n.locale", runtime.language.code());
        context.insert_string("i18n.session_window", strings.session_window);
        context.insert_string("i18n.weekly_window", strings.weekly_window);
        context.insert_string("i18n.cursor_auto_window", strings.cursor_auto_window);
        context.insert_string("i18n.cursor_api_window", strings.cursor_api_window);
        context.insert_string("i18n.now", strings.now);
        context.insert_string("i18n.day_suffix", strings.day_suffix);
        context.insert_string("i18n.hour_suffix", strings.hour_suffix);
        context.insert_string("i18n.minute_suffix", strings.minute_suffix);
        context.insert_string("i18n.second_suffix", strings.second_suffix);
        context.insert(
            "providers.count",
            (runtime.provider_count()
                + pool_separate(runtime, ProviderId::Claude) as usize
                + pool_separate(runtime, ProviderId::Codex) as usize
                + agy_separate(data, runtime) as usize) as f64,
        );
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_secs_f64())
            .unwrap_or(0.0);
        context.insert("time.now.unix", now);
        context.insert("time.now.milliseconds", now * 1000.0);
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
                    context.insert(&format!("time.{zone}.{component}"), f64::from(value));
                }
            }
        }
        for descriptor in PROVIDER_DESCRIPTORS {
            context.insert_string(&format!("{}.account.name", descriptor.key), "");
            context.insert(
                &format!("providers.{}.enabled", descriptor.key),
                runtime.provider_enabled(descriptor.id) as u8 as f64,
            );
        }
        context.insert("display.countdown", runtime.countdown as u8 as f64);
        context.insert("display.show_pace", runtime.show_pace as u8 as f64);
        context.insert(
            "display.pool_split_supported",
            runtime.pool_split_supported as u8 as f64,
        );
        context.insert("display.show_fable", runtime.show_fable as u8 as f64);
        let (fable_main, fable_x) = fable_pools(runtime);
        context.insert("display.fable_main", fable_main as u8 as f64);
        context.insert("display.fable_x", fable_x as u8 as f64);
        context.insert(
            "display.fable_visible",
            fable_visible(data, runtime) as u8 as f64,
        );
        context.insert(
            "display.fable_separate",
            (fable_visible(data, runtime) && claude_dual(runtime)) as u8 as f64,
        );
        context.insert(
            "display.fable_row",
            (if pool_separate(runtime, ProviderId::Claude) {
                runtime.show_fable
            } else {
                fable_visible(data, runtime) && !claude_dual(runtime)
            }) as u8 as f64,
        );
        context.insert(
            "display.fable_x_row",
            (pool_separate(runtime, ProviderId::Claude) && runtime.show_fable_x) as u8 as f64,
        );
        context.insert(
            "display.claude_split",
            pool_separate(runtime, ProviderId::Claude) as u8 as f64,
        );
        context.insert(
            "display.codex_split",
            pool_separate(runtime, ProviderId::Codex) as u8 as f64,
        );
        context.insert(
            "display.agy_pool_split",
            agy_separate(data, runtime) as u8 as f64,
        );
        insert_animation_bindings(&mut context, &runtime, widget_count(data, runtime));
        context.insert(
            "antigravity.gemini.enabled",
            runtime.antigravity_gemini_enabled as u8 as f64,
        );
        context.insert(
            "antigravity.external.enabled",
            runtime.antigravity_external_enabled as u8 as f64,
        );
        let pool_o = antigravity_pool_o(data, runtime) && !agy_separate(data, runtime);
        context.insert(
            "antigravity.x.enabled",
            runtime.antigravity_x_enabled as u8 as f64,
        );
        context.insert("antigravity.pool_o", pool_o as u8 as f64);
        context.insert(
            "antigravity.a.enabled",
            runtime.antigravity_a_enabled as u8 as f64,
        );
        context.insert(
            "display.agy_split",
            antigravity_split(data, runtime) as u8 as f64,
        );
        context.insert("display.agy_o_external", 0.0);
        // Two rows: Gemini and external models (pool A), or pools A and O.
        let agy_dual_mode =
            (runtime.antigravity_gemini_enabled && runtime.antigravity_external_enabled) || pool_o;
        context.insert("antigravity.dual_mode", agy_dual_mode as u8 as f64);
        let crossfade = runtime.crossfade_opacity.clamp(0.0, 1.0);
        context.insert("antigravity.crossfade_opacity", crossfade);
        let (agy_normal_opacity, agy_hover_opacity) =
            asymmetric_crossfade_opacities(1.0 - crossfade);
        context.insert("antigravity.normal.opacity", agy_normal_opacity);
        context.insert("antigravity.hover.opacity", agy_hover_opacity);
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
        context.insert(
            "antigravity.gemini.opacity",
            if only_gemini_a { 1.0 } else { gemini_opacity },
        );
        context.insert(
            "antigravity.external.opacity",
            if only_gemini_a { 0.0 } else { external_opacity },
        );
        context.insert("claude.hovered", runtime.claude_hovered as u8 as f64);
        context.insert_hover_crossfade("claude", runtime.claude_crossfade_opacity);
        context.insert(
            "claude.main.enabled",
            runtime.claude_main_enabled as u8 as f64,
        );
        context.insert("claude.x.enabled", runtime.claude_x_enabled as u8 as f64);
        context.insert("claude.dual_mode", claude_dual(runtime) as u8 as f64);
        context.insert("codex.hovered", runtime.codex_hovered as u8 as f64);
        context.insert_hover_crossfade("codex", runtime.codex_crossfade_opacity);
        context.insert(
            "codex.main.enabled",
            runtime.codex_main_enabled as u8 as f64,
        );
        context.insert("codex.x.enabled", runtime.codex_x_enabled as u8 as f64);
        context.insert(
            "codex.dual_mode",
            (runtime.codex_main_enabled
                && runtime.codex_x_enabled
                && runtime.codex_pools_one_widget) as u8 as f64,
        );
        context.insert_hover_crossfade("opencode", runtime.opencode_crossfade_opacity);
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
        context.insert("qoder.hover_phase", qoder_phase);
        context.insert_hover_crossfade("qoder", qoder_phase);
        context.insert_hover_crossfade(AGY_EXTERNAL_KEY, runtime.agy_external_crossfade_opacity);
        if let Some(data) = data {
            for account in &data.accounts {
                let key = format!(
                    "accounts.{}.{}",
                    account.provider.descriptor().key,
                    account.profile.id
                );
                context.insert_string(&format!("{key}.name"), &account.profile.name);
                context.insert(&format!("{key}.selected"), account.selected as u8 as f64);
                context.insert(
                    &format!("{key}.has_error"),
                    account.error.is_some() as u8 as f64,
                );
                context.insert_provider(
                    &key,
                    account.usage.as_ref(),
                    account.provider == ProviderId::Codex,
                    runtime.countdown,
                );
            }
            context.insert_missing_pool_accounts(data, runtime.countdown);
            for descriptor in PROVIDER_DESCRIPTORS {
                if let Some(account) = data
                    .accounts
                    .iter()
                    .find(|account| account.provider == descriptor.id && account.selected)
                {
                    context.insert(
                        &format!("{}.has_error", descriptor.key),
                        account.error.is_some() as u8 as f64,
                    );
                }
                context.insert_string(
                    &format!("{}.account.name", descriptor.key),
                    data.selected_account_name(descriptor.id).unwrap_or(""),
                );
                context.insert_provider(
                    descriptor.key,
                    data.get(descriptor.id),
                    descriptor.id == ProviderId::Codex,
                    runtime.countdown,
                );
            }
            let pool_o_usage = data
                .antigravity_x()
                .and_then(|account| account.usage.as_ref());
            // With pool A off, pool O fills the single-pool layout alone.
            let pool_a = if runtime.antigravity_o_only() {
                pool_o_usage
            } else {
                // Never pool O's stand-in reading under pool A's label.
                data.antigravity_pool_a_reading()
            };
            context.insert_antigravity_rows(pool_a, pool_o_usage, pool_o, runtime);
            if let Some(agy) = pool_a {
                // Antigravity answered this poll; pool-level gaps are "no limit
                // data" ("--"), not a provider error.
                context.insert("antigravity.reported", 1.0);
                context.insert_provider("antigravity.gemini", Some(agy), false, runtime.countdown);
                // A missing external pool stays empty ("--"); never show Gemini figures in its place.
                context.insert_provider(
                    "antigravity.external",
                    agy.external_pool.as_deref(),
                    false,
                    runtime.countdown,
                );
            } else {
                context.insert_provider("antigravity.gemini", None, false, runtime.countdown);
                context.insert_provider("antigravity.external", None, false, runtime.countdown);
            }
            let active = ProviderId::ALL
                .into_iter()
                .find_map(|provider| data.get(provider).map(|usage| (provider, usage)));
            context.insert_provider(
                "active",
                active.map(|(_, usage)| usage),
                active.is_some_and(|(provider, _)| provider == ProviderId::Codex),
                runtime.countdown,
            );
        } else {
            for descriptor in PROVIDER_DESCRIPTORS {
                context.insert_provider(
                    descriptor.key,
                    None,
                    descriptor.id == ProviderId::Codex,
                    runtime.countdown,
                );
            }
            context.insert_provider("antigravity.gemini", None, false, runtime.countdown);
            context.insert_provider("antigravity.external", None, false, runtime.countdown);
            context.insert_antigravity_rows(None, None, pool_o, runtime);
            context.insert_provider("active", None, false, runtime.countdown);
        }
        // Theme validation evaluates every expression even when its parent is
        // hidden, so the split namespaces must exist in single-pool contexts.
        for pool in SplitPool::ALL {
            context.copy_prefix(
                &format!("{}.", pool.base_key()),
                &format!("{}.", pool.key()),
            );
        }
        if pool_separate(runtime, ProviderId::Claude) {
            // Claude rebinds even before the first poll (the tiles then read
            // nothing); Codex keeps its empty provider bindings until data.
            context.bind_account_pools(SplitPool::ClaudeX, &[], true, runtime);
        }
        if pool_separate(runtime, ProviderId::Codex) {
            context.bind_account_pools(SplitPool::CodexX, &["credits."], data.is_some(), runtime);
        }
        if agy_separate(data, runtime) {
            context.insert_antigravity_pool_o(data, runtime);
        }
        context
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
        usage: Option<&crate::models::UsageData>,
        codex_compatibility: bool,
        countdown: bool,
    ) {
        // What a gauge or a badge should show. `percentage` stays the share
        // that has been spent so warning thresholds keep working, while
        // `display` follows the countdown setting.
        self.insert_limits(name, usage, countdown);
        let display = |percentage: f64| {
            if countdown {
                100.0 - percentage
            } else {
                percentage
            }
        };
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
        let (five_hour, weekly) = usage
            .map(|usage| (usage.session.percentage, usage.weekly.percentage))
            .unwrap_or((0.0, 0.0));
        // Before Codex windows were classified by duration, a weekly-only
        // response arrived in the primary slot and was exposed to themes as
        // `codex.session`. Keep that established binding working for existing
        // custom themes, while `codex.five_hour` always means the real window.
        let use_codex_session_fallback = codex_compatibility
            && usage.is_some_and(|usage| !usage.session.available && usage.weekly.available);
        let session = if use_codex_session_fallback {
            weekly
        } else {
            five_hour
        };
        let (session_available, weekly_available) = usage
            .map(|usage| {
                (
                    usage.session.available || use_codex_session_fallback,
                    usage.weekly.available,
                )
            })
            .unwrap_or((false, false));
        self.insert(
            &format!("{name}.session.available"),
            session_available as u8 as f64,
        );
        self.insert(
            &format!("{name}.five_hour.available"),
            usage.is_some_and(|usage| usage.session.available) as u8 as f64,
        );
        self.insert(
            &format!("{name}.weekly.available"),
            weekly_available as u8 as f64,
        );
        self.insert(&format!("{name}.session.percentage"), session);
        self.insert(&format!("{name}.session.remaining"), 100.0 - session);
        // An Antigravity pool window that was not reported draws an empty
        // gauge in either usage direction instead of a full "remaining" one.
        let shown_session = if name.starts_with("antigravity") && !session_available {
            0.0
        } else {
            display(session)
        };
        self.insert(&format!("{name}.session.display"), shown_session);
        self.insert(&format!("{name}.five_hour.percentage"), five_hour);
        self.insert(&format!("{name}.five_hour.remaining"), 100.0 - five_hour);
        // An Antigravity pool window that was not reported draws an empty
        // gauge in either usage direction instead of a full "remaining" one.
        let shown_five_hour = if name.starts_with("antigravity")
            && !usage.is_some_and(|usage| usage.session.available)
        {
            0.0
        } else {
            display(five_hour)
        };
        self.insert(&format!("{name}.five_hour.display"), shown_five_hour);
        self.insert(&format!("{name}.weekly.percentage"), weekly);
        self.insert(&format!("{name}.weekly.remaining"), 100.0 - weekly);
        // An Antigravity pool window that was not reported draws an empty
        // gauge in either usage direction instead of a full "remaining" one.
        let shown_weekly = if name.starts_with("antigravity") && !weekly_available {
            0.0
        } else {
            display(weekly)
        };
        self.insert(&format!("{name}.weekly.display"), shown_weekly);
        // Pace: the share a window would have spent by now at an even rate.
        // `expected_display` follows the countdown setting like `display`.
        // Providers with a non-standard session slot (Qoder credit packs)
        // give its start; everyone else uses the 5-hour window.
        let session_length = usage
            .and_then(|usage| usage.session_started_at.zip(usage.session.resets_at))
            .and_then(|(start, end)| end.duration_since(start).ok())
            .filter(|length| !length.is_zero())
            .unwrap_or(FIVE_HOURS);
        let five_hour_section = usage
            .filter(|usage| usage.session.available)
            .map(|usage| &usage.session);
        let weekly_section = usage
            .filter(|usage| usage.weekly.available)
            .map(|usage| &usage.weekly);
        let session_section = if use_codex_session_fallback {
            weekly_section.map(|section| (section, WEEK))
        } else {
            five_hour_section.map(|section| (section, session_length))
        };
        for (window, section) in [
            ("session", session_section),
            (
                "five_hour",
                five_hour_section.map(|section| (section, FIVE_HOURS)),
            ),
            ("weekly", weekly_section.map(|section| (section, WEEK))),
        ] {
            let expected =
                section.and_then(|(section, length)| expected_used(section.resets_at?, length));
            self.insert(
                &format!("{name}.{window}.expected_available"),
                expected.is_some() as u8 as f64,
            );
            let expected = expected.unwrap_or(0.0);
            self.insert(&format!("{name}.{window}.expected"), expected);
            self.insert(
                &format!("{name}.{window}.expected_display"),
                display(expected),
            );
        }
        // The true 7-day window (OpenCode Go). `weekly` above may instead
        // hold the more constrained 30-day window, so its pace bindings must
        // come from `week` directly rather than from the auto-selected slot.
        let week = usage.and_then(|usage| usage.week.as_ref());
        let week_percentage = week.map(|week| week.percentage).unwrap_or(0.0);
        let week_label = usage
            .and_then(|usage| usage.week_label.as_deref())
            .unwrap_or("7d")
            .to_string();
        self.insert_string(&format!("{name}.week.name"), pool_name(&week_label));
        self.insert_string(&format!("{name}.week.label"), week_label);
        let week_length = usage
            .and_then(|usage| {
                usage
                    .week_started_at
                    .zip(week.and_then(|week| week.resets_at))
            })
            .and_then(|(start, end)| end.duration_since(start).ok())
            .filter(|length| !length.is_zero())
            .unwrap_or(WEEK);
        self.insert(
            &format!("{name}.week.available"),
            week.is_some() as u8 as f64,
        );
        self.insert(&format!("{name}.week.percentage"), week_percentage);
        self.insert(&format!("{name}.week.remaining"), 100.0 - week_percentage);
        let shown_week = if name == "opencode" && week.is_none() {
            0.0
        } else {
            display(week_percentage)
        };
        self.insert(&format!("{name}.week.display"), shown_week);
        let week_expected = week
            .filter(|week| week.available)
            .and_then(|week| expected_used(week.resets_at?, week_length));
        self.insert(
            &format!("{name}.week.expected_available"),
            week_expected.is_some() as u8 as f64,
        );
        let week_expected = week_expected.unwrap_or(0.0);
        self.insert(&format!("{name}.week.expected"), week_expected);
        self.insert(
            &format!("{name}.week.expected_display"),
            display(week_expected),
        );
        let monthly = usage.and_then(|usage| usage.monthly.as_ref());
        let monthly_percentage = monthly.map(|m| m.percentage).unwrap_or(0.0);
        self.insert_string(&format!("{name}.monthly.label"), "30d");
        self.insert(&format!("{name}.monthly.percentage"), monthly_percentage);
        self.insert(
            &format!("{name}.monthly.remaining"),
            100.0 - monthly_percentage,
        );
        let shown_monthly = if name == "opencode" && monthly.is_none() {
            0.0
        } else {
            display(monthly_percentage)
        };
        self.insert(&format!("{name}.monthly.display"), shown_monthly);
        self.insert(
            &format!("{name}.monthly.available"),
            monthly.is_some() as u8 as f64,
        );
        // The 30-day window's real length is the billing period. Cached
        // readings without `startsAt` fall back to a fixed 30-day window.
        let monthly_length = usage
            .and_then(|usage| usage.monthly_started_at)
            .zip(monthly.and_then(|monthly| monthly.resets_at))
            .and_then(|(start, end)| end.duration_since(start).ok())
            .filter(|length| !length.is_zero())
            .unwrap_or(MONTH);
        let monthly_expected = monthly
            .filter(|monthly| monthly.available)
            .and_then(|monthly| expected_used(monthly.resets_at?, monthly_length));
        self.insert(
            &format!("{name}.monthly.expected_available"),
            monthly_expected.is_some() as u8 as f64,
        );
        let monthly_expected = monthly_expected.unwrap_or(0.0);
        self.insert(&format!("{name}.monthly.expected"), monthly_expected);
        self.insert(
            &format!("{name}.monthly.expected_display"),
            display(monthly_expected),
        );
        self.insert(&format!("{name}.available"), usage.is_some() as u8 as f64);
        // Carried over from an earlier poll: real figures, not current ones.
        self.insert(
            &format!("{name}.stale"),
            usage.is_some_and(|usage| usage.stale) as u8 as f64,
        );
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
            display(credits_percentage),
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
            None => five_hour.max(weekly),
        };
        self.insert(&format!("{name}.headline.percentage"), headline);
        self.insert(&format!("{name}.headline.remaining"), 100.0 - headline);
        self.insert(&format!("{name}.headline.display"), display(headline));
        let reset_value = |reset: Option<std::time::SystemTime>| {
            let unix = reset
                .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|value| value.as_secs_f64())
                .unwrap_or(0.0);
            let seconds = reset
                .and_then(|value| value.duration_since(std::time::SystemTime::now()).ok())
                .map(|value| value.as_secs_f64())
                .unwrap_or(0.0);
            (unix, seconds)
        };
        let (five_hour_unix, five_hour_seconds) =
            reset_value(usage.and_then(|value| value.session.resets_at));
        let (weekly_unix, weekly_seconds) =
            reset_value(usage.and_then(|value| value.weekly.resets_at));
        let (session_unix, session_seconds) = if use_codex_session_fallback {
            (weekly_unix, weekly_seconds)
        } else {
            (five_hour_unix, five_hour_seconds)
        };
        let five_hour_available = usage.is_some_and(|value| value.session.available);
        let weekly_available = usage.is_some_and(|value| value.weekly.available);
        let session_available = if use_codex_session_fallback {
            weekly_available
        } else {
            five_hour_available
        };
        // Presence is independent of percentage and reset times. Monthly
        // availability remains based on its explicitly optional section.
        for (window, available) in [
            ("session", session_available),
            ("five_hour", five_hour_available),
            ("weekly", weekly_available),
        ] {
            self.insert(
                &format!("{name}.{window}.available"),
                available as u8 as f64,
            );
        }
        let (monthly_unix, monthly_seconds) =
            reset_value(monthly.and_then(|value| value.resets_at));
        let (week_unix, week_seconds) = reset_value(week.and_then(|value| value.resets_at));
        for (window, unix, seconds) in [
            ("session", session_unix, session_seconds),
            ("five_hour", five_hour_unix, five_hour_seconds),
            ("weekly", weekly_unix, weekly_seconds),
            ("week", week_unix, week_seconds),
            ("monthly", monthly_unix, monthly_seconds),
        ] {
            self.insert(&format!("{name}.{window}.reset.unix"), unix);
            self.insert(&format!("{name}.{window}.reset.seconds"), seconds);
            self.insert(&format!("{name}.{window}.reset.minutes"), seconds / 60.0);
            self.insert(&format!("{name}.{window}.reset.hours"), seconds / 3600.0);
            self.insert(&format!("{name}.{window}.reset.days"), seconds / 86400.0);
        }
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
