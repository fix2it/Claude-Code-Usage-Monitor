use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const GITHUB_API_ACCEPT: &str = "application/vnd.github+json";
const GITHUB_API_VERSION: &str = "2022-11-28";
/// Files the pre-2.23.603 helper-based updater left in
/// `%LOCALAPPDATA%\ClaudeCodeUsageMonitor\updates`; removed on startup.
const LEGACY_UPDATE_FILES: [&str; 3] = [
    "updater-helper.exe",
    "update-download.exe",
    "update-download.exe.part",
];
/// How long startup keeps trying to delete `<exe>.old` while the previous
/// process is still exiting (it holds the image mapped until it is gone).
const LEFTOVER_CLEANUP_ATTEMPTS: u32 = 40;
const LEFTOVER_CLEANUP_DELAY: Duration = Duration::from_millis(500);

mod download;
mod release;

#[cfg(test)]
use download::AssetIntegrity;
use download::{download_release_asset, open_verified_source};
pub use release::ReleaseDescriptor;
use release::{release_descriptor, GitHubRelease};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallChannel {
    Portable,
}

#[derive(Debug)]
pub enum UpdateCheckResult {
    UpToDate,
    Available(AvailableUpdate),
}

#[derive(Clone, Debug)]
pub enum AvailableUpdate {
    /// Portable installs download this GitHub release asset.
    Release(ReleaseDescriptor),
}

impl AvailableUpdate {
    /// The version this update installs.
    pub fn version(&self) -> &str {
        match self {
            Self::Release(release) => &release.latest_version,
        }
    }
}

pub fn current_install_channel() -> InstallChannel {
    // Fork builds always update from the fork's GitHub Releases, even when the
    // executable sits under a WinGet packages folder, so the upstream WinGet
    // package can never replace them.
    InstallChannel::Portable
}

pub fn check_for_updates(channel: InstallChannel) -> Result<UpdateCheckResult, String> {
    check_channel_updates(channel, fetch_latest_release)
}

fn check_channel_updates(
    channel: InstallChannel,
    github: impl FnOnce() -> Result<Option<ReleaseDescriptor>, String>,
) -> Result<UpdateCheckResult, String> {
    match channel {
        InstallChannel::Portable => Ok(match github()? {
            Some(release) => UpdateCheckResult::Available(AvailableUpdate::Release(release)),
            None => UpdateCheckResult::UpToDate,
        }),
    }
}

/// Downloads the release next to the running exe, swaps it in by renaming
/// and starts it. On `Ok` the caller must exit so the new process can take
/// over the single-instance mutex; on `Err` the installed exe is in place.
///
/// No helper copy and no hidden child process: Windows allows renaming a
/// running executable, so `<exe>` becomes `<exe>.old` and the verified
/// `<exe>.new` becomes `<exe>`. The next start deletes `<exe>.old`.
pub fn begin_self_update(release: &ReleaseDescriptor) -> Result<(), String> {
    let current_exe =
        std::env::current_exe().map_err(|e| format!("Unable to locate current executable: {e}"))?;
    ensure_target_location_writable(&current_exe)?;

    let staged = staged_path_for(&current_exe);
    let partial = partial_path_for(&current_exe);
    for stale in [&staged, &partial] {
        if stale.exists() {
            std::fs::remove_file(stale)
                .map_err(|e| format!("Unable to remove {}: {e}", stale.display()))?;
        }
    }

    download_release_asset(release, &partial, &staged)?;
    // Re-check the staged bytes right before the swap. The handle denies
    // writers meanwhile and is released so the file can be renamed.
    let swapped = open_verified_source(&staged, &release.integrity).and_then(|handle| {
        drop(handle);
        swap_in_staged_binary(&current_exe, &staged)
    });
    if let Err(error) = swapped {
        let _ = std::fs::remove_file(&staged);
        return Err(error);
    }

    if let Err(error) = launch_updated_binary(&current_exe) {
        return Err(match rollback_swap(&current_exe, &staged) {
            Ok(()) => {
                let _ = std::fs::remove_file(&staged);
                format!(
                    "Unable to start the updated app: {error}. The previous version was restored."
                )
            }
            Err(rollback_error) => {
                format!("Unable to start the updated app: {error}. {rollback_error}")
            }
        });
    }
    Ok(())
}

