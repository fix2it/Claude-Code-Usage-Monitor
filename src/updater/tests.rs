use std::os::windows::fs::OpenOptionsExt;

use super::*;

#[test]
fn configured_https_transport_does_not_panic() {
    crate::https_test::assert_tls_handshake(super::build_agent().expect("HTTP agent should build"));
}

#[test]
#[ignore = "requires direct Internet access to github.com and badssl.com"]
fn configured_https_transport_validates_certificates() {
    crate::https_test::assert_https_certificate_validation(
        build_agent().expect("HTTP agent should build"),
    );
}

const ABC_DIGEST: &str = "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

fn installed(dir: &Path) -> PathBuf {
    let target = dir.join("app.exe");
    std::fs::write(&target, b"original").unwrap();
    target
}

fn stage(target: &Path, bytes: &[u8]) -> PathBuf {
    let staged = staged_path_for(target);
    std::fs::write(&staged, bytes).unwrap();
    staged
}

#[test]
fn update_files_sit_next_to_the_exe() {
    let target = Path::new(r"C:\Apps\monitor.exe");
    assert_eq!(
        staged_path_for(target),
        Path::new(r"C:\Apps\monitor.exe.new")
    );
    assert_eq!(
        backup_path_for(target),
        Path::new(r"C:\Apps\monitor.exe.old")
    );
    assert_eq!(
        partial_path_for(target),
        Path::new(r"C:\Apps\monitor.exe.new.part")
    );
}

#[test]
fn corrupt_staged_update_fails_verification() {
    let dir = tempfile::tempdir().unwrap();
    let staged = dir.path().join("app.exe.new");
    std::fs::write(&staged, b"abd").unwrap();
    let integrity = AssetIntegrity::new(3, Some(ABC_DIGEST)).unwrap();
    let error = open_verified_source(&staged, &integrity).unwrap_err();
    assert!(error.contains("SHA-256"));
}

#[test]
fn swap_installs_the_staged_binary_and_keeps_the_old_one_aside() {
    let dir = tempfile::tempdir().unwrap();
    let target = installed(dir.path());
    let staged = stage(&target, b"abc");

    swap_in_staged_binary(&target, &staged).unwrap();

    assert_eq!(std::fs::read(&target).unwrap(), b"abc");
    assert_eq!(
        std::fs::read(backup_path_for(&target)).unwrap(),
        b"original"
    );
    assert!(!staged.exists());
}

#[test]
fn swap_replaces_a_leftover_backup_from_an_earlier_update() {
    let dir = tempfile::tempdir().unwrap();
    let target = installed(dir.path());
    std::fs::write(backup_path_for(&target), b"older").unwrap();
    let staged = stage(&target, b"abc");

    swap_in_staged_binary(&target, &staged).unwrap();

    assert_eq!(std::fs::read(&target).unwrap(), b"abc");
    assert_eq!(
        std::fs::read(backup_path_for(&target)).unwrap(),
        b"original"
    );
}

#[test]
fn failed_install_rename_restores_the_installed_binary() {
    let dir = tempfile::tempdir().unwrap();
    let target = installed(dir.path());
    // The staged file is missing, so the second rename fails.
    let staged = staged_path_for(&target);

    let error = swap_in_staged_binary(&target, &staged).unwrap_err();

    assert!(error.contains("Unable to install the update"), "{error}");
    assert_eq!(std::fs::read(&target).unwrap(), b"original");
    assert!(!backup_path_for(&target).exists());
}

#[test]
fn failed_move_aside_leaves_everything_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let target = installed(dir.path());
    let staged = stage(&target, b"abc");
    // A handle without FILE_SHARE_DELETE blocks renaming the target.
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&target)
        .unwrap();

    let error = swap_in_staged_binary(&target, &staged).unwrap_err();
    drop(lock);

    assert!(error.contains("move the running app aside"), "{error}");
    assert_eq!(std::fs::read(&target).unwrap(), b"original");
    assert_eq!(std::fs::read(&staged).unwrap(), b"abc");
    assert!(!backup_path_for(&target).exists());
}

#[test]
fn swap_never_deletes_the_only_copy_in_the_backup() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("app.exe");
    let backup = backup_path_for(&target);
    // A previous restore failed: the installed exe survives only as the backup.
    std::fs::write(&backup, b"original").unwrap();
    let staged = stage(&target, b"abc");

    let error = swap_in_staged_binary(&target, &staged).unwrap_err();

    assert!(error.contains("kept at"), "{error}");
    assert_eq!(std::fs::read(&backup).unwrap(), b"original");
    assert!(!target.exists());
}

