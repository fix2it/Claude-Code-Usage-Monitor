use super::*;
use crate::models::{limit_slug, UsageData, UsageLimit};

impl DataContext {
    pub(super) fn insert_limits(&mut self, name: &str, usage: Option<&UsageData>, countdown: bool) {
        let limits = usage
            .map(|usage| usage.limits.as_slice())
            .unwrap_or_default();
        self.insert(&format!("{name}.limits.count"), limits.len() as f64);
        for limit in limits {
            self.insert_limit(&format!("{name}.limits.{}", limit.key), limit, countdown);
        }
        // The model shortcut means a weekly model cap. Other kinds remain
        // individually addressable through limits.<key>, avoiding ambiguity.
        let mut models = std::collections::BTreeMap::<String, Vec<&UsageLimit>>::new();
        for limit in limits.iter().filter(|limit| {
            matches!(
                limit.kind.as_str(),
                "weekly_scoped" | "seven_day_opus" | "seven_day_sonnet"
            )
        }) {
            if let Some(model) = &limit.model {
                models.entry(limit_slug(model)).or_default().push(limit);
            }
        }
        for (model, candidates) in models {
            // Never select an arbitrary quota when two distinct scopes share
            // a model name. The full limit keys remain available in Studio.
            if let [limit] = candidates.as_slice() {
                self.insert_limit(&format!("{name}.model.{model}"), limit, countdown);
            }
        }
        let active: Vec<_> = limits
            .iter()
            .filter(|limit| limit.is_active && (limit.scope.is_some() || limit.model.is_some()))
            .collect();
        // Preserve ambiguity rather than guessing which active cap binds.
        if let [limit] = active.as_slice() {
            self.insert_limit(&format!("{name}.scoped"), limit, countdown);
        }
    }

