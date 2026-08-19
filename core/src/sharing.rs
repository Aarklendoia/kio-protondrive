//! Worker-friendly view of a node's sharing state (see #6), built on top of
//! `crate::cli`'s raw `sharing_*` wrappers — mirrors `crate::photos`'s split
//! between raw CLI JSON (`crate::entry`) and a merged shape callers actually
//! want.
//!
//! `sharing status`'s three separate lists (`members`, `protonInvitations`,
//! `nonProtonInvitations`) collapse into one flat [`ShareMember`] list here:
//! the dialog only needs "who has access or a pending invite, and what
//! role", not which of the three CLI-internal buckets they're currently in.

use crate::cli::{self, CommandRunner, DriveError};
use crate::entry::PublicLink;

/// One person with access to a node, or with a pending invitation —
/// `pending` distinguishes the two (see this module's doc comment for why
/// the CLI's three separate lists collapse into one here).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareMember {
    pub email: String,
    pub role: String,
    pub pending: bool,
}

#[derive(Debug, Clone)]
pub struct SharingStatus {
    pub members: Vec<ShareMember>,
    pub editors_can_share: bool,
}

pub fn status(runner: &dyn CommandRunner, path: &str) -> Result<SharingStatus, DriveError> {
    let raw = cli::sharing_status(runner, path)?;
    let mut members: Vec<ShareMember> = raw
        .members
        .into_iter()
        .map(|m| ShareMember {
            email: m.invitee_email,
            role: m.role,
            pending: false,
        })
        .collect();
    members.extend(raw.proton_invitations.into_iter().map(|i| ShareMember {
        email: i.invitee_email,
        role: i.role,
        pending: true,
    }));
    members.extend(raw.non_proton_invitations.into_iter().map(|i| ShareMember {
        email: i.invitee_email,
        role: i.role,
        pending: true,
    }));
    Ok(SharingStatus {
        members,
        editors_can_share: raw.editors_can_share,
    })
}

pub fn invite(
    runner: &dyn CommandRunner,
    path: &str,
    email: &str,
    role: &str,
    message: &str,
) -> Result<(), DriveError> {
    cli::sharing_invite(runner, path, email, role, message)
}

pub fn remove_member(
    runner: &dyn CommandRunner,
    path: &str,
    email: &str,
) -> Result<(), DriveError> {
    cli::sharing_remove_member(runner, path, email)
}

pub fn set_link(
    runner: &dyn CommandRunner,
    path: &str,
    role: &str,
    password: &str,
    expiration: &str,
) -> Result<PublicLink, DriveError> {
    cli::sharing_set_link(runner, path, role, password, expiration)
}

pub fn remove_link(runner: &dyn CommandRunner, path: &str) -> Result<(), DriveError> {
    cli::sharing_remove_link(runner, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::CommandOutput;
    use std::time::Duration;

    struct MockRunner {
        stdout: String,
    }

    impl CommandRunner for MockRunner {
        fn run(&self, _args: &[&str], _timeout: Duration) -> Result<CommandOutput, DriveError> {
            Ok(CommandOutput {
                stdout: self.stdout.clone(),
                stderr: String::new(),
                success: true,
            })
        }
    }

    #[test]
    fn status_merges_members_and_both_invitation_lists() {
        let runner = MockRunner {
            stdout: r#"{
                "protonInvitations": [{"inviteeEmail": "pending-proton@example.com", "role": "viewer"}],
                "nonProtonInvitations": [{"inviteeEmail": "pending-outside@example.com", "role": "editor"}],
                "members": [{"inviteeEmail": "already-a-member@example.com", "role": "admin"}],
                "editorsCanShare": true
            }"#
            .to_string(),
        };

        let result = status(&runner, "/my-files/report.pdf").unwrap();

        assert!(result.editors_can_share);
        assert_eq!(
            result.members,
            vec![
                ShareMember {
                    email: "already-a-member@example.com".to_string(),
                    role: "admin".to_string(),
                    pending: false,
                },
                ShareMember {
                    email: "pending-proton@example.com".to_string(),
                    role: "viewer".to_string(),
                    pending: true,
                },
                ShareMember {
                    email: "pending-outside@example.com".to_string(),
                    role: "editor".to_string(),
                    pending: true,
                },
            ]
        );
    }

    #[test]
    fn status_handles_a_node_with_no_members_or_invitations() {
        let runner = MockRunner {
            stdout: r#"{"protonInvitations":[],"nonProtonInvitations":[],"members":[],"editorsCanShare":false}"#
                .to_string(),
        };

        let result = status(&runner, "/my-files/report.pdf").unwrap();

        assert!(result.members.is_empty());
        assert!(!result.editors_can_share);
    }
}
