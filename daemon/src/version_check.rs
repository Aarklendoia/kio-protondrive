//! Checks for a newer `proton-drive` CLI release (#26, #65) and, with the
//! user's explicit confirmation, applies it.
//!
//! Earlier versions of this check just ran `proton-drive --version` and
//! relayed whichever "a newer version is available" sentence the CLI itself
//! printed — but that self-check is a CLI feature that only exists in
//! builds roughly 0.6.0+, so it's silently absent on anything older,
//! meaning a daemon watching an old CLI could never tell the user anything
//! at all (the actual bug #65 was filed for). This now fetches
//! [`protondrive_core::cli_update`]'s release manifest itself — independent
//! of whatever CLI version happens to be installed — and compares versions
//! directly.
//!
//! Only *notifies* by default. It goes one step further — best-effort
//! spawning the setup wizard in `--update-cli` mode, so the user can
//! actually apply it with one click — only when the resolved `proton-drive`
//! binary is one this process can write to (never a root-owned system
//! install: this project never escalates privileges itself, same rule as
//! `wizard::route_setup_pass`'s "never runs `apt install`").

use std::time::Duration;

use protondrive_core::cli::CommandRunner;
use protondrive_core::cli_update::{self, CliUpdateError, Release};
use protondrive_core::local_ctrl;

use crate::notification::Notifier;

/// `--version`'s own network check times out at 5s internally (seen in the
/// CLI's source); this leaves headroom for process startup on top of that.
const CHECK_TIMEOUT: Duration = Duration::from_secs(10);

/// Runs `proton-drive --version` to learn the installed version, fetches
/// the latest stable release via `fetch_latest`, and — if it's newer —
/// notifies and calls `offer_update` (real production behavior:
/// [`offer_wizard_update`], gated on the install being one this process can
/// actually write to). `offer_update` is a separate injected side effect
/// (rather than hardcoded) purely so tests never spawn a real
/// `kio-protondrive-wizard` GUI process — a developer running `cargo test`
/// on a real desktop with the CLI already installed must never see an
/// actual wizard window pop up from a test fixture's made-up "0.8.0", same
/// concern `notification::Notifier` already exists to prevent for
/// `notify-send`. `already_notified` suppresses repeat
/// notifications/offers for the same remote version across check cycles,
/// reset once the installed version catches up — same falling-edge pattern
/// as `main::report_authentication_failure`.
pub fn check(
    runner: &dyn CommandRunner,
    notifier: &dyn Notifier,
    fetch_latest: &dyn Fn() -> Result<Release, CliUpdateError>,
    offer_update: &dyn Fn(),
    already_notified: &mut Option<String>,
) {
    let output = match runner.run(&["--version"], CHECK_TIMEOUT) {
        Ok(output) => output,
        Err(err) => {
            log::debug!("could not check the proton-drive CLI version: {err}");
            return;
        }
    };
    let Some(installed) = cli_update::installed_version(&output.stdout) else {
        log::debug!(
            "could not parse the installed proton-drive CLI's version from --version's output"
        );
        return;
    };

    let release = match fetch_latest() {
        Ok(release) => release,
        Err(err) => {
            log::debug!("could not check for a newer proton-drive CLI release: {err}");
            return;
        }
    };

    if !cli_update::is_newer(&release.version, installed) {
        log::debug!("proton-drive CLI {installed} is up to date");
        *already_notified = None;
        return;
    }

    log::warn!(
        "a newer version of the Proton Drive CLI is available: {} (you have {installed})",
        release.version
    );
    if already_notified.as_deref() == Some(release.version.as_str()) {
        return;
    }
    notifier.cli_update_available(&release.version, installed);
    *already_notified = Some(release.version.clone());
    offer_update();
}