    fn insert_limit(&mut self, base: &str, limit: &UsageLimit, countdown: bool) {
        let percentage = limit.usage.percentage;
        for (metric, value) in [
            ("available", limit.usage.available as u8 as f64),
            ("percentage", percentage),
            ("remaining", 100.0 - percentage),
            (
                "display",
                if countdown {
                    100.0 - percentage
                } else {
                    percentage
                },
            ),
            ("is_active", limit.is_active as u8 as f64),
        ] {
            self.insert(&format!("{base}.{metric}"), value);
        }
        for (field, value) in [
            ("key", limit.key.as_str()),
            ("label", limit.label.as_str()),
            ("kind", limit.kind.as_str()),
            ("model_id", limit.model_id.as_deref().unwrap_or("")),
        ] {
            self.insert_string(&format!("{base}.{field}"), value);
        }
        // Pace: the share of the window an even rate would have spent by now.
        // Only weekly caps have a known length; any other kind has no tick.
        let expected = (limit.kind.starts_with("weekly") || limit.kind.starts_with("seven_day"))
            .then(|| {
                limit
                    .usage
                    .available
                    .then(|| limit.usage.resets_at.and_then(|at| expected_used(at, WEEK)))
                    .flatten()
            })
            .flatten();
        self.insert(
            &format!("{base}.expected_available"),
            expected.is_some() as u8 as f64,
        );
        let expected = expected.unwrap_or(0.0);
        self.insert(&format!("{base}.expected"), expected);
        self.insert(
            &format!("{base}.expected_display"),
            if countdown {
                100.0 - expected
            } else {
                expected
            },
        );
        self.insert_string(
            &format!("{base}.scope"),
            limit
                .scope
                .as_ref()
                .map(serde_json::Value::to_string)
                .unwrap_or_default(),
        );
        let unix = limit
            .usage
            .resets_at
            .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs_f64())
            .unwrap_or(0.0);
        let seconds = limit
            .usage
            .resets_at
            .and_then(|at| at.duration_since(std::time::SystemTime::now()).ok())
            .map(|duration| duration.as_secs_f64())
            .unwrap_or(0.0);
        for (unit, value) in [
            ("unix", unix),
            ("seconds", seconds),
            ("minutes", seconds / 60.0),
            ("hours", seconds / 3600.0),
            ("days", seconds / 86400.0),
        ] {
            self.insert(&format!("{base}.reset.{unit}"), value);
        }
    }

    /// Valid dynamic quota paths still validate when the account is offline or
    /// no longer reports the quota. Unknown fields continue to catch typos.
    pub(super) fn limit_field(name: &str) -> Option<(&str, &str)> {
        let (owner, field) = if let Some(rest) = name.strip_prefix("accounts.") {
            let (provider, rest) = rest.split_once('.')?;
            let (id, field) = rest.split_once('.')?;
            if !matches!(provider, "claude" | "codex") || !valid_key(id) {
                return None;
            }
            (&name[..name.len() - field.len() - 1], field)
        } else {
            let (owner, field) = name.split_once('.')?;
            if !matches!(
                owner,
                "claude"
                    | "claude_x"
                    | "codex"
                    | "codex_x"
                    | "antigravity"
                    | "opencode"
                    | "cursor"
                    | "grok"
                    | "copilot"
                    | "qoder"
                    | "active"
            ) {
                return None;
            }
            (owner, field)
        };
        if let Some(field) = field.strip_prefix("scoped.") {
            return Some((owner, field));
        }
        let rest = field
            .strip_prefix("limits.")
            .or_else(|| field.strip_prefix("model."))?;
        let (key, field) = rest.split_once('.')?;
        valid_key(key).then_some((owner, field))
    }

    pub(super) fn limit_default(&self, name: &str) -> Option<f64> {
        let (_, field) = Self::limit_field(name)?;
        match field {
            "available" | "percentage" | "is_active" | "reset.unix" | "reset.seconds"
            | "reset.minutes" | "reset.hours" | "reset.days" | "expected_available"
            | "expected" => Some(0.0),
            "remaining" => Some(100.0),
            "display" | "expected_display" => Some(
                if self.values.get("display.countdown").copied().unwrap_or(0.0) != 0.0 {
                    100.0
                } else {
                    0.0
                },
            ),
            _ => None,
        }
    }

    pub(super) fn limit_string_default(name: &str) -> Option<&'static str> {
        let (_, field) = Self::limit_field(name)?;
        matches!(field, "key" | "label" | "kind" | "model_id" | "scope").then_some("")
    }

    #[cfg(test)]
    pub fn limit_variables(&self) -> Vec<&str> {
        let mut keys: Vec<_> = self
            .values
            .keys()
            .chain(self.strings.keys())
            .map(String::as_str)
            .filter(|key| Self::limit_field(key).is_some() || key.ends_with(".limits.count"))
            .collect();
        keys.sort_unstable();
        keys
    }
}

