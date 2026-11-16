//! Tray tooltip text and the themed tray icon sync.

use super::*;

/// Compact usage text (whole percents); the widget tooltip adds precision.
#[cfg(test)]
pub(super) fn tray_usage_summary_lines(
    data: &AppUsageData,
    providers: ProviderSet,
    language: LanguageId,
    countdown: bool,
) -> Vec<String> {
    usage_summary_lines(data, providers, language, countdown, true, true, None)
}

#[cfg(test)]
pub(super) fn usage_summary_lines(
    data: &AppUsageData,
    providers: ProviderSet,
    language: LanguageId,
    countdown: bool,
    show_antigravity_gemini: bool,
    show_antigravity_external: bool,
    detailed_at: Option<SystemTime>,
) -> Vec<String> {
    usage_summary_lines_with_side_pools(
        data,
        providers,
        language,
        countdown,
        show_antigravity_gemini,
        show_antigravity_external,
        detailed_at,
        false,
    )
}

/// `side_pools` adds a line for every enabled pool that is not the provider's
/// own reading (Claude X, Codex X, agy O next to pool A), so a split tile's
/// numbers are also in the widget tooltip.
#[allow(clippy::too_many_arguments)]
pub(super) fn usage_summary_lines_with_side_pools(
    data: &AppUsageData,
    providers: ProviderSet,
    language: LanguageId,
    countdown: bool,
    show_antigravity_gemini: bool,
    show_antigravity_external: bool,
    detailed_at: Option<SystemTime>,
    side_pools: bool,
) -> Vec<String> {
    let strings = language.strings();
    let shown = |percentage: f64| {
        if countdown {
            100.0 - percentage
        } else {
            percentage
        }
    };
    let window_text = |section: &crate::models::UsageSection| {
        if section.available {
            if let Some(now) = detailed_at {
                let percentage = format!("{:.1}%", shown(section.percentage));
                match section.resets_at {
                    Some(reset) => format!(
                        "{percentage} ({})",
                        widget_tooltip::reset_countdown(reset, now, &strings)
                    ),
                    None => percentage,
                }
            } else {
                format!("{:.0}%", shown(section.percentage))
            }
        } else {
            "--".to_string()
        }
    };
    let quota_line = |label: &str, usage: &crate::models::UsageData| {
        let session_label = usage
            .session_label
            .as_deref()
            .unwrap_or(strings.session_window);
        let weekly_label = usage
            .weekly_label
            .as_deref()
            .unwrap_or(strings.weekly_window);
        format!(
            "{label} {session_label}: {} | {weekly_label}: {}",
            window_text(&usage.session),
            window_text(&usage.weekly),
        )
    };
    let mut lines = Vec::new();
    for provider in providers.iter() {
        let base_name = language.text(provider.descriptor().display_name);
        let acct = data.selected_account_name(provider);
        let label = |pool: Option<&str>| match (pool, acct) {
            (Some(pool), Some(name)) => format!("{base_name} ({pool}, {name})"),
            (Some(pool), None) => format!("{base_name} ({pool})"),
            (None, Some(name)) => format!("{base_name} ({name})"),
            (None, None) => base_name.to_string(),
        };
        if provider == ProviderId::Antigravity {
            if show_antigravity_gemini {
                if let Some(usage) = data.get(provider) {
                    lines.push(quota_line(&label(Some("Gemini")), usage));
                }
            }
            if show_antigravity_external {
                if data.get(provider).is_some() && data.antigravity_external().is_none() {
                    lines.push(format!("{}: --", label(Some("Claude/GPT"))));
                }
                if let Some(usage) = data.antigravity_external() {
                    lines.push(quota_line(&label(Some("Claude/GPT")), usage));
                }
            }
        } else {
            let Some(usage) = data.get(provider) else {
                continue;
            };
            if provider == ProviderId::Claude && !usage.session.available {
                let weekly_label = usage
                    .weekly_label
                    .as_deref()
                    .unwrap_or(strings.weekly_window);
                lines.push(format!(
                    "{} {weekly_label}: {}",
                    label(None),
                    window_text(&usage.weekly)
                ));
                continue;
            }
            lines.push(quota_line(&label(None), usage));
        }
    }
    if side_pools {
        for account in data.accounts.iter().filter(|account| {
            providers.contains(account.provider)
                && !account.selected
                && !account.is_antigravity_pool_a()
        }) {
            let label = format!(
                "{} ({})",
                language.text(account.provider.descriptor().display_name),
                account.profile.name
            );
            if !account.is_side_pool() || show_antigravity_gemini {
                match &account.usage {
                    Some(usage) => lines.push(quota_line(&label, usage)),
                    None => lines.push(format!("{label}: --")),
                }
            }
            if account.is_side_pool() && show_antigravity_external {
                let label = format!(
                    "{} (Claude/GPT, {})",
                    language.text(account.provider.descriptor().display_name),
                    account.profile.name
                );
                match account
                    .usage
                    .as_ref()
                    .and_then(|usage| usage.external_pool.as_deref())
                {
                    Some(external) => lines.push(quota_line(&label, external)),
                    None => lines.push(format!("{label}: --")),
                }
            }
        }
    }
    lines
}