/// Real `offer_update`: best-effort spawns the setup wizard in
/// `--update-cli` mode, but only if the resolved `proton-drive` binary is
/// one this process can write to — never a root-owned system install, same
/// "never escalate privileges" rule as `wizard::route_setup_pass`'s "never
/// runs `apt install`".
pub fn offer_wizard_update() {
    let Some(cli_path) = local_ctrl::which_path("proton-drive") else {
        return;
    };
    if !cli_update::is_writable(&cli_path) {
        log::debug!(
            "{} isn't writable by this process — leaving the update to the user \
             (never escalating privileges to apply it)",
            cli_path.display()
        );
        return;
    }
    // Best-effort: kio-protondrive-wizard is a Recommends, not a hard
    // Depends, so it may not be installed — that's fine, the notification
    // already told the user a new version exists.
    if let Err(err) = std::process::Command::new("kio-protondrive-wizard")
        .arg("--update-cli")
        .spawn()
    {
        log::debug!("could not launch the setup wizard to offer the update: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protondrive_core::cli::{self, DriveError};
    use protondrive_core::cli_update::ReleaseFile;
    use std::cell::RefCell;

    struct ScriptedRunner(cli::CommandOutput);

    impl CommandRunner for ScriptedRunner {
        fn run(&self, args: &[&str], _timeout: Duration) -> Result<cli::CommandOutput, DriveError> {
            assert_eq!(args, ["--version"]);
            Ok(cli::CommandOutput {
                stdout: self.0.stdout.clone(),
                stderr: self.0.stderr.clone(),
                success: self.0.success,
            })
        }
    }

    /// Records calls instead of shelling out to a real `notify-send` — a
    /// developer running `cargo test` on a real desktop must never see a
    /// notification pop up from a test fixture's made-up "0.9.0" (this is
    /// exactly what happened before this trait existed).
    #[derive(Default)]
    struct RecordingNotifier(RefCell<Vec<(String, String)>>);

    impl Notifier for RecordingNotifier {
        fn cli_update_available(&self, latest: &str, installed: &str) {
            self.0
                .borrow_mut()
                .push((latest.to_string(), installed.to_string()));
        }
    }

    fn version_output(stdout: &str) -> cli::CommandOutput {
        cli::CommandOutput {
            stdout: stdout.to_string(),
            stderr: String::new(),
            success: true,
        }
    }

    fn no_op() {}

    fn release(version: &str) -> Release {
        Release {
            category: "Stable".to_string(),
            version: version.to_string(),
            files: vec![ReleaseFile {
                url: "https://proton.me/download/drive/cli/x/linux-x64/proton-drive".to_string(),
                sha512: "deadbeef".to_string(),
                platform: "linux/x64".to_string(),
            }],
        }
    }

    #[test]
    fn notifies_once_when_a_newer_release_exists_and_offers_to_apply_it() {
        let runner = ScriptedRunner(version_output(
            "Proton Drive CLI cli-drive@0.7.0+5174900c\n",
        ));
        let notifier = RecordingNotifier::default();
        let mut notified = None;
        let fetch = || Ok(release("0.8.0"));
        let offers = RefCell::new(0u32);
        let offer_update = || *offers.borrow_mut() += 1;

        check(&runner, &notifier, &fetch, &offer_update, &mut notified);
        assert_eq!(notified.as_deref(), Some("0.8.0"));
        assert_eq!(
            notifier.0.borrow().as_slice(),
            [("0.8.0".to_string(), "0.7.0".to_string())]
        );
        assert_eq!(*offers.borrow(), 1);

        // A second cycle with the same remote version shouldn't notify
        // (or re-offer) again.
        check(&runner, &notifier, &fetch, &offer_update, &mut notified);
        assert_eq!(notifier.0.borrow().len(), 1);
        assert_eq!(*offers.borrow(), 1);
    }

    #[test]
    fn does_not_notify_when_already_up_to_date() {
        let runner = ScriptedRunner(version_output(
            "Proton Drive CLI cli-drive@0.8.0+06e8c605\n",
        ));
        let notifier = RecordingNotifier::default();
        let mut notified = None;
        let fetch = || Ok(release("0.8.0"));

        check(&runner, &notifier, &fetch, &no_op, &mut notified);
        assert_eq!(notified, None);
        assert!(notifier.0.borrow().is_empty());
    }

    #[test]
    fn clears_notified_state_once_up_to_date_again() {
        let runner = ScriptedRunner(version_output(
            "Proton Drive CLI cli-drive@0.9.0+deadbeef\n",
        ));
        let notifier = RecordingNotifier::default();
        let mut notified = Some("0.9.0".to_string());
        let fetch = || Ok(release("0.9.0"));

        check(&runner, &notifier, &fetch, &no_op, &mut notified);
        assert_eq!(notified, None);
        assert!(notifier.0.borrow().is_empty());
    }

    #[test]
    fn does_nothing_when_the_manifest_fetch_fails() {
        let runner = ScriptedRunner(version_output(
            "Proton Drive CLI cli-drive@0.7.0+5174900c\n",
        ));
        let notifier = RecordingNotifier::default();
        let mut notified = None;
        let fetch = || Err(CliUpdateError::ChecksumMismatch);

        check(&runner, &notifier, &fetch, &no_op, &mut notified);
        assert_eq!(notified, None);
        assert!(notifier.0.borrow().is_empty());
    }

    #[test]
    fn does_nothing_when_the_installed_version_cannot_be_parsed() {
        let runner = ScriptedRunner(version_output("garbage output\n"));
        let notifier = RecordingNotifier::default();
        let mut notified = None;
        let fetch = || Ok(release("0.8.0"));

        check(&runner, &notifier, &fetch, &no_op, &mut notified);
        assert_eq!(notified, None);
        assert!(notifier.0.borrow().is_empty());
    }
}
