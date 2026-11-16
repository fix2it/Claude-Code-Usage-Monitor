use super::*;
use crate::accounts::{fingerprint, AccountProfile, AccountSettings};
use crate::models::AccountUsage;
use std::path::PathBuf;

#[derive(Clone)]
struct Target {
    provider: ProviderId,
    profile: Option<AccountProfile>,
    path: Result<Option<PathBuf>, String>,
}

impl Target {
    fn signature(&self) -> String {
        if self.profile.is_none() {
            return String::new();
        }
        match &self.path {
            Ok(Some(path)) => account_source_signature(self.provider, path),
            Ok(None) if self.provider == ProviderId::Antigravity => antigravity_pool_a_signature(),
            Ok(None) => fingerprint(&format!(
                "{:?}",
                credential_watch_snapshot(CredentialWatchMode::ActiveSource(self.provider))
            )),
            Err(error) => fingerprint(error),
        }
    }
}

/// Pool A's source: its Credential Manager target. Tests never read the real
/// credential store, so they see one fixed source.
#[cfg(not(test))]
fn antigravity_pool_a_signature() -> String {
    fingerprint(&format!(
        "{:?}",
        credential_watch_snapshot(CredentialWatchMode::ActiveSource(ProviderId::Antigravity))
    ))
}

#[cfg(test)]
fn antigravity_pool_a_signature() -> String {
    "agy-pool-a-fixture".into()
}

pub(super) fn poll_accounts(
    enabled: ProviderSet,
    settings: &AccountSettings,
    previous: Option<&AppUsageData>,
    force: bool,
    on_progress: impl FnMut(AppUsageData),
) -> Result<AppUsageData, PollFailure> {
    poll_accounts_with_progress(
        enabled,
        settings,
        previous,
        force,
        |provider, path| match path {
            Some(path) => match provider {
                ProviderId::Claude => claude::poll_account(path),
                ProviderId::Codex => codex::poll_account(path),
                ProviderId::Antigravity => antigravity::poll_opencode_account(path),
                _ => poll_provider(provider),
            },
            None => poll_provider(provider),
        },
        on_progress,
    )
}

#[cfg(test)]
pub(super) fn poll_accounts_for_test<F>(
    enabled: ProviderSet,
    settings: &AccountSettings,
    poll: F,
) -> Result<AppUsageData, PollFailure>
where
    F: Fn(ProviderId, Option<&std::path::Path>) -> Result<UsageData, PollError> + Sync,
{
    poll_accounts_with(enabled, settings, poll)
}

#[cfg(test)]
fn poll_accounts_with<F>(
    enabled: ProviderSet,
    settings: &AccountSettings,
    poll: F,
) -> Result<AppUsageData, PollFailure>
where
    F: Fn(ProviderId, Option<&std::path::Path>) -> Result<UsageData, PollError> + Sync,
{
    poll_accounts_with_history(enabled, settings, None, false, poll)
}

#[cfg(test)]
fn poll_accounts_with_history<F>(
    enabled: ProviderSet,
    settings: &AccountSettings,
    previous: Option<&AppUsageData>,
    force: bool,
    poll: F,
) -> Result<AppUsageData, PollFailure>
where
    F: Fn(ProviderId, Option<&std::path::Path>) -> Result<UsageData, PollError> + Sync,
{
    poll_accounts_with_progress(enabled, settings, previous, force, poll, |_| {})
}