/// Windows cuts a tray tooltip at 127 characters (`szTip`).
pub(super) const TRAY_TIP_MAX_CHARS: usize = 127;

/// The tray tooltip is a status line, not the usage numbers (those are on the
/// widget and the icon): "updated HH:MM", or one short line per failing pool.
pub(super) fn tray_status_from_state() -> Option<String> {
    let state = lock_state();
    let state = state.as_ref()?;
    let errors = tray_short_error_lines(
        state.data.as_ref(),
        state.last_poll_failure,
        state.providers,
        state.language,
    );
    if !errors.is_empty() {
        return Some(fit_tray_tip(&errors, state.language));
    }
    if !state.last_poll_ok {
        return None;
    }
    let (hour, minute) = state.last_poll_ok_at.and_then(local_hour_minute)?;
    Some(tray_updated_text(state.language, hour, minute))
}

pub(super) fn tray_updated_text(language: LanguageId, hour: u16, minute: u16) -> String {
    format!(
        "{} \u{2014} {} {hour:02}:{minute:02}",
        language.strings().window_title,
        language.text("updated"),
    )
}

/// Local wall-clock hour and minute of `time`.
pub(super) fn local_hour_minute(time: SystemTime) -> Option<(u16, u16)> {
    use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTimeEx};
    // FILETIME counts 100 ns ticks since 1601-01-01.
    let ticks = (time.duration_since(UNIX_EPOCH).ok()?.as_nanos() / 100) as u64
        + 11_644_473_600 * 10_000_000;
    let file_time = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    unsafe {
        FileTimeToSystemTime(&file_time, &mut utc).ok()?;
        SystemTimeToTzSpecificLocalTimeEx(None, &utc, &mut local).ok()?;
    }
    Some((local.wHour, local.wMinute))
}

/// Short failure text for the tray, where the full message would be cut off.
pub(super) fn tray_short_error(error: poller::PollError, language: LanguageId) -> &'static str {
    use poller::PollError;
    language.text(match error {
        PollError::NoCredentials => "sign in again",
        _ if error.is_auth() => "sign in again",
        PollError::HttpStatus(429) => "rate limited",
        PollError::NetworkError => "no connection",
        _ => "poll failed",
    })
}

/// Same failures as `tray_error_lines`, one short line each.
pub(super) fn tray_short_error_lines(
    data: Option<&AppUsageData>,
    failure: Option<poller::PollFailure>,
    providers: ProviderSet,
    language: LanguageId,
) -> Vec<String> {
    tray_failures(data, failure, providers)
        .into_iter()
        .map(|(provider, account, error)| {
            format!(
                "{}: {}",
                tray_pool_label(provider, account),
                tray_short_error(error, language)
            )
        })
        .collect()
}

/// Short pool name for the tray: the provider key for its default pool
/// (`claude`), the pool id for a second pool (`claude_x`, `codex_x`,
/// `antigravity_x`), and the key plus the name for any other profile.
pub(super) fn tray_pool_label(
    provider: ProviderId,
    account: Option<&crate::accounts::AccountProfile>,
) -> String {
    let key = provider.descriptor().key;
    match account {
        None => key.to_string(),
        Some(profile) if profile.id == "default" || profile.id.is_empty() => key.to_string(),
        Some(profile) if profile.id.starts_with(key) => profile.id.clone(),
        Some(profile) => format!("{key} {}", profile.name),
    }
}

/// Length as Windows counts it in `szTip`: UTF-16 code units.
pub(super) fn tray_tip_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// Shorten a "label: reason" line to `budget` UTF-16 units by cutting the
/// label, so the reason at the end always stays readable.
pub(super) fn shorten_tray_line(line: &str, budget: usize) -> String {
    if tray_tip_len(line) <= budget {
        return line.to_string();
    }
    let (label, reason) = line.rsplit_once(": ").unwrap_or((line, ""));
    let reason = if reason.is_empty() {
        String::new()
    } else {
        format!(": {reason}")
    };
    let room = budget.saturating_sub(tray_tip_len(&reason) + 1);
    let mut head = String::new();
    for ch in label.chars() {
        if tray_tip_len(&head) + ch.len_utf16() > room {
            break;
        }
        head.push(ch);
    }
    let shortened = format!("{head}\u{2026}{reason}");
    if tray_tip_len(&shortened) <= budget {
        return shortened;
    }
    // A reason longer than the whole budget: keep what fits of the line.
    let mut cut = String::new();
    for ch in line.chars() {
        if tray_tip_len(&cut) + ch.len_utf16() + 1 > budget {
            break;
        }
        cut.push(ch);
    }
    cut.push('\u{2026}');
    cut
}