/// `target` -> `target.old`, then `staged` -> `target`. On failure `target`
/// holds the previously installed exe again (or the error says where it is).
fn swap_in_staged_binary(target: &Path, staged: &Path) -> Result<(), String> {
    let backup = backup_path_for(target);
    if backup.exists() {
        // The backup may be the only copy of the installed exe (an earlier
        // restore failed); drop it only while the target itself is in place.
        if !target.exists() {
            return Err(format!(
                "Unable to update {}. The previous version is kept at {}.",
                target.display(),
                backup.display()
            ));
        }
        std::fs::remove_file(&backup).map_err(|e| {
            format!(
                "Unable to remove the leftover {} from an earlier update: {e}",
                backup.display()
            )
        })?;
    }

    std::fs::rename(target, &backup)
        .map_err(|e| format!("Unable to move the running app aside: {e}"))?;

    if let Err(error) = std::fs::rename(staged, target) {
        return Err(match std::fs::rename(&backup, target) {
            Ok(()) => format!("Unable to install the update: {error}"),
            Err(restore_error) => format!(
                "Unable to install the update: {error}. Restoring the previous version \
                 failed ({restore_error}); it is kept at {}.",
                backup.display()
            ),
        });
    }
    Ok(())
}

/// Undoes [`swap_in_staged_binary`]: the new exe goes back to `staged`, the
/// backup back to `target`.
fn rollback_swap(target: &Path, staged: &Path) -> Result<(), String> {
    let backup = backup_path_for(target);
    std::fs::rename(target, staged)
        .and_then(|()| std::fs::rename(&backup, target))
        .map_err(|e| {
            format!(
                "Restoring the previous version failed ({e}); it is kept at {}.",
                backup.display()
            )
        })
}

/// Starts the updated exe as an ordinary process with this process's
/// arguments (keeps `--allow-multiple`, `--diagnose`). `CCUM_RELAUNCH` makes
/// it wait for this process to release the single-instance mutex.
fn launch_updated_binary(target: &Path) -> Result<(), String> {
    let mut command = Command::new(target);
    command.args(std::env::args_os().skip(1));
    if let Some(parent) = target.parent() {
        command.current_dir(parent);
    }
    command
        .env(crate::window::ENV_RELAUNCH, "1")
        .env_remove(crate::window::ENV_LAST_RELAUNCH_UNIX)
        .env_remove(crate::window::ENV_HANG_RELAUNCHES)
        .spawn()
        .map(drop)
        .map_err(|e| e.to_string())
}

/// Startup housekeeping after an update. Runs only in the instance that owns
/// the single-instance mutex, before any new update can start. Deletes a
/// stale `<exe>.new` / `<exe>.new.part` at once, the files the old
/// helper-based updater left in the local updates folder, and `<exe>.old`.
/// The previous process may still be exiting and hold `<exe>.old` mapped, so
/// that one is retried in the background; a failure is silent and retried on
/// the next start. The retry can never hit the running exe moved aside by a
/// later update: a mapped image cannot be deleted.
pub fn cleanup_after_update() {
    if let Ok(dir) = updates_dir() {
        remove_legacy_update_files(&dir);
    }
    let Ok(current_exe) = std::env::current_exe() else {
        return;
    };
    for stale in [
        staged_path_for(&current_exe),
        partial_path_for(&current_exe),
    ] {
        let _ = std::fs::remove_file(stale);
    }
    let backup = [backup_path_for(&current_exe)];
    if remove_leftovers(&backup, 1, Duration::ZERO) == 0 {
        return;
    }
    std::thread::spawn(move || {
        remove_leftovers(&backup, LEFTOVER_CLEANUP_ATTEMPTS, LEFTOVER_CLEANUP_DELAY);
    });
}

