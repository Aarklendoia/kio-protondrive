#pragma once

#include <QDialog>
#include <QString>

#include <functional>

class QComboBox;
class QLabel;
class QLineEdit;
class QListWidget;
class QCheckBox;
class QDateEdit;
class QPushButton;

/**
 * "Share..." context-menu dialog (see fileitemactionplugin.cpp, issue #6):
 * members/pending invitations and the public link for a single node, backed
 * directly by the `sharing_*` cxx bridge functions (core/src/bridge.rs).
 *
 * Every action (invite, remove, create/update/remove link) is a synchronous,
 * blocking cxx call on the GUI thread — matches this codebase's existing
 * precedent (restore/empty_trash in fileitemactionplugin.cpp) rather than
 * introducing new async infrastructure; a wait cursor is shown for the
 * ~1-4s the CLI typically takes. Unlike those fire-and-forget actions,
 * failures here are shown with QMessageBox — a submitted form needs visible
 * feedback, not a silent no-op.
 */
class ShareDialog : public QDialog
{
    Q_OBJECT

public:
    ShareDialog(const QString &remotePath, const QString &displayName, QWidget *parent = nullptr);

private:
    void reloadStatus();
    void inviteClicked();
    void removeSelectedMemberClicked();
    void createOrUpdateLinkClicked();
    void removeLinkClicked();
    void copyLinkClicked();

    // Runs `action` (expected to make exactly one blocking cxx call) under
    // a wait cursor, restored before either returning true or, on a thrown
    // rust::Error, showing it in a warning dialog and returning false — the
    // single place this shared set-cursor/try/catch/restore/warn pattern
    // lives, instead of each handler hand-rolling its own (previously with
    // subtle variants: some restored-then-returned, others restored then
    // had to re-set the cursor for unrelated UI work that followed).
    bool tryOrWarn(const std::function<void()> &action);

    QString m_remotePath;
    // Whether the node had an active public link the last time this was
    // checked — from `sharing_status`'s own `has_public_link`/
    // `public_link_url` (confirmed live: `sharing status` does carry the
    // active link, same as `sharing set-url`'s response shape — see
    // core/src/entry.rs's SharingStatus doc comment) or a link was
    // created/updated/removed in this dialog session.
    bool m_hasPublicLink = false;

    QListWidget *m_memberList = nullptr;
    QPushButton *m_removeMemberButton = nullptr;
    QLineEdit *m_inviteEmail = nullptr;
    QComboBox *m_inviteRole = nullptr;
    QLineEdit *m_inviteMessage = nullptr;

    QLabel *m_linkStatusLabel = nullptr;
    QLineEdit *m_linkUrl = nullptr;
    QPushButton *m_copyLinkButton = nullptr;
    QComboBox *m_linkRole = nullptr;
    QLineEdit *m_linkPassword = nullptr;
    QCheckBox *m_linkHasExpiration = nullptr;
    QDateEdit *m_linkExpiration = nullptr;
    QPushButton *m_removeLinkButton = nullptr;

    QLabel *m_editorsCanShareLabel = nullptr;
};