/// Whole lines that fit the tray limit; the lines left over become "… +N".
/// A line too long on its own is shortened, never cut by Windows.
pub(super) fn fit_tray_tip(lines: &[String], language: LanguageId) -> String {
    let more = |count: usize| format!("\u{2026} +{count} {}", language.text("more"));
    let mut text = String::new();
    for (index, line) in lines.iter().enumerate() {
        let left_after = lines.len() - index - 1;
        let sep = usize::from(!text.is_empty());
        let tail = if left_after > 0 {
            tray_tip_len(&more(left_after)) + 1
        } else {
            0
        };
        let used = tray_tip_len(&text) + sep;
        if used + tray_tip_len(line) + tail <= TRAY_TIP_MAX_CHARS {
            if sep == 1 {
                text.push('\n');
            }
            text.push_str(line);
            continue;
        }
        if text.is_empty() {
            // The first line always shows, shortened to leave room for "+N".
            text = shorten_tray_line(line, TRAY_TIP_MAX_CHARS.saturating_sub(tail));
            if left_after > 0 {
                text.push('\n');
                text.push_str(&more(left_after));
            }
            return text;
        }
        text.push('\n');
        text.push_str(&more(lines.len() - index));
        return text;
    }
    text
}

pub(super) fn tray_error_lines(
    data: Option<&AppUsageData>,
    failure: Option<poller::PollFailure>,
    providers: ProviderSet,
    language: LanguageId,
) -> Vec<String> {
    tray_failures(data, failure, providers)
        .into_iter()
        .map(|(provider, account, error)| {
            let provider = language.text(provider.descriptor().display_name);
            match account.map(|profile| &profile.name) {
                Some(name) => format!("{provider} ({name}): {}", error.message(language)),
                None => format!("{provider}: {}", error.message(language)),
            }
        })
        .collect()
}

/// Failing pools as (provider, account name, error): accounts first, the
/// selected one leading, then a provider-wide failure no account explains.
pub(super) fn tray_failures(
    data: Option<&AppUsageData>,
    failure: Option<poller::PollFailure>,
    providers: ProviderSet,
) -> Vec<(
    ProviderId,
    Option<&crate::accounts::AccountProfile>,
    poller::PollError,
)> {
    let mut accounts: Vec<_> = data
        .into_iter()
        .flat_map(|data| &data.accounts)
        .filter(|account| {
            providers.contains(account.provider)
                && account.profile.enabled
                && account.error.is_some()
                // A hidden duplicate of agy pool A is not a failure.
                && !(account.is_side_pool()
                    && account.error == Some(poller::PollError::NoCredentials))
        })
        .collect();
    // Windows truncates tray tooltips: put the selected account's error first.
    accounts.sort_by_key(|account| !account.selected);
    let mut failures: Vec<_> = accounts
        .into_iter()
        .filter_map(|account| Some((account.provider, Some(&account.profile), account.error?)))
        .collect();
    let explained_by_account = |provider: ProviderId| {
        data.is_some_and(|data| {
            data.accounts.iter().any(|account| {
                account.provider == provider
                    && account.stands_for_provider()
                    && account.error.is_some()
            })
        })
    };
    // A whole failed poll (the newest error) first, then a provider without
    // profiles that failed while others answered earlier (its last reading
    // carried in as stale).
    let provider_errors = data
        .into_iter()
        .flat_map(|data| data.provider_errors())
        .map(|(provider, error)| poller::PollFailure { provider, error });
    for failure in failure.into_iter().chain(provider_errors) {
        if providers.contains(failure.provider)
            && !explained_by_account(failure.provider)
            && !failures
                .iter()
                .any(|(provider, account, _)| *provider == failure.provider && account.is_none())
        {
            failures.push((failure.provider, None, failure.error));
        }
    }
    failures
}

pub(super) fn tray_icon_tooltip_from_state() -> String {
    tray_status_from_state().unwrap_or_else(|| {
        lock_state()
            .as_ref()
            .map(|state| state.language.strings().window_title.to_string())
            .unwrap_or_else(|| "Claude Code Usage Monitor".to_string())
    })
}

