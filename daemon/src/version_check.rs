//! Best-effort check for a newer `proton-drive` CLI release (#26).
//!
//! Proton doesn't publish an apt/deb repo or any self-update mechanism for
//! the CLI (see README's "Installing" section) — but the CLI itself, as of
//! some version between 0.6.0 and 0.8.0, started checking on every
//! `--version` call and printing the verdict as one of two fixed sentences,
//! fetching `https://proton.me/download/drive/cli/version.json` internally
//! (found by reading the CLI's own bundled JS source — undocumented
//! anywhere, so this could silently stop working if Proton ever changes the
//! wording). Rather than reimplementing that check ourselves (a second HTTP
//! call, a second copy of Proton's version-compare logic), this just runs
//! `proton-drive --version` and relays whichever sentence it printed.
//!
//! Silently does nothing if the installed CLI predates this feature (no
//! such sentence in the output) or the check itself failed (e.g. offline —
//! the CLI swallows that internally and just omits the verdict line) —
//! matches the CLI's own graceful degradation, and this project's existing
//! "best-effort, never fail the daemon over it" stance (see
//! `notification::auth_required`).

use std::time::Duration;

use protondrive_core::cli::CommandRunner;

use crate::notification::Notifier;

/// `--version`'s own network check times out at 5s internally (seen in the
/// CLI's source); this leaves headroom for process startup on top of that.
const CHECK_TIMEOUT: Duration = Duration::from_secs(10);

const UP_TO_DATE_MARKER: &str = "You are running the latest version.";
const UPDATE_AVAILABLE_MARKER: &str = "A newer version is available";

/// Runs `proton-drive --version` and logs/notifies if it reports a newer
/// release available. `already_notified` suppresses repeat desktop
/// notifications for the same message across check cycles — reset once the
/// message changes (e.g. the user updates, or a further release ships) —
/// same falling-edge pattern as `main::report_authentication_failure`.
pub fn check(
    runner: &dyn CommandRunner,
    notifier: &dyn Notifier,
    already_notified: &mut Option<String>,
) {
    let output = match runner.run(&["--version"], CHECK_TIMEOUT) {
        Ok(output) => output,
        Err(err) => {
            log::debug!("could not check the proton-drive CLI version: {err}");
            return;
        }
    };

    if let Some(line) = find_update_message(&output.stdout) {
        log::warn!("{line} — see https://proton.me/drive/download");
        if already_notified.as_deref() != Some(line) {
            notifier.cli_update_available(line);
            *already_notified = Some(line.to_string());
        }
    } else if output.stdout.contains(UP_TO_DATE_MARKER) {
        log::debug!("proton-drive CLI is up to date");
        *already_notified = None;
    } else {
        log::debug!(
            "proton-drive CLI's --version output had no update-check verdict \
             (older CLI without this feature, or the check itself failed offline)"
        );
    }
}

/// Pulls out the CLI's own "A newer version is available: X.Y.Z (you have
/// A.B.C)." line, if present — kept as one opaque string rather than parsing
/// out the version numbers, since relaying the CLI's exact wording is all
/// this needs and doesn't require tracking its message format beyond the one
/// fixed marker.
fn find_update_message(stdout: &str) -> Option<&str> {
    stdout
        .lines()
        .find(|line| line.starts_with(UPDATE_AVAILABLE_MARKER))
}

#[cfg(test)]
mod tests {
    use super::*;
    use protondrive_core::cli::{self, DriveError};
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
    struct RecordingNotifier(RefCell<Vec<String>>);

    impl Notifier for RecordingNotifier {
        fn cli_update_available(&self, message: &str) {
            self.0.borrow_mut().push(message.to_string());
        }
    }

    fn output(stdout: &str) -> cli::CommandOutput {
        cli::CommandOutput {
            stdout: stdout.to_string(),
            stderr: String::new(),
            success: true,
        }
    }

    #[test]
    fn finds_the_update_available_line() {
        let stdout = "Proton Drive CLI cli-drive@0.8.0+06e8c605\n\
                       Proton Drive SDK js@0.21.0+06e8c605\n\
                       A newer version is available: 0.9.0 (you have 0.8.0).\n\
                       Download at https://proton.me/download/drive/cli/index.html\n";
        assert_eq!(
            find_update_message(stdout),
            Some("A newer version is available: 0.9.0 (you have 0.8.0).")
        );
    }

    #[test]
    fn finds_nothing_when_up_to_date() {
        let stdout = "Proton Drive CLI cli-drive@0.8.0+06e8c605\n\
                       Proton Drive SDK js@0.21.0+06e8c605\n\
                       You are running the latest version.\n";
        assert_eq!(find_update_message(stdout), None);
    }

    #[test]
    fn finds_nothing_on_an_older_cli_without_the_verdict_line() {
        let stdout = "Proton Drive CLI cli-drive@0.6.0+f8e16aac\n\
                       Proton Drive SDK js@0.19.2+f8e16aac\n";
        assert_eq!(find_update_message(stdout), None);
    }

    #[test]
    fn notifies_once_per_distinct_message() {
        let runner = ScriptedRunner(output(
            "Proton Drive CLI cli-drive@0.8.0+06e8c605\n\
             A newer version is available: 0.9.0 (you have 0.8.0).\n",
        ));
        let notifier = RecordingNotifier::default();
        let mut notified = None;
        check(&runner, &notifier, &mut notified);
        assert_eq!(
            notified.as_deref(),
            Some("A newer version is available: 0.9.0 (you have 0.8.0).")
        );

        // A second cycle with the same message shouldn't notify again.
        check(&runner, &notifier, &mut notified);
        assert_eq!(
            notified.as_deref(),
            Some("A newer version is available: 0.9.0 (you have 0.8.0).")
        );
        assert_eq!(
            notifier.0.into_inner(),
            vec!["A newer version is available: 0.9.0 (you have 0.8.0).".to_string()]
        );
    }

    #[test]
    fn clears_notified_state_once_up_to_date_again() {
        let mut notified =
            Some("A newer version is available: 0.9.0 (you have 0.8.0).".to_string());
        let runner = ScriptedRunner(output(
            "Proton Drive CLI cli-drive@0.9.0+deadbeef\n\
             You are running the latest version.\n",
        ));
        let notifier = RecordingNotifier::default();
        check(&runner, &notifier, &mut notified);
        assert_eq!(notified, None);
        assert!(notifier.0.into_inner().is_empty());
    }
}
