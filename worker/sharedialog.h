#pragma once

#include <QDialog>
#include <QString>

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

    QString m_remotePath;
    // Whether the node had an active public link the last time this was
    // checked (from stat's isSharedByUrl — `sharing status` itself doesn't
    // carry this, see core/src/entry.rs's SharingStatus doc comment) or a
    // link was created/updated in this dialog session. The link's actual
    // URL is only known once create/update returns it — see this class's
    // own doc comment on why "remove" stays available even without it.
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