fn valid_key(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AccountUsage, UsageSection};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn limit() -> UsageLimit {
        UsageLimit {
            key: "weekly_scoped_fable".into(),
            kind: "weekly_scoped".into(),
            label: "Fable".into(),
            model: Some("Fable".into()),
            is_active: true,
            usage: UsageSection {
                available: true,
                percentage: 91.0,
                resets_at: Some(SystemTime::now() + Duration::from_secs(7200)),
            },
            ..Default::default()
        }
    }

    #[test]
    fn account_limits_are_independent_and_missing_fields_have_typed_defaults() {
        let mut data = AppUsageData::default();
        for (id, percentage) in [("personal", 91.0), ("work", 12.0)] {
            let mut limit = limit();
            limit.usage.percentage = percentage;
            data.accounts.push(AccountUsage {
                provider: ProviderId::Claude,
                profile: crate::accounts::AccountProfile {
                    id: id.into(),
                    name: id.into(),
                    enabled: true,
                    ..Default::default()
                },
                source_signature: String::new(),
                source_path: None,
                usage: Some(UsageData {
                    limits: vec![limit],
                    ..Default::default()
                }),
                error: None,
                selected: id == "personal",
            });
        }
        let context = DataContext::from_usage(Some(&data), &Canvas::default());
        assert_eq!(
            context.get("accounts.claude.personal.model.fable.percentage"),
            Some(91.0)
        );
        assert_eq!(
            context.get("accounts.claude.work.model.fable.percentage"),
            Some(12.0)
        );
        assert_eq!(
            format_template("{accounts.claude.work.model.fable:usage_badge}", &context),
            "12%"
        );
        assert!(
            context
                .get("accounts.claude.work.model.fable.reset.seconds")
                .unwrap()
                > 7100.0
        );
        assert!(context
            .limit_variables()
            .contains(&"accounts.claude.work.limits.weekly_scoped_fable.label"));

        for data in [None, Some(&AppUsageData::default())] {
            let context = DataContext::from_usage_with_runtime(
                data,
                &Canvas::default(),
                ThemeRuntime::default().with_countdown(true),
            );
            for base in [
                "claude.model.future_model",
                "claude.limits.weekly_scoped_future_model",
                "claude.scoped",
                "accounts.claude.deleted.model.fable",
                "accounts.claude.work.scoped",
            ] {
                assert_eq!(
                    evaluate(&format!("{base}.available"), &context).unwrap(),
                    0.0
                );
                assert_eq!(
                    evaluate(&format!("{base}.remaining"), &context).unwrap(),
                    100.0
                );
                assert_eq!(
                    evaluate(&format!("{base}.display"), &context).unwrap(),
                    100.0
                );
                assert_eq!(
                    evaluate(&format!("{base}.reset.unix"), &context).unwrap(),
                    0.0
                );
                assert_eq!(format_template(&format!("{{{base}.label}}"), &context), "");
                assert!(evaluate(&format!("{base}.percentge"), &context).is_err());
            }
            assert!(evaluate("claud.model.fable.percentage", &context).is_err());
        }
    }

    fn fable_usage(percentage: Option<f64>) -> UsageData {
        let mut limits = vec![UsageLimit {
            key: "weekly_all".into(),
            kind: "weekly_all".into(),
            label: "weekly all".into(),
            usage: UsageSection {
                available: true,
                percentage: 7.0,
                resets_at: None,
            },
            ..Default::default()
        }];
        if let Some(percentage) = percentage {
            let mut fable = limit();
            fable.usage.percentage = percentage;
            limits.push(fable);
        }
        UsageData {
            limits,
            ..Default::default()
        }
    }

    fn claude_account(id: &str, usage: UsageData, selected: bool) -> AccountUsage {
        AccountUsage {
            provider: ProviderId::Claude,
            profile: crate::accounts::AccountProfile {
                id: id.into(),
                name: id.into(),
                enabled: true,
                ..Default::default()
            },
            source_signature: String::new(),
            source_path: None,
            usage: Some(usage),
            error: None,
            selected,
        }
    }

    /// Mirrors what the poller leaves behind: the selected account's usage is
    /// also the legacy `claude` provider entry.
    fn claude_profiles(accounts: Vec<AccountUsage>) -> AppUsageData {
        let mut data = AppUsageData::default();
        if let Some(selected) = accounts.iter().find(|account| account.selected) {
            data.insert(ProviderId::Claude, selected.usage.clone().unwrap());
        }
        data.accounts = accounts;
        data
    }

    fn fable_bindings<'a>(context: &'a DataContext, profile: &str) -> Vec<&'a str> {
        let prefix = format!("accounts.claude.{profile}.model.fable.");
        context
            .limit_variables()
            .into_iter()
            .filter(|key| key.starts_with(&prefix))
            .collect()
    }

    #[test]
    fn both_claude_profiles_expose_their_own_fable_bindings() {
        let data = claude_profiles(vec![
            claude_account("default", fable_usage(Some(43.0)), true),
            claude_account("claude_x", fable_usage(Some(81.0)), false),
        ]);
        let context = DataContext::from_usage(Some(&data), &Canvas::default());
        for (profile, percentage) in [("default", 43.0), ("claude_x", 81.0)] {
            assert!(
                !fable_bindings(&context, profile).is_empty(),
                "{profile} bindings"
            );
            let base = format!("accounts.claude.{profile}.model.fable");
            assert_eq!(context.get(&format!("{base}.percentage")), Some(percentage));
            assert_eq!(context.get(&format!("{base}.available")), Some(1.0));
            assert_eq!(
                context.get(&format!("{base}.remaining")),
                Some(100.0 - percentage)
            );
            assert_eq!(context.get_string(&format!("{base}.label")), Some("Fable"));
        }
    }

    #[test]
    fn single_claude_profile_has_no_claude_x_fable_bindings() {
        let data = claude_profiles(vec![claude_account(
            "default",
            fable_usage(Some(43.0)),
            true,
        )]);
        let context = DataContext::from_usage(Some(&data), &Canvas::default());
        assert!(!fable_bindings(&context, "default").is_empty());
        assert_eq!(
            context.get("accounts.claude.default.model.fable.percentage"),
            Some(43.0)
        );
        assert!(fable_bindings(&context, "claude_x").is_empty());
        assert_eq!(
            context.get("accounts.claude.claude_x.model.fable.available"),
            Some(0.0)
        );
    }

    #[test]
    fn only_claude_x_profile_leaves_the_default_profile_without_fable_bindings() {
        let data = claude_profiles(vec![claude_account(
            "claude_x",
            fable_usage(Some(81.0)),
            true,
        )]);
        let context = DataContext::from_usage(Some(&data), &Canvas::default());
        assert!(!fable_bindings(&context, "claude_x").is_empty());
        assert_eq!(
            context.get("accounts.claude.claude_x.model.fable.percentage"),
            Some(81.0)
        );
        assert!(fable_bindings(&context, "default").is_empty());
        assert_eq!(
            context.get("accounts.claude.default.model.fable.percentage"),
            Some(0.0)
        );
    }

    #[test]
    fn legacy_entry_without_claude_accounts_still_feeds_the_default_fallback() {
        let data = AppUsageData::from_iter([(ProviderId::Claude, fable_usage(Some(43.0)))]);
        let context = DataContext::from_usage(Some(&data), &Canvas::default());
        assert!(!fable_bindings(&context, "default").is_empty());
        assert_eq!(
            context.get("accounts.claude.default.model.fable.percentage"),
            Some(43.0)
        );
    }

    #[test]
    fn profile_without_fable_does_not_borrow_the_default_profile_value() {
        let data = claude_profiles(vec![
            claude_account("default", fable_usage(Some(43.0)), true),
            claude_account("claude_x", fable_usage(None), false),
        ]);
        let context = DataContext::from_usage(Some(&data), &Canvas::default());
        assert!(fable_bindings(&context, "claude_x").is_empty());
        assert_eq!(
            context.get("accounts.claude.claude_x.model.fable.percentage"),
            Some(0.0)
        );
        assert_eq!(
            context.get("accounts.claude.claude_x.model.fable.available"),
            Some(0.0)
        );
        assert_eq!(
            context.get("accounts.claude.claude_x.limits.count"),
            Some(1.0)
        );
        assert_eq!(
            context.get("accounts.claude.default.model.fable.percentage"),
            Some(43.0)
        );
    }

    #[test]
    fn ambiguous_and_absent_quotas_never_choose_an_arbitrary_model_or_active_cap() {
        let first = limit();
        let mut second = first.clone();
        second.key.push_str("_other");
        second.model_id = Some("other-account-scope".into());
        let data = AppUsageData::from_iter([(
            ProviderId::Claude,
            UsageData {
                limits: vec![first, second],
                ..Default::default()
            },
        )]);
        let context = DataContext::from_usage(Some(&data), &Canvas::default());
        assert_eq!(context.get("claude.model.fable.available"), Some(0.0));
        assert_eq!(context.get("claude.scoped.available"), Some(0.0));
        assert_eq!(context.get("claude.limits.count"), Some(2.0));
        assert_eq!(
            format_template("{claude.model.missing:usage_line}", &context),
            "--"
        );
        assert_eq!(
            context.get("claude.limits.weekly_scoped_fable_other.available"),
            Some(1.0)
        );
    }

    #[test]
    fn unrecognized_limits_do_not_change_builtin_rendering_or_headlines() {
        let plain_usage = UsageData {
            session: UsageSection {
                available: true,
                percentage: 29.0,
                resets_at: None,
            },
            weekly: UsageSection {
                available: true,
                percentage: 26.0,
                resets_at: None,
            },
            ..Default::default()
        };
        let plain = AppUsageData::from_iter([(ProviderId::Claude, plain_usage.clone())]);
        let mut extra_limit = limit();
        extra_limit.key = "weekly_scoped_future".into();
        extra_limit.label = "Future".into();
        extra_limit.model = Some("Future".into());
        let extra = AppUsageData::from_iter([(
            ProviderId::Claude,
            UsageData {
                limits: vec![extra_limit],
                ..plain_usage
            },
        )]);
        for (_, source) in BUILTIN_THEME_SOURCES {
            let mut theme: ThemeDocument = serde_json::from_str(source).unwrap();
            theme.prepare_runtime();
            for countdown in [false, true] {
                let runtime = ThemeRuntime::new(true, false, false).with_countdown(countdown);
                for index in 0..theme.surfaces.len() {
                    let before =
                        render_theme_surface_with_runtime(&theme, index, Some(&plain), runtime);
                    let after =
                        render_theme_surface_with_runtime(&theme, index, Some(&extra), runtime);
                    assert_eq!((before.width, before.height), (after.width, after.height));
                    assert_eq!(
                        before.pixels, after.pixels,
                        "{} surface {index}",
                        theme.name
                    );
                    assert_eq!(before.warnings, after.warnings);
                }
            }
        }
        let context = DataContext::from_usage(Some(&extra), &Canvas::default());
        assert_eq!(context.get("claude.headline.percentage"), Some(29.0));
    }

    #[test]
    fn additional_resets_and_stale_quota_caches_follow_existing_poll_rules() {
        let mut limit = limit();
        limit.usage.resets_at = Some(UNIX_EPOCH);
        let usage = UsageData {
            limits: vec![limit],
            ..Default::default()
        };
        assert!(crate::poller::is_past_reset(&usage));
        let previous = AppUsageData::from_iter([(ProviderId::Claude, usage)]);
        let carried = crate::poller::carry_forward_failures(
            AppUsageData::default(),
            &previous,
            ProviderSet::from_enabled([ProviderId::Claude]),
        );
        assert_eq!(
            carried.get(ProviderId::Claude).unwrap().limits,
            previous.get(ProviderId::Claude).unwrap().limits
        );
        assert!(!crate::poller::app_is_past_reset(&carried));
        let mut account = AccountUsage {
            provider: ProviderId::Claude,
            profile: Default::default(),
            source_signature: String::new(),
            source_path: None,
            usage: carried.get(ProviderId::Claude).cloned(),
            error: Some(crate::poller::PollError::UnexpectedResponse),
            selected: true,
        };
        let restored: AccountUsage =
            serde_json::from_str(&serde_json::to_string(&account).unwrap()).unwrap();
        assert_eq!(restored, account);
        account.error = None;
        assert_ne!(restored, account);
        let old: UsageData = serde_json::from_str(r#"{"session":{"percentage":0,"resets_at":null},"weekly":{"percentage":0,"resets_at":null}}"#).unwrap();
        assert!(old.limits.is_empty());
    }
}