/// Returns how many of `paths` still exist after the last attempt.
fn remove_leftovers(paths: &[PathBuf], attempts: u32, delay: Duration) -> usize {
    let mut pending = 0;
    for attempt in 0..attempts {
        pending = paths
            .iter()
            .filter(|path| path.exists() && std::fs::remove_file(path).is_err())
            .count();
        if pending == 0 || attempt + 1 == attempts {
            break;
        }
        std::thread::sleep(delay);
    }
    pending
}

fn remove_legacy_update_files(dir: &Path) {
    for name in LEGACY_UPDATE_FILES {
        let _ = std::fs::remove_file(dir.join(name));
    }
    // Succeeds only when the folder is empty.
    let _ = std::fs::remove_dir(dir);
}

fn fetch_latest_release() -> Result<Option<ReleaseDescriptor>, String> {
    let (owner, repo) = github_repo()?;
    let url = format!("https://api.github.com/repos/{owner}/{repo}/releases/latest");
    let agent = build_agent()?;

    let mut response = agent
        .get(&url)
        .header("Accept", GITHUB_API_ACCEPT)
        .header("User-Agent", user_agent())
        .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
        .call()
        .map_err(|e| format!("Unable to check GitHub releases: {e}"))?;

    let release: GitHubRelease = response
        .body_mut()
        .read_json()
        .map_err(|e| format!("Unable to parse GitHub release data: {e}"))?;

    release_descriptor(release, env!("CARGO_PKG_VERSION"))
}

fn build_agent() -> Result<ureq::Agent, String> {
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::NativeTls)
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    Ok(ureq::Agent::config_builder()
        .https_only(true)
        .timeout_global(Some(Duration::from_secs(30)))
        .tls_config(tls)
        .build()
        .into())
}

fn updates_dir() -> Result<PathBuf, String> {
    crate::known_folders::local_app_data_dir()
        .map(|dir| dir.join("ClaudeCodeUsageMonitor").join("updates"))
        .or_else(|| {
            Some(
                std::env::temp_dir()
                    .join("ClaudeCodeUsageMonitor")
                    .join("updates"),
            )
        })
        .ok_or_else(|| "Unable to resolve a writable local updates directory.".to_string())
}

/// `<exe>.<suffix>` next to the executable (`app.exe` -> `app.exe.old`).
fn sibling_path(target: &Path, suffix: &str) -> PathBuf {
    let mut name = target
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| "app.exe".into());
    name.push(".");
    name.push(suffix);
    target.with_file_name(name)
}

fn backup_path_for(target: &Path) -> PathBuf {
    sibling_path(target, "old")
}

fn staged_path_for(target: &Path) -> PathBuf {
    sibling_path(target, "new")
}

fn partial_path_for(target: &Path) -> PathBuf {
    sibling_path(target, "new.part")
}

fn ensure_target_location_writable(target: &Path) -> Result<(), String> {
    let parent = target.parent().ok_or_else(|| {
        "Unable to determine the install directory for the current executable.".to_string()
    })?;

    let probe_path = parent.join(".__ccum_update_probe");
    match File::create(&probe_path) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe_path);
            Ok(())
        }
        Err(error) => Err(format!(
            "The current install location is not writable. Move the app to a user-writable folder or install it somewhere outside Program Files. {error}"
        )),
    }
}

fn github_repo() -> Result<(&'static str, &'static str), String> {
    let repository = env!("CARGO_PKG_REPOSITORY").trim_end_matches('/');
    let parts: Vec<&str> = repository.split('/').collect();
    if parts.len() < 2 {
        return Err("Package repository URL is not configured for GitHub releases.".to_string());
    }

    let owner = parts[parts.len() - 2];
    let repo = parts[parts.len() - 1];
    if owner.is_empty() || repo.is_empty() {
        return Err("Package repository URL is not configured for GitHub releases.".to_string());
    }

    Ok((owner, repo))
}

fn user_agent() -> &'static str {
    concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests;