pub(super) fn sync_tray_icon(hwnd: HWND) {
    let hwnd = controller_for(hwnd);
    widget_tooltip::refresh();
    let usage_tooltip = tray_status_from_state();
    let themed = {
        let mut state = lock_state();
        state.as_mut().and_then(|state| {
            effective_theme_from_state(state).map(|theme| {
                let runtime = theme_runtime_from_state(state);
                let metrics = tray_metrics(state, &theme, runtime);
                (theme, state.data.clone(), runtime, metrics)
            })
        })
    };
    if let Some((theme, data, runtime, metrics)) = themed {
        let has_tray_surfaces = theme.surfaces.iter().any(|surface| {
            surface
                .placement
                .nest
                .resolve(surface.placement.reference.region)
                == SurfaceNest::TrayIcon
        });
        if has_tray_surfaces {
            let icons = theme
                .surfaces
                .iter()
                .enumerate()
                .filter(|(surface_index, surface)| {
                    if !tray_surface_candidate(surface) {
                        return false;
                    }
                    let Some(metrics) = metrics.as_ref() else {
                        return false;
                    };
                    let surface_runtime =
                        theme_runtime_for_surface(&theme, *surface_index, runtime);
                    let surface_metrics =
                        theme_engine::metrics_for_phase(metrics.clone(), surface_runtime);
                    theme_engine::surface_should_render_with_metrics(
                        &theme,
                        *surface_index,
                        data.as_ref(),
                        surface_runtime,
                        &surface_metrics,
                    )
                })
                .filter_map(|(surface_index, surface)| {
                    let surface_runtime = theme_runtime_for_surface(&theme, surface_index, runtime);
                    let surface_metrics = theme_engine::metrics_for_phase(
                        metrics.as_ref().unwrap().clone(),
                        surface_runtime,
                    );
                    let (logical_width, logical_height) =
                        theme_engine::resolve_surface_size_with_metrics(
                            &theme,
                            surface_index,
                            data.as_ref(),
                            surface_runtime,
                            &surface_metrics,
                        );
                    let max_dimension = logical_width.max(logical_height) as f64;
                    let scale =
                        theme_surface_scale(&theme, surface_index).min(if max_dimension > 0.0 {
                            512.0 / max_dimension
                        } else {
                            1.0
                        });
                    if scale < 0.25 {
                        diagnose::log(format!(
                            "tray-icon theme surface '{}' exceeds the 512px source limit",
                            surface.name
                        ));
                        return None;
                    }
                    let rendered = theme_engine::render_theme_surface_with_metrics_at_scale(
                        &theme,
                        surface_index,
                        data.as_ref(),
                        surface_runtime,
                        scale,
                        &surface_metrics,
                    );
                    Some(tray_icon::ThemedTrayIcon {
                        surface_index,
                        tooltip: usage_tooltip
                            .clone()
                            .unwrap_or_else(|| surface.name.clone()),
                        width: rendered.width,
                        height: rendered.height,
                        pixels: rendered.pixels,
                    })
                })
                .collect::<Vec<_>>();
            tray_icon::sync_themed(hwnd, &icons);
            for icon in icons {
                theme_engine::recycle_pixel_buffer(icon.pixels);
            }
            return;
        }
    }
    tray_icon::sync(hwnd, &tray_icon_tooltip_from_state());
}

// Literal zero is disabled independently of data, surface size, and runtime.
pub(super) fn tray_surface_candidate(surface: &theme_engine::SceneObject) -> bool {
    surface
        .placement
        .nest
        .resolve(surface.placement.reference.region)
        == SurfaceNest::TrayIcon
        && !surface
            .render
            .0
            .trim()
            .parse::<f64>()
            .is_ok_and(|value| value == 0.0)
}

pub(super) fn tray_metrics(
    state: &mut AppState,
    theme: &ThemeDocument,
    runtime: theme_engine::ThemeRuntime,
) -> Option<theme_engine::DisplayMetrics> {
    theme
        .surfaces
        .iter()
        .enumerate()
        .any(|(index, surface)| {
            tray_surface_candidate(surface)
                && theme_engine::DataContext::surface_render_without_size(
                    &surface.render,
                    state.data.as_ref(),
                    &theme.canvas,
                    theme_runtime_for_surface(theme, index, runtime),
                ) != Some(false)
        })
        .then(|| prepared_metrics(state, theme, theme_runtime_for_surface(theme, 0, runtime)))
}

pub(super) fn theme_tray_uses_current_time(theme: &ThemeDocument) -> bool {
    theme
        .surfaces
        .iter()
        .enumerate()
        .filter(|(_, surface)| {
            surface
                .placement
                .nest
                .resolve(surface.placement.reference.region)
                == SurfaceNest::TrayIcon
        })
        .any(|(surface_index, _)| {
            theme
                .surface_current_time_refresh_interval(surface_index)
                .is_some()
        })
}

#[cfg(test)]
mod tray_usage_summary_tests {
    use super::*;
    use crate::models::{UsageData, UsageSection};

    fn usage(session: f64, weekly: f64, weekly_label: Option<&str>) -> UsageData {
        UsageData {
            session: UsageSection {
                available: true,
                percentage: session,
                resets_at: None,
            },
            weekly: UsageSection {
                available: true,
                percentage: weekly,
                resets_at: None,
            },
            weekly_label: weekly_label.map(str::to_string),
            ..Default::default()
        }
    }