fn poll_accounts_with_progress<F>(
    enabled: ProviderSet,
    settings: &AccountSettings,
    previous: Option<&AppUsageData>,
    force: bool,
    poll: F,
    mut on_progress: impl FnMut(AppUsageData),
) -> Result<AppUsageData, PollFailure>
where
    F: Fn(ProviderId, Option<&std::path::Path>) -> Result<UsageData, PollError> + Sync,
{
    let mut targets = Vec::new();
    for provider in enabled.iter() {
        if let Some(accounts) = settings.get(provider) {
            for profile in accounts.profiles.iter().filter(|profile| profile.enabled) {
                let path = profile.credential_path(provider).map(|path| {
                    path.or_else(|| match provider {
                        ProviderId::Codex => codex::codex_auth_path(),
                        ProviderId::Claude => crate::accounts::environment_directory(provider)
                            .map(|directory| directory.join(".credentials.json"))
                            .or_else(claude::native_credential_path),
                        _ => None,
                    })
                });
                targets.push(Target {
                    provider,
                    profile: Some(profile.clone()),
                    path,
                });
            }
        } else if provider == ProviderId::Antigravity
            && settings.antigravity_x().is_some()
            && !settings.antigravity_a_enabled()
        {
            // Pool A is off: only pool O (pushed below) is polled.
        } else if provider == ProviderId::Antigravity && settings.antigravity_x().is_some() {
            // With pool O on, pool A is polled as an account of its own, the
            // way two Claude or Codex profiles are: each keeps its own error,
            // stale reading and login pause while the other keeps updating.
            targets.push(Target {
                provider,
                profile: Some(AccountSettings::antigravity_a_profile()),
                path: Ok(None),
            });
        } else {
            targets.push(Target {
                provider,
                profile: None,
                path: Ok(None),
            });
        }
        // The OpenCode pool is an extra account next to pool A.
        if provider == ProviderId::Antigravity {
            if let Some(profile) = settings.antigravity_x() {
                targets.push(Target {
                    provider,
                    profile: Some(profile.clone()),
                    path: profile.credential_path(provider),
                });
            }
        }
    }
    // Profiles sharing a source must not race token refresh or credit writes.
    // Each distinct source has one job, while unrelated accounts run concurrently.
    let mut groups: Vec<Vec<Target>> = Vec::new();
    for target in targets {
        if let Some(group) = groups.iter_mut().find(|group| {
            group[0].provider == target.provider
                && source_key(&group[0].path) == source_key(&target.path)
        }) {
            group.push(target);
        } else {
            groups.push(vec![target]);
        }
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut results = std::thread::scope(|scope| {
        let (sender, receiver) = std::sync::mpsc::channel();
        for _ in 0..groups.len().min(MAX_CONCURRENT_PROVIDER_POLLS) {
            let sender = sender.clone();
            let groups = &groups;
            let next = &next;
            let poll = &poll;
            scope.spawn(move || loop {
                let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(group) = groups.get(index) else {
                    break;
                };
                let target = &group[0];
                let mut signature = target.signature();
                let paused_error = if force {
                    None
                } else {
                    group.iter().find_map(|target| {
                        previous?.auth_error_for_source(
                            target.provider,
                            target.profile.as_ref()?,
                            &signature,
                        )
                    })
                };
                let mut result = Err(paused_error.unwrap_or(PollError::RequestFailed));
                diagnose::log_lazy(|| {
                    format!(
                        "{} account {} polling source={:?} paused={paused_error:?}",
                        target.provider.descriptor().display_name,
                        target
                            .profile
                            .as_ref()
                            .map(|profile| profile.name.as_str())
                            .unwrap_or("default"),
                        target.path
                    )
                });
                for _ in 0..if paused_error.is_some() { 0 } else { 2 } {
                    result = retry_after::with_manual_refresh(force, || match &target.path {
                        Err(_) => Err(PollError::NoCredentials),
                        Ok(path) => poll(target.provider, path.as_deref()),
                    });
                    let current_signature = target.signature();
                    if signature == current_signature {
                        break;
                    }
                    // Re-read once after login/token rotation; never associate
                    // a response (or a stale reading) with replaced credentials.
                    signature = current_signature;
                    result = Err(PollError::RequestFailed);
                }
                for target in group {
                    let _ = sender.send((index, target.clone(), signature.clone(), result.clone()));
                }
            });
        }
        drop(sender);
        receiver
            .into_iter()
            .inspect(|(_, target, signature, result)| {
                let mut update = AppUsageData::default();
                append_account_result(&mut update, target, signature, result);
                if !update.is_empty() || !update.accounts.is_empty() {
                    on_progress(update);
                }
            })
            .collect::<Vec<_>>()
    });
    results.sort_by_key(|(index, _, _, _)| *index);
    let mut data = AppUsageData::default();
    let mut first_error = None;
    for (_, target, signature, result) in results {
        if let Ok(usage) = &result {
            diagnose::log_lazy(|| {
                format!(
                    "{} account {} usage received: session={} weekly={} stale={}",
                    target.provider.descriptor().display_name,
                    target
                        .profile
                        .as_ref()
                        .map(|profile| profile.name.as_str())
                        .unwrap_or("default"),
                    usage.session.percentage,
                    usage.weekly.percentage,
                    usage.stale
                )
            });
        }
        if let Err(error) = &result {
            crate::diagnose::log_lazy(|| {
                format!(
                    "{} account {} usage poll failed: {error:?}",
                    target.provider.descriptor().display_name,
                    target
                        .profile
                        .as_ref()
                        .map(|profile| profile.name.as_str())
                        .unwrap_or("default")
                )
            });
            first_error.get_or_insert(PollFailure {
                provider: target.provider,
                error: *error,
            });
        }
        append_account_result(&mut data, &target, &signature, &result);
    }
    data.select_accounts(settings);
    // Account failures are data too: publish their status to the dashboard,
    // and keep polling other accounts instead of pausing the whole provider.
    match first_error {
        Some(error) if data.accounts.is_empty() && data.is_empty() => Err(error),
        _ => Ok(data),
    }
}

fn append_account_result(
    data: &mut AppUsageData,
    target: &Target,
    signature: &str,
    result: &Result<UsageData, PollError>,
) {
    if let Some(profile) = &target.profile {
        data.accounts.push(AccountUsage {
            provider: target.provider,
            profile: profile.clone(),
            source_signature: signature.to_string(),
            source_path: target.path.as_ref().ok().cloned().flatten(),
            usage: result.as_ref().ok().cloned(),
            error: result.as_ref().err().copied(),
            selected: false,
        });
    } else {
        match result {
            Ok(usage) => {
                data.insert(target.provider, usage.clone());
            }
            // A provider without profiles keeps its failure next to the
            // reading that may be carried in for it.
            Err(error) => data.set_provider_error(target.provider, *error),
        }
    }
}

fn source_key(path: &Result<Option<PathBuf>, String>) -> String {
    match path {
        Ok(Some(path)) => crate::accounts::source_key(path),
        other => format!("{other:?}"),
    }
}

pub(super) fn carry_accounts(fresh: &mut AppUsageData, previous: &AppUsageData) {
    for account in &mut fresh.accounts {
        if account.usage.is_some() {
            continue;
        }
        // Only transient failures may keep a reading, and only for the same
        // configured source and unchanged credentials file.
        if !account.error.is_some_and(PollError::is_transient) {
            continue;
        }
        let last_account = previous
            .accounts
            .iter()
            .find(|last| {
                last.provider == account.provider
                    && last.profile.same_source(&account.profile)
                    && last.source_signature == account.source_signature
            })
            .and_then(|last| last.usage.as_ref());
        // Pool A turned into an account when pool O was switched on: its last
        // reading is still the provider's own one.
        let last_pool_a = (account.is_antigravity_pool_a()
            && !previous
                .accounts
                .iter()
                .any(AccountUsage::is_antigravity_pool_a))
        // Pool O's stand-in reading (pool A was off) is never pool A's.
        .then(|| previous.antigravity_pool_a_reading())
        .flatten();
        if let Some(last) = last_account.or(last_pool_a) {
            let mut carried = last.clone();
            carried.stale = true;
            account.usage = Some(carried);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::file_signature;
    use crate::accounts::ProviderAccounts;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn settings() -> AccountSettings {
        AccountSettings {
            claude: ProviderAccounts {
                profiles: ["personal", "work"]
                    .into_iter()
                    .map(|name| AccountProfile {
                        id: name.into(),
                        name: name.into(),
                        config_dir: format!("C:\\account-tests\\{name}"),
                        ..Default::default()
                    })
                    .collect(),
                selected: "work".into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn usage(percentage: f64) -> UsageData {
        UsageData {
            session: UsageSection {
                available: true,
                percentage,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn each_claude_profile_keeps_its_own_fable_limit_through_polling_and_theme_bindings() {
        let fable = |percentage: f64| UsageData {
            limits: vec![crate::models::UsageLimit {
                key: "weekly_scoped_fable".into(),
                kind: "weekly_scoped".into(),
                label: "Fable".into(),
                model: Some("Fable".into()),
                is_active: true,
                usage: UsageSection {
                    available: true,
                    percentage,
                    resets_at: None,
                },
                ..Default::default()
            }],
            ..usage(10.0)
        };
        let mut settings = settings();
        settings.claude.profiles = ["default", "claude_x"]
            .into_iter()
            .map(|id| AccountProfile {
                id: id.into(),
                name: id.into(),
                config_dir: format!("C:\\account-tests\\fable-{id}"),
                ..Default::default()
            })
            .collect();
        settings.claude.selected = "default".into();
        let data = poll_accounts_with(ProviderSet::default(), &settings, |_, path| {
            let path = path.unwrap().to_string_lossy().into_owned();
            Ok(fable(if path.contains("claude_x") {
                81.0
            } else {
                43.0
            }))
        })
        .unwrap();
        for (id, expected) in [("default", 43.0), ("claude_x", 81.0)] {
            let account = data
                .accounts
                .iter()
                .find(|account| account.provider == ProviderId::Claude && account.profile.id == id)
                .unwrap();
            let limits = &account.usage.as_ref().unwrap().limits;
            assert_eq!(limits.len(), 1);
            assert_eq!(limits[0].usage.percentage, expected);
        }
        let context = crate::theme_engine::DataContext::from_usage(
            Some(&data),
            &crate::theme_engine::Canvas::default(),
        );
        assert_eq!(
            context.get("accounts.claude.default.model.fable.percentage"),
            Some(43.0)
        );
        assert_eq!(
            context.get("accounts.claude.claude_x.model.fable.percentage"),
            Some(81.0)
        );
    }

    #[test]
    fn agy_pool_o_is_polled_next_to_pool_a_without_replacing_it() {
        let mut settings = AccountSettings::default();
        let mut profile = AccountSettings::antigravity_x_profile();
        profile.credentials_path = "C:\\account-tests\\opencode.db".into();
        profile.enabled = true;
        settings.antigravity.profiles.push(profile);
        let calls = std::sync::Mutex::new(Vec::new());
        let data = poll_accounts_with(
            ProviderSet::from_enabled([ProviderId::Antigravity]),
            &settings,
            |provider, path| {
                assert_eq!(provider, ProviderId::Antigravity, "only agy is enabled");
                calls
                    .lock()
                    .unwrap()
                    .push(path.map(|path| path.to_path_buf()));
                Ok(usage(if path.is_some() { 70.0 } else { 15.0 }))
            },
        )
        .unwrap();
        let mut calls = calls.into_inner().unwrap();
        calls.sort();
        assert_eq!(
            calls,
            [None, Some(PathBuf::from("C:\\account-tests\\opencode.db"))]
        );
        assert_eq!(
            data.get(ProviderId::Antigravity)
                .unwrap()
                .session
                .percentage,
            15.0
        );
        let pool_o = data.antigravity_x().unwrap();
        assert!(!pool_o.selected);
        assert_eq!(pool_o.usage.as_ref().unwrap().session.percentage, 70.0);

        // Pool O failing alone does not hide pool A's reading.
        let ok = poll_accounts_with(
            ProviderSet::from_enabled([ProviderId::Antigravity]),
            &settings,
            |_, path| match path {
                Some(_) => Err(PollError::NetworkError),
                None => Ok(usage(15.0)),
            },
        )
        .unwrap();
        assert_eq!(
            ok.get(ProviderId::Antigravity).unwrap().session.percentage,
            15.0
        );

        // Turned off: only pool A is polled, exactly as before.
        settings.antigravity.profiles[0].enabled = false;
        let data = poll_accounts_with(
            ProviderSet::from_enabled([ProviderId::Antigravity]),
            &settings,
            |_, path| {
                assert!(path.is_none());
                Ok(usage(15.0))
            },
        )
        .unwrap();
        assert!(data.antigravity_x().is_none());
    }

    fn agy_pool_o_settings() -> AccountSettings {
        let mut settings = AccountSettings::default();
        let mut profile = AccountSettings::antigravity_x_profile();
        profile.credentials_path = "C:\\account-tests\\opencode.db".into();
        profile.enabled = true;
        settings.antigravity.profiles.push(profile);
        settings
    }

    fn agy_only() -> ProviderSet {
        ProviderSet::from_enabled([ProviderId::Antigravity])
    }

    #[test]
    fn agy_only_pool_a_login_failure_keeps_pool_o_live_and_shows_pool_a_error() {
        let settings = agy_pool_o_settings();
        let first = poll_accounts_with(agy_only(), &settings, |_, path| match path {
            Some(_) => Ok(usage(70.0)),
            None => Err(PollError::AuthRequired),
        })
        .unwrap();
        // Pool O's reading is published; pool A shows its own error.
        assert_eq!(
            first
                .antigravity_x()
                .unwrap()
                .usage
                .as_ref()
                .unwrap()
                .session
                .percentage,
            70.0
        );
        let pool_a = first
            .accounts
            .iter()
            .find(|account| account.is_antigravity_pool_a())
            .unwrap();
        assert_eq!(pool_a.error, Some(PollError::AuthRequired));
        assert!(pool_a.selected);
        assert!(first.get(ProviderId::Antigravity).is_none());
        let failures = first.new_auth_failures(None, false);
        assert_eq!(failures.len(), 1);
        assert!(failures[0].is_antigravity_pool_a());
        let context = crate::theme_engine::DataContext::from_usage(
            Some(&first),
            &crate::theme_engine::Canvas::default(),
        );
        assert_eq!(context.get("antigravity.has_error"), Some(1.0));

        // Next cycle: pool A stays paused (no new login), pool O is polled.
        let calls = std::sync::Mutex::new(Vec::new());
        let second =
            poll_accounts_with_history(agy_only(), &settings, Some(&first), false, |_, path| {
                calls.lock().unwrap().push(path.is_some());
                assert!(path.is_some(), "paused pool A must not refresh its token");
                Ok(usage(72.0))
            })
            .unwrap();
        assert_eq!(calls.into_inner().unwrap(), [true]);
        assert_eq!(
            second
                .antigravity_x()
                .unwrap()
                .usage
                .as_ref()
                .unwrap()
                .session
                .percentage,
            72.0
        );
        assert!(second.new_auth_failures(Some(&first), false).is_empty());
        assert_eq!(
            second
                .accounts
                .iter()
                .find(|account| account.is_antigravity_pool_a())
                .unwrap()
                .error,
            Some(PollError::AuthRequired)
        );
    }

    #[test]
    fn agy_only_pool_a_transient_failure_keeps_pool_o_and_a_stale_pool_a_reading() {
        let settings = agy_pool_o_settings();
        // The last reading before pool O was on: pool A as the provider.
        let mut before = AppUsageData::default();
        before.insert(ProviderId::Antigravity, usage(15.0));
        for previous in [before, {
            poll_accounts_with(agy_only(), &settings, |_, path| {
                Ok(usage(if path.is_some() { 60.0 } else { 15.0 }))
            })
            .unwrap()
        }] {
            let fresh = poll_accounts_with(agy_only(), &settings, |_, path| match path {
                Some(_) => Ok(usage(70.0)),
                None => Err(PollError::NetworkError),
            })
            .unwrap();
            let mut merged = carry_forward_failures(fresh, &previous, agy_only());
            merged.select_accounts(&settings);
            let pool_a = merged.get(ProviderId::Antigravity).unwrap();
            assert!(pool_a.stale);
            assert_eq!(pool_a.session.percentage, 15.0);
            assert_eq!(
                merged
                    .accounts
                    .iter()
                    .find(|account| account.is_antigravity_pool_a())
                    .unwrap()
                    .error,
                Some(PollError::NetworkError)
            );
            let pool_o = merged.antigravity_x().unwrap().usage.as_ref().unwrap();
            assert!(!pool_o.stale);
            assert_eq!(pool_o.session.percentage, 70.0);
        }
    }

    #[test]
    fn ready_accounts_are_published_while_another_provider_is_waiting() {
        let mut settings = settings();
        settings.claude.profiles.truncate(1);
        settings.claude.selected = "personal".into();
        settings.codex.profiles[0].config_dir = "C:\\account-tests\\codex".into();
        let (release, wait) = std::sync::mpsc::channel();
        let wait = std::sync::Mutex::new(wait);
        let mut visible = AppUsageData::default();
        let mut order = Vec::new();
        let final_data = poll_accounts_with_progress(
            ProviderSet::from_enabled([ProviderId::Claude, ProviderId::Codex]),
            &settings,
            None,
            false,
            |provider, _| {
                if provider == ProviderId::Claude {
                    wait.lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap();
                }
                Ok(usage(42.0))
            },
            |update| {
                let provider = update.accounts[0].provider;
                order.push(provider);
                visible = merge_poll_progress(update, &visible, &settings);
                if provider == ProviderId::Codex {
                    assert!(visible.get(ProviderId::Codex).is_some());
                    assert!(visible.get(ProviderId::Claude).is_none());
                    release.send(()).unwrap();
                }
            },
        )
        .unwrap();
        assert_eq!(order, [ProviderId::Codex, ProviderId::Claude]);
        for provider in [ProviderId::Claude, ProviderId::Codex] {
            assert_eq!(visible.get(provider), final_data.get(provider));
        }
    }

    #[test]
    fn partial_account_results_preserve_selection_and_error_rules() {
        let settings = settings();
        let previous =
            poll_accounts_with(ProviderSet::default(), &settings, |_, _| Ok(usage(25.0))).unwrap();
        let personal = previous
            .accounts
            .iter()
            .find(|account| account.profile.id == "personal")
            .unwrap()
            .clone();
        let work = previous
            .accounts
            .iter()
            .find(|account| account.profile.id == "work")
            .unwrap()
            .clone();
        let delta = |account| {
            let mut data = AppUsageData::default();
            data.accounts.push(account);
            data
        };
        // The unselected account finishing first must not replace the selected
        // account's old reading or display itself as the selected account.
        let mut refreshed = personal.clone();
        refreshed.usage = Some(usage(60.0));
        let merged = merge_poll_progress(delta(refreshed.clone()), &previous, &settings);
        assert_eq!(
            merged.get(ProviderId::Claude).unwrap().session.percentage,
            25.0
        );
        assert!(
            merge_poll_progress(delta(refreshed), &AppUsageData::default(), &settings)
                .get(ProviderId::Claude)
                .is_none()
        );
        for (error, changed, keeps_reading) in [
            (PollError::RequestFailed, false, true),
            (PollError::AuthRequired, false, false),
            (PollError::RequestFailed, true, false),
        ] {
            let mut failed = work.clone();
            failed.usage = None;
            failed.error = Some(error);
            if changed {
                failed.source_signature.push('x');
            }
            let update = merge_poll_progress(delta(failed), &merged, &settings);
            assert_eq!(update.accounts.len(), 2);
            assert_eq!(update.get(ProviderId::Claude).is_some(), keeps_reading);
            if keeps_reading {
                assert!(update.get(ProviderId::Claude).unwrap().stale);
            }
            assert_eq!(
                update
                    .accounts
                    .iter()
                    .find(|account| account.profile.id == "personal")
                    .unwrap()
                    .usage
                    .as_ref()
                    .unwrap()
                    .session
                    .percentage,
                60.0
            );
        }
    }

    #[test]
    fn auth_failures_pause_only_the_failed_account_and_notify_once() {
        let settings = settings();
        for error in [
            PollError::AuthRequired,
            PollError::TokenExpired,
            PollError::HttpStatus(401),
            PollError::HttpStatus(403),
        ] {
            let first = poll_accounts_with(ProviderSet::default(), &settings, |_, path| {
                if path.unwrap().to_string_lossy().contains("work") {
                    Err(error)
                } else {
                    Ok(usage(25.0))
                }
            })
            .unwrap();
            assert_eq!(first.new_auth_failures(None, false).len(), 1);
            let calls = AtomicUsize::new(0);
            let second = poll_accounts_with_history(
                ProviderSet::default(),
                &settings,
                Some(&first),
                false,
                |_, path| {
                    assert!(
                        !path.unwrap().to_string_lossy().contains("work"),
                        "paused account must not refresh its CLI token"
                    );
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(usage(30.0))
                },
            )
            .unwrap();
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert_eq!(
                second.accounts[0]
                    .usage
                    .as_ref()
                    .unwrap()
                    .session
                    .percentage,
                30.0
            );
            assert_eq!(second.accounts[1].error, Some(error));
            assert!(second.new_auth_failures(Some(&first), false).is_empty());

            let forced = poll_accounts_with_history(
                ProviderSet::default(),
                &settings,
                Some(&second),
                true,
                |_, _| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Err(error)
                },
            )
            .unwrap();
            assert_eq!(calls.load(Ordering::SeqCst), 3);
            assert_eq!(forced.new_auth_failures(Some(&second), true).len(), 2);
        }
    }

    #[test]
    fn all_accounts_can_wait_for_login_and_resume_after_credential_change() {
        let directory = std::env::temp_dir().join(format!(
            "usage-auth-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join(".credentials.json");
        std::fs::write(&path, "old fixture").unwrap();
        let mut settings = settings();
        settings.claude.profiles.truncate(1);
        settings.claude.profiles[0].config_dir = directory.to_string_lossy().into_owned();
        let first = poll_accounts_with(ProviderSet::default(), &settings, |_, _| {
            Err(PollError::TokenExpired)
        })
        .unwrap();
        assert!(first.is_empty());
        assert_eq!(first.new_auth_failures(None, false).len(), 1);
        let waiting = poll_accounts_with_history(
            ProviderSet::default(),
            &settings,
            Some(&first),
            false,
            |_, _| panic!("unchanged expired credentials must stay paused"),
        )
        .unwrap();
        assert!(waiting.new_auth_failures(Some(&first), false).is_empty());
        std::fs::write(&path, "new credentials fixture after login").unwrap();
        let resumed = poll_accounts_with_history(
            ProviderSet::default(),
            &settings,
            Some(&waiting),
            false,
            |_, _| Ok(usage(80.0)),
        )
        .unwrap();
        assert!(resumed.new_auth_failures(Some(&waiting), false).is_empty());
        assert_eq!(
            resumed.accounts[0]
                .usage
                .as_ref()
                .unwrap()
                .session
                .percentage,
            80.0
        );
        let failed_again = poll_accounts_with_history(
            ProviderSet::default(),
            &settings,
            Some(&resumed),
            false,
            |_, _| Err(PollError::AuthRequired),
        )
        .unwrap();
        assert_eq!(
            failed_again.new_auth_failures(Some(&resumed), false).len(),
            1
        );
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn failing_selected_account_never_borrows_another_accounts_usage() {
        let settings = settings();
        let data = poll_accounts_with(ProviderSet::default(), &settings, |_, path| {
            if path.unwrap().to_string_lossy().contains("work") {
                Err(PollError::AuthRequired)
            } else {
                Ok(usage(25.0))
            }
        })
        .unwrap();
        assert_eq!(data.accounts.len(), 2);
        assert!(data.get(ProviderId::Claude).is_none());
        assert_eq!(
            data.accounts[0].usage.as_ref().unwrap().session.percentage,
            25.0
        );
        assert_eq!(data.accounts[1].error, Some(PollError::AuthRequired));
        assert_eq!(data.selected_account_name(ProviderId::Claude), Some("work"));
    }

    #[test]
    fn stale_data_requires_the_same_profile_source_and_transient_error() {
        let settings = settings();
        let previous =
            poll_accounts_with(ProviderSet::default(), &settings, |_, _| Ok(usage(42.0))).unwrap();
        for (error, change_source, change_path, expected) in [
            (PollError::RequestFailed, false, false, true),
            (PollError::RequestFailed, true, false, false),
            (PollError::RequestFailed, false, true, false),
            (PollError::HttpStatus(429), false, false, true),
            (PollError::HttpStatus(503), false, false, true),
            (PollError::HttpStatus(429), true, false, false),
            (PollError::HttpStatus(429), false, true, false),
            (PollError::HttpStatus(401), false, false, false),
            (PollError::HttpStatus(403), false, false, false),
            (PollError::AuthRequired, false, false, false),
            (PollError::TokenExpired, false, false, false),
            (PollError::NoCredentials, false, false, false),
        ] {
            let mut fresh =
                poll_accounts_with(ProviderSet::default(), &settings, |_, _| Err(error)).unwrap();
            if change_source {
                fresh.accounts[1].source_signature.push('x');
            }
            if change_path {
                fresh.accounts[1].profile.config_dir.push('x');
            }
            carry_accounts(&mut fresh, &previous);
            assert_eq!(fresh.accounts[1].usage.is_some(), expected);
            if expected {
                assert!(fresh.accounts[1].usage.as_ref().unwrap().stale);
            }
        }
    }

    #[test]
    fn http_failure_survives_cache_and_clears_after_success() {
        let settings = settings();
        let failed = poll_accounts_with(ProviderSet::default(), &settings, |_, _| {
            Err(PollError::HttpStatus(429))
        })
        .unwrap();
        let cache = serde_json::to_string(&failed).unwrap();
        let failed: AppUsageData = serde_json::from_str(&cache).unwrap();
        assert_eq!(failed.accounts[0].error, Some(PollError::HttpStatus(429)));
        assert!(failed.accounts[0].usage.is_none());
        let calls = AtomicUsize::new(0);
        let recovered = poll_accounts_with_history(
            ProviderSet::default(),
            &settings,
            Some(&failed),
            false,
            |_, _| {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(usage(35.0))
            },
        )
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(recovered
            .accounts
            .iter()
            .all(|account| account.error.is_none()));
        assert_eq!(
            recovered
                .get(ProviderId::Claude)
                .unwrap()
                .session
                .percentage,
            35.0
        );
        assert_eq!(
            serde_json::from_str::<PollError>("\"request_failed\"").unwrap(),
            PollError::RequestFailed
        );
    }

    #[test]
    fn cache_round_trip_selection_removal_and_path_changes_are_isolated() {
        let mut settings = settings();
        let mut data = poll_accounts_with(ProviderSet::default(), &settings, |_, path| {
            Ok(usage(if path.unwrap().to_string_lossy().contains("work") {
                80.0
            } else {
                20.0
            }))
        })
        .unwrap();
        let json = serde_json::to_string(&data).unwrap();
        data = serde_json::from_str(&json).unwrap();
        assert_eq!(
            data.get(ProviderId::Claude).unwrap().session.percentage,
            80.0
        );
        settings.claude.selected = "personal".into();
        data.select_accounts(&settings);
        assert_eq!(
            data.get(ProviderId::Claude).unwrap().session.percentage,
            20.0
        );
        settings.claude.profiles[0].config_dir = "C:\\a-different-account".into();
        data.select_accounts(&settings);
        assert!(data.get(ProviderId::Claude).is_none());
        assert_eq!(data.accounts.len(), 1);
        settings.claude.profiles.clear();
        data.select_accounts(&settings);
        assert!(data.is_empty());
        assert!(data.accounts.is_empty());
    }

    #[test]
    fn invalid_or_disabled_profiles_make_no_requests() {
        let mut settings = settings();
        settings.claude.profiles[0].config_dir.clear();
        settings.claude.profiles[1].enabled = false;
        let data = poll_accounts_with(ProviderSet::default(), &settings, |_, _| {
            panic!("invalid profile polled")
        })
        .unwrap();
        assert_eq!(data.accounts.len(), 1);
        assert_eq!(data.accounts[0].error, Some(PollError::NoCredentials));
        assert!(data.accounts[0].usage.is_none());
    }

    #[test]
    fn account_concurrency_is_bounded_and_duplicate_sources_are_polled_once() {
        let mut settings = settings();
        for index in 2..6 {
            settings.claude.profiles.push(AccountProfile {
                id: format!("profile_{index}"),
                config_dir: format!("C:\\account-tests\\{index}"),
                ..Default::default()
            });
        }
        let mut duplicate = settings.claude.profiles[0].clone();
        duplicate.id = "alias".into();
        duplicate.config_dir = duplicate.config_dir.to_uppercase();
        settings.claude.profiles.push(duplicate);
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let calls = AtomicUsize::new(0);
        let data = poll_accounts_with(ProviderSet::default(), &settings, |_, _| {
            calls.fetch_add(1, Ordering::SeqCst);
            let current = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(current, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(20));
            active.fetch_sub(1, Ordering::SeqCst);
            Ok(usage(0.0))
        })
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 6);
        assert_eq!(data.accounts.len(), 7);
        assert!(peak.load(Ordering::SeqCst) > 1);
        assert!(peak.load(Ordering::SeqCst) <= MAX_CONCURRENT_PROVIDER_POLLS);
    }

    #[test]
    fn credential_rotation_during_poll_retries_before_publishing() {
        let directory = std::env::temp_dir().join(format!(
            "usage-account-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join(".credentials.json");
        std::fs::write(&path, "fixture before rotation").unwrap();
        let mut settings = settings();
        settings.claude.profiles.truncate(1);
        settings.claude.profiles[0].config_dir = directory.to_string_lossy().into_owned();
        let calls = AtomicUsize::new(0);
        let data = poll_accounts_with(ProviderSet::default(), &settings, |_, _| {
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                std::fs::write(&path, "fixture after token rotation, different length").unwrap();
                Ok(usage(10.0))
            } else {
                Ok(usage(80.0))
            }
        })
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            data.get(ProviderId::Claude).unwrap().session.percentage,
            80.0
        );
        assert_eq!(data.accounts[0].source_signature, file_signature(&path));
        let mut cached = data;
        std::fs::write(&path, "another login").unwrap();
        cached.invalidate_changed_credentials();
        cached.select_accounts(&settings);
        assert!(cached.get(ProviderId::Claude).is_none());
        assert!(cached.accounts[0].usage.is_none());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