#[test]
fn rollback_after_a_failed_launch_restores_the_installed_binary() {
    let dir = tempfile::tempdir().unwrap();
    let target = installed(dir.path());
    let staged = stage(&target, b"abc");
    swap_in_staged_binary(&target, &staged).unwrap();

    rollback_swap(&target, &staged).unwrap();

    assert_eq!(std::fs::read(&target).unwrap(), b"original");
    assert_eq!(std::fs::read(&staged).unwrap(), b"abc");
    assert!(!backup_path_for(&target).exists());
}

#[test]
fn failed_rollback_reports_where_the_previous_version_is() {
    let dir = tempfile::tempdir().unwrap();
    let target = installed(dir.path());
    let staged = stage(&target, b"abc");
    swap_in_staged_binary(&target, &staged).unwrap();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&target)
        .unwrap();

    let error = rollback_swap(&target, &staged).unwrap_err();
    drop(lock);

    assert!(error.contains("kept at"), "{error}");
    assert_eq!(
        std::fs::read(backup_path_for(&target)).unwrap(),
        b"original"
    );
}

#[test]
fn leftover_cleanup_removes_files_and_reports_locked_ones() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("app.exe.old");
    let locked = dir.path().join("app.exe.new");
    let missing = dir.path().join("app.exe.new.part");
    std::fs::write(&old, b"x").unwrap();
    std::fs::write(&locked, b"x").unwrap();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&locked)
        .unwrap();
    let paths = [old.clone(), locked.clone(), missing];

    assert_eq!(remove_leftovers(&paths, 2, Duration::ZERO), 1);
    assert!(!old.exists());
    assert!(locked.exists());

    drop(lock);
    assert_eq!(remove_leftovers(&paths, 1, Duration::ZERO), 0);
    assert!(!locked.exists());
}

#[test]
fn legacy_helper_files_are_removed_with_their_folder() {
    let dir = tempfile::tempdir().unwrap();
    let updates = dir.path().join("updates");
    std::fs::create_dir(&updates).unwrap();
    for name in LEGACY_UPDATE_FILES {
        std::fs::write(updates.join(name), b"x").unwrap();
    }

    remove_legacy_update_files(&updates);

    assert!(!updates.exists());
}

#[test]
fn legacy_cleanup_keeps_a_folder_with_unknown_files() {
    let dir = tempfile::tempdir().unwrap();
    let updates = dir.path().join("updates");
    std::fs::create_dir(&updates).unwrap();
    std::fs::write(updates.join("updater-helper.exe"), b"x").unwrap();
    std::fs::write(updates.join("keep.txt"), b"x").unwrap();

    remove_legacy_update_files(&updates);

    assert!(!updates.join("updater-helper.exe").exists());
    assert!(updates.join("keep.txt").exists());
}

#[test]
fn updater_agent_rejects_plain_http() {
    let error = build_agent()
        .unwrap()
        .get("http://127.0.0.1:1/update.exe")
        .call()
        .unwrap_err();
    assert!(matches!(error, ureq::Error::RequireHttpsOnly(_)));
}

#[test]
fn install_channel_is_always_portable_for_the_fork() {
    assert_eq!(current_install_channel(), InstallChannel::Portable);
}

#[test]
fn portable_checks_only_github_and_returns_its_verified_release() {
    let release = ReleaseDescriptor {
        latest_version: "99.0.0".into(),
        asset_url: "https://example.invalid/update.exe".into(),
        integrity: AssetIntegrity::new(3, Some(ABC_DIGEST)).unwrap(),
    };
    let result = check_channel_updates(InstallChannel::Portable, || Ok(Some(release))).unwrap();
    let UpdateCheckResult::Available(AvailableUpdate::Release(release)) = result else {
        panic!("Expected a portable release");
    };
    assert_eq!(release.latest_version, "99.0.0");
    assert_eq!(release.integrity.digest_arg(), ABC_DIGEST);
    assert!(matches!(
        check_channel_updates(InstallChannel::Portable, || Ok(None)).unwrap(),
        UpdateCheckResult::UpToDate
    ));
}

#[test]
fn portable_check_reports_github_errors() {
    let portable =
        check_channel_updates(
            InstallChannel::Portable,
            || Err("GitHub unavailable".into()),
        );
    assert_eq!(portable.unwrap_err(), "GitHub unavailable");
}