    /// F69: Cursor (no profiles) fails while Claude answers. Its last reading
    /// is carried in as stale, and the tray names the failure instead of
    /// "updated", on both poll paths.
    #[test]
    fn a_failed_provider_without_profiles_is_listed_not_hidden_by_its_carried_reading() {
        use crate::poller::PollError;
        let providers = ProviderSet::from_enabled([ProviderId::Claude, ProviderId::Cursor]);
        let previous: AppUsageData = [
            (ProviderId::Claude, usage(10.0, 20.0, None)),
            (ProviderId::Cursor, usage(30.0, 40.0, None)),
        ]
        .into_iter()
        .collect();
        let poll = |provider: ProviderId| match provider {
            ProviderId::Cursor => Err(PollError::NetworkError),
            _ => Ok(usage(11.0, 21.0, None)),
        };
        let without_profiles = crate::poller::poll_providers_for_test(providers, poll).unwrap();
        let with_claude_profiles = crate::poller::poll_accounts_for_test(
            providers,
            &crate::accounts::AccountSettings::default(),
            |provider, _| poll(provider),
        )
        .unwrap();
        for fresh in [without_profiles, with_claude_profiles] {
            let data = crate::poller::carry_forward_failures(fresh, &previous, providers);
            assert!(
                data.get(ProviderId::Cursor)
                    .is_some_and(|usage| usage.stale),
                "the last Cursor reading is carried in as stale"
            );
            let lines = tray_short_error_lines(Some(&data), None, providers, LanguageId::English);
            assert_eq!(lines.len(), 1, "{lines:?}");
            assert!(lines[0].ends_with("no connection"), "{lines:?}");
            assert!(lines[0].to_lowercase().starts_with("cursor"), "{lines:?}");

            // A whole failed poll (nothing answered) keeps Cursor's failure.
            let failed_again =
                crate::poller::carry_forward_failures(AppUsageData::default(), &data, providers);
            let lines =
                tray_short_error_lines(Some(&failed_again), None, providers, LanguageId::English);
            assert_eq!(lines.len(), 1, "{lines:?}");
            assert!(lines[0].ends_with("no connection"), "{lines:?}");

            // A later whole-poll failure of Cursor replaces the older reason.
            let newer = Some(crate::poller::PollFailure {
                provider: ProviderId::Cursor,
                error: PollError::AuthRequired,
            });
            let lines = tray_short_error_lines(Some(&data), newer, providers, LanguageId::English);
            assert_eq!(lines.len(), 1, "{lines:?}");
            assert!(lines[0].ends_with("sign in again"), "{lines:?}");

            // The next poll with Cursor back clears the failure.
            let progress: AppUsageData = [(ProviderId::Cursor, usage(31.0, 41.0, None))]
                .into_iter()
                .collect();
            let recovered = crate::poller::merge_poll_progress(
                progress,
                &data,
                &crate::accounts::AccountSettings::default(),
            );
            assert!(
                tray_short_error_lines(Some(&recovered), None, providers, LanguageId::English)
                    .is_empty()
            );
        }
    }

    #[test]
    fn tray_status_names_each_failing_pool_in_a_few_words() {
        use crate::poller::{PollError, PollFailure};
        let providers = ProviderSet::from_enabled([ProviderId::Claude]);
        for (error, expected, russian) in [
            (PollError::HttpStatus(429), "rate limited", "лимит запросов"),
            (PollError::NetworkError, "no connection", "нет связи"),
            (
                PollError::TokenExpired,
                "sign in again",
                "нужно войти заново",
            ),
            (
                PollError::NoCredentials,
                "sign in again",
                "нужно войти заново",
            ),
            (
                PollError::HttpStatus(401),
                "sign in again",
                "нужно войти заново",
            ),
            (PollError::HttpStatus(500), "poll failed", "ошибка опроса"),
            (PollError::RequestFailed, "poll failed", "ошибка опроса"),
        ] {
            let failure = Some(PollFailure {
                provider: ProviderId::Claude,
                error,
            });
            assert_eq!(
                tray_short_error_lines(None, failure, providers, LanguageId::English),
                [format!("claude: {expected}")]
            );
            assert_eq!(
                tray_short_error_lines(
                    None,
                    failure,
                    providers,
                    LanguageId::from_code("ru").unwrap()
                ),
                [format!("claude: {russian}")]
            );
        }
        let mut data = AppUsageData::default();
        for (id, name, selected) in [
            ("claude_x", "Claude Code X", false),
            ("default", "Default", true),
            ("a1b2", "Work", false),
        ] {
            data.accounts.push(crate::models::AccountUsage {
                provider: ProviderId::Claude,
                profile: crate::accounts::AccountProfile {
                    id: id.into(),
                    name: name.into(),
                    enabled: true,
                    ..Default::default()
                },
                source_signature: String::new(),
                source_path: None,
                usage: None,
                error: Some(PollError::HttpStatus(429)),
                selected,
            });
        }
        let lines = tray_short_error_lines(Some(&data), None, providers, LanguageId::English);
        assert_eq!(
            lines,
            [
                "claude: rate limited",
                "claude_x: rate limited",
                "claude Work: rate limited"
            ]
        );
        // Both pools fit whole in the Windows tooltip, unlike the long messages.
        let tip = fit_tray_tip(&lines, LanguageId::English);
        assert_eq!(tip, lines.join("\n"));
        assert!(tray_tip_len(&tip) <= TRAY_TIP_MAX_CHARS);
    }

    #[test]
    fn tray_tip_keeps_whole_lines_and_counts_the_rest() {
        let lines: Vec<String> = (1..=6)
            .map(|index| format!("Provider number {index} (Account): rate limited"))
            .collect();
        let tip = fit_tray_tip(&lines, LanguageId::English);
        assert!(tray_tip_len(&tip) <= TRAY_TIP_MAX_CHARS, "{tip}");
        let shown: Vec<&str> = tip.lines().collect();
        let (last, kept) = shown.split_last().unwrap();
        assert_eq!(kept, &lines[..kept.len()]);
        assert_eq!(
            *last,
            format!("\u{2026} +{} more", lines.len() - kept.len())
        );
        assert!(!kept.is_empty());
        assert_eq!(fit_tray_tip(&lines[..1], LanguageId::English), lines[0]);
        // A first line longer than the limit keeps its reason and the count;
        // emoji take two UTF-16 units, the unit Windows counts.
        let long = vec![
            format!("Claude Code ({}): rate limited", "\u{1F600}".repeat(80)),
            "Codex: no connection".to_string(),
        ];
        let tip = fit_tray_tip(&long, LanguageId::English);
        assert!(tray_tip_len(&tip) <= TRAY_TIP_MAX_CHARS, "{tip}");
        let shown: Vec<&str> = tip.lines().collect();
        assert!(
            shown[0].starts_with("Claude Code (") && shown[0].ends_with("\u{2026}: rate limited")
        );
        assert_eq!(shown[1], "\u{2026} +1 more");
        let alone = fit_tray_tip(&long[..1], LanguageId::English);
        assert!(tray_tip_len(&alone) <= TRAY_TIP_MAX_CHARS);
        assert!(alone.ends_with(": rate limited"));
        // Lines that only fit when counted in chars are summarized instead.
        let wide: Vec<String> = (0..3)
            .map(|index| format!("P{index} ({}): poll failed", "\u{1F600}".repeat(18)))
            .collect();
        let tip = fit_tray_tip(&wide, LanguageId::English);
        assert!(tray_tip_len(&tip) <= TRAY_TIP_MAX_CHARS, "{tip}");
        assert_eq!(fit_tray_tip(&[], LanguageId::English), "");
    }

    #[test]
    fn tray_status_when_healthy_is_the_update_time() {
        assert_eq!(
            tray_updated_text(LanguageId::English, 7, 5),
            format!(
                "{} \u{2014} updated 07:05",
                LanguageId::English.strings().window_title
            )
        );
        assert!(
            tray_updated_text(LanguageId::from_code("ru").unwrap(), 17, 52)
                .ends_with("обновлено 17:52")
        );
        assert!(local_hour_minute(SystemTime::now()).is_some_and(|(h, m)| h < 24 && m < 60));
    }

    #[test]
    fn tray_errors_distinguish_causes_and_disappear_after_recovery() {
        use crate::poller::{PollError, PollFailure};
        let providers = ProviderSet::from_enabled([ProviderId::Claude]);
        for (error, expected) in [
            (PollError::TokenExpired, "Login expired"),
            (PollError::NoCredentials, "No usable login"),
            (PollError::AuthRequired, "Login rejected"),
            (PollError::RequestFailed, "Usage request failed"),
            (PollError::NetworkError, "Service unreachable"),
            (PollError::UnexpectedResponse, "Unexpected usage response"),
            (PollError::HttpStatus(429), "HTTP 429"),
        ] {
            let failure = PollFailure {
                provider: ProviderId::Claude,
                error,
            };
            let lines = tray_error_lines(None, Some(failure), providers, LanguageId::English);
            assert_eq!(lines.len(), 1);
            assert!(lines[0].contains(expected), "{:?}", lines);
            assert!(tray_error_lines(
                None,
                Some(failure),
                ProviderSet::from_enabled([ProviderId::Codex]),
                LanguageId::English
            )
            .is_empty());
        }
        assert!(tray_error_lines(None, None, providers, LanguageId::English).is_empty());
        let mut data = AppUsageData::default();
        for (name, selected) in [("Work", false), ("Personal", true)] {
            data.accounts.push(crate::models::AccountUsage {
                provider: ProviderId::Claude,
                profile: crate::accounts::AccountProfile {
                    name: name.into(),
                    enabled: true,
                    ..Default::default()
                },
                source_signature: String::new(),
                source_path: None,
                usage: None,
                error: Some(PollError::TokenExpired),
                selected,
            });
        }
        let lines = tray_error_lines(Some(&data), None, providers, LanguageId::English);
        assert!(lines[0].starts_with("Claude Code (Personal): Login expired"));
        assert!(lines[1].contains("Work"));
        for account in &mut data.accounts {
            account.error = None;
        }
        assert!(tray_error_lines(Some(&data), None, providers, LanguageId::English).is_empty());
        let instruction = LanguageId::English
            .provider_auth_error(ProviderId::Claude)
            .1;
        assert!(instruction.contains("desktop app") && instruction.contains("/login"));
    }

    #[test]
    fn claude_summaries_omit_an_absent_five_hour_window() {
        let mut reading = usage(0.0, 26.0, None);
        reading.session = UsageSection::default();
        let data = [(ProviderId::Claude, reading)].into_iter().collect();
        let providers = ProviderSet::from_enabled([ProviderId::Claude]);
        for (countdown, expected) in [(false, "26"), (true, "74")] {
            assert_eq!(
                tray_usage_summary_lines(&data, providers, LanguageId::English, countdown),
                [format!("Claude Code 7d: {expected}%")]
            );
            assert_eq!(
                usage_summary_lines(
                    &data,
                    providers,
                    LanguageId::English,
                    countdown,
                    true,
                    true,
                    Some(UNIX_EPOCH),
                ),
                [format!("Claude Code 7d: {expected}.0%")]
            );
        }
    }

    #[test]
    fn tray_summary_formats_enabled_provider_usage() {
        let data = [(ProviderId::Claude, usage(4.6, 42.4, None))]
            .into_iter()
            .collect();

        assert_eq!(
            tray_usage_summary_lines(
                &data,
                ProviderSet::from_enabled([ProviderId::Claude]),
                LanguageId::English,
                false,
            ),
            ["Claude Code 5h: 5% | 7d: 42%"]
        );
    }

    /// F58/F61: the widget tooltip lists every enabled side pool with its own
    /// numbers (Claude X, Codex X, agy O); the tray summary is unchanged.
    #[test]
    fn widget_summary_lists_side_pools_with_their_own_numbers() {
        use crate::accounts::{AccountProfile, AccountSettings, ANTIGRAVITY_X_ID};
        use crate::models::AccountUsage;

        let account = |provider, profile: AccountProfile, usage, selected| AccountUsage {
            provider,
            profile,
            source_signature: String::new(),
            source_path: None,
            usage,
            error: None,
            selected,
        };
        let named = |id: &str, name: &str| AccountProfile {
            id: id.into(),
            name: name.into(),
            enabled: true,
            ..Default::default()
        };
        let mut data: AppUsageData = [
            (ProviderId::Claude, usage(10.0, 20.0, None)),
            (ProviderId::Codex, usage(30.0, 40.0, None)),
            (ProviderId::Antigravity, usage(50.0, 60.0, None)),
        ]
        .into_iter()
        .collect();
        data.accounts = vec![
            account(
                ProviderId::Claude,
                named("default", "O"),
                Some(usage(10.0, 20.0, None)),
                true,
            ),
            account(
                ProviderId::Claude,
                named("claude_x", "X"),
                Some(usage(71.0, 72.0, None)),
                false,
            ),
            account(
                ProviderId::Codex,
                named("default", "O"),
                Some(usage(30.0, 40.0, None)),
                true,
            ),
            account(ProviderId::Codex, named("codex_x", "X"), None, false),
            account(
                ProviderId::Antigravity,
                AccountSettings::antigravity_a_profile(),
                Some(usage(50.0, 60.0, None)),
                true,
            ),
            account(
                ProviderId::Antigravity,
                named(ANTIGRAVITY_X_ID, "OpenCode"),
                Some(crate::models::UsageData {
                    external_pool: Some(Box::new(usage(91.0, 92.0, None))),
                    ..usage(81.0, 82.0, None)
                }),
                false,
            ),
        ];
        let providers = ProviderSet::from_enabled([
            ProviderId::Claude,
            ProviderId::Codex,
            ProviderId::Antigravity,
        ]);
        let widget = usage_summary_lines_with_side_pools(
            &data,
            providers,
            LanguageId::English,
            false,
            true,
            true,
            None,
            true,
        );
        let tray = usage_summary_lines(
            &data,
            providers,
            LanguageId::English,
            false,
            true,
            true,
            None,
        );
        assert_eq!(widget[..tray.len()], tray[..]);
        let side = &widget[tray.len()..];
        assert_eq!(side.len(), 4, "{side:?}");
        assert!(
            side[0].starts_with("Claude Code (X)") && side[0].contains("71%"),
            "{side:?}"
        );
        assert!(
            side[1].ends_with("(X): --") && side[1].starts_with("Codex"),
            "{side:?}"
        );
        assert!(
            side[2].contains("(OpenCode)") && side[2].contains("81%"),
            "{side:?}"
        );
        assert!(!tray
            .iter()
            .any(|line| line.contains("71%") || line.contains("81%") || line.contains("91%")));
        assert!(
            side[3].contains("(Claude/GPT, OpenCode)") && side[3].contains("91%"),
            "{side:?}"
        );
    }

    #[test]
    fn widget_summary_adds_precision_and_reset_times_without_changing_tray_text() {
        let now = UNIX_EPOCH + Duration::from_secs(1_000_000);
        let mut reading = usage(4.64, 42.37, Some("30d"));
        reading.session.resets_at = Some(now + Duration::from_secs(11_520));
        reading.weekly.resets_at = Some(now + Duration::from_secs(183_600));
        let data = [(ProviderId::Claude, reading)].into_iter().collect();
        let providers = ProviderSet::from_enabled([ProviderId::Claude]);
        assert_eq!(
            usage_summary_lines(
                &data,
                providers,
                LanguageId::English,
                false,
                true,
                true,
                Some(now)
            ),
            ["Claude Code 5h: 4.6% (3h 12m) | 30d: 42.4% (2d 3h)"]
        );
        assert_eq!(
            usage_summary_lines(
                &data,
                providers,
                LanguageId::English,
                true,
                true,
                true,
                Some(now)
            ),
            ["Claude Code 5h: 95.4% (3h 12m) | 30d: 57.6% (2d 3h)"]
        );
        assert_eq!(
            tray_usage_summary_lines(&data, providers, LanguageId::English, false),
            ["Claude Code 5h: 5% | 30d: 42%"]
        );
    }

    #[test]
    fn widget_summary_preserves_pool_selection_and_missing_windows() {
        let now = UNIX_EPOCH;
        let mut reading = usage(12.34, 56.78, None);
        reading.weekly = UsageSection::default();
        let data = [(ProviderId::Antigravity, reading)].into_iter().collect();
        let providers = ProviderSet::from_enabled([ProviderId::Antigravity]);
        let lines = usage_summary_lines(
            &data,
            providers,
            LanguageId::English,
            false,
            true,
            true,
            Some(now),
        );
        assert!(
            lines[0].ends_with("(Gemini) 5h: 12.3% | 7d: --"),
            "{lines:?}"
        );
        assert!(lines[1].ends_with("(Claude/GPT): --"), "{lines:?}");
        assert!(usage_summary_lines(
            &data,
            providers,
            LanguageId::English,
            false,
            false,
            false,
            Some(now)
        )
        .is_empty());
        assert!(usage_summary_lines(
            &data,
            ProviderSet::from_enabled([ProviderId::Claude]),
            LanguageId::English,
            false,
            true,
            true,
            Some(now)
        )
        .is_empty());
    }

    #[test]
    fn compact_tray_summary_shows_unavailable_windows_as_dashes() {
        let mut reading = usage(10.0, 20.0, Some("30d"));
        reading.weekly = UsageSection::default();
        let data = [(ProviderId::OpenCode, reading)].into_iter().collect();

        assert_eq!(
            tray_usage_summary_lines(
                &data,
                ProviderSet::from_enabled([ProviderId::OpenCode]),
                LanguageId::English,
                true,
            ),
            ["OpenCode 5h: 90% | 30d: --"]
        );
    }

    #[test]
    fn tray_summary_counts_down_when_the_widget_shows_what_is_left() {
        let data = [(ProviderId::Claude, usage(4.6, 42.4, None))]
            .into_iter()
            .collect();

        assert_eq!(
            tray_usage_summary_lines(
                &data,
                ProviderSet::from_enabled([ProviderId::Claude]),
                LanguageId::English,
                true,
            ),
            ["Claude Code 5h: 95% | 7d: 58%"]
        );
    }

    #[test]
    fn tray_summary_uses_provider_window_labels_and_selection() {
        let data = [
            (ProviderId::Claude, usage(10.0, 20.0, None)),
            (ProviderId::OpenCode, usage(30.0, 40.0, Some("30d"))),
        ]
        .into_iter()
        .collect();

        assert_eq!(
            tray_usage_summary_lines(
                &data,
                ProviderSet::from_enabled([ProviderId::OpenCode]),
                LanguageId::English,
                false,
            ),
            ["OpenCode 5h: 30% | 30d: 40%"]
        );
    }

    #[test]
    fn tray_summary_prefers_the_reported_session_window_label() {
        // Cursor's included-usage window is not the default five-hour session.
        let mut data = usage(7.1, 0.0, Some("API"));
        data.session_label = Some("7d".into());
        let data = [(ProviderId::Cursor, data)].into_iter().collect();

        assert_eq!(
            tray_usage_summary_lines(
                &data,
                ProviderSet::from_enabled([ProviderId::Cursor]),
                LanguageId::English,
                false,
            ),
            ["Cursor 7d: 7% | API: 0%"]
        );
        assert_eq!(
            usage_summary_lines(
                &data,
                ProviderSet::from_enabled([ProviderId::Cursor]),
                LanguageId::English,
                false,
                true,
                true,
                Some(UNIX_EPOCH),
            ),
            ["Cursor 7d: 7.1% | API: 0.0%"]
        );
    }
}
