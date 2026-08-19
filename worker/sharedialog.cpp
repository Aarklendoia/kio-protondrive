#include "sharedialog.h"

#include <KLocalizedString>
#include <QCheckBox>
#include <QClipboard>
#include <QComboBox>
#include <QDateEdit>
#include <QDialogButtonBox>
#include <QGroupBox>
#include <QGuiApplication>
#include <QHBoxLayout>
#include <QIcon>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QMessageBox>
#include <QPushButton>
#include <QVBoxLayout>

#include "rust/cxx.h"

#include "protondrive-core-cxxbridge/bridge.h"

using namespace protondrive;

namespace
{
// Mirrors protondriveworker.cpp's own toQString(const rust::String &) —
// duplicated rather than shared, same call as trashPrefix/photosPrefix in
// fileitemactionplugin.cpp: a small stable helper, not worth a header for.
QString toQString(const rust::String &value)
{
    return QString::fromUtf8(value.data(), static_cast<int>(value.size()));
}

QString roleLabel(const QString &role)
{
    if (role == QLatin1String("viewer")) {
        return i18nd("kio_protondrive", "Viewer");
    }
    if (role == QLatin1String("editor")) {
        return i18nd("kio_protondrive", "Editor");
    }
    if (role == QLatin1String("admin")) {
        return i18nd("kio_protondrive", "Admin");
    }
    // "inherited" (folder members only) or anything else the CLI might add
    // later — shown as-is rather than silently dropped.
    return role;
}
}

ShareDialog::ShareDialog(const QString &remotePath, const QString &displayName, QWidget *parent)
    : QDialog(parent)
    , m_remotePath(remotePath)
{
    setWindowTitle(i18nd("kio_protondrive", "Share \"%1\"", displayName));
    setMinimumWidth(420);

    auto *layout = new QVBoxLayout(this);

    auto *membersGroup = new QGroupBox(i18nd("kio_protondrive", "People with access"), this);
    auto *membersLayout = new QVBoxLayout(membersGroup);

    m_memberList = new QListWidget(membersGroup);
    membersLayout->addWidget(m_memberList);

    m_removeMemberButton =
        new QPushButton(QIcon::fromTheme(QStringLiteral("list-remove")), i18nd("kio_protondrive", "Remove Access"), membersGroup);
    m_removeMemberButton->setEnabled(false);
    connect(m_removeMemberButton, &QPushButton::clicked, this, &ShareDialog::removeSelectedMemberClicked);
    connect(m_memberList, &QListWidget::itemSelectionChanged, this, [this]() {
        m_removeMemberButton->setEnabled(!m_memberList->selectedItems().isEmpty());
    });
    membersLayout->addWidget(m_removeMemberButton);

    auto *inviteLayout = new QHBoxLayout;
    m_inviteEmail = new QLineEdit(membersGroup);
    m_inviteEmail->setPlaceholderText(i18nd("kio_protondrive", "Email address"));
    m_inviteRole = new QComboBox(membersGroup);
    m_inviteRole->addItem(i18nd("kio_protondrive", "Viewer"), QStringLiteral("viewer"));
    m_inviteRole->addItem(i18nd("kio_protondrive", "Editor"), QStringLiteral("editor"));
    m_inviteRole->addItem(i18nd("kio_protondrive", "Admin"), QStringLiteral("admin"));
    auto *inviteButton =
        new QPushButton(QIcon::fromTheme(QStringLiteral("list-add-user")), i18nd("kio_protondrive", "Invite"), membersGroup);
    connect(inviteButton, &QPushButton::clicked, this, &ShareDialog::inviteClicked);
    inviteLayout->addWidget(m_inviteEmail, 1);
    inviteLayout->addWidget(m_inviteRole);
    inviteLayout->addWidget(inviteButton);
    membersLayout->addLayout(inviteLayout);

    m_inviteMessage = new QLineEdit(membersGroup);
    m_inviteMessage->setPlaceholderText(i18nd("kio_protondrive", "Message (optional)"));
    membersLayout->addWidget(m_inviteMessage);

    layout->addWidget(membersGroup);

    auto *linkGroup = new QGroupBox(i18nd("kio_protondrive", "Public link"), this);
    auto *linkLayout = new QVBoxLayout(linkGroup);

    m_linkStatusLabel = new QLabel(linkGroup);
    m_linkStatusLabel->setWordWrap(true);
    linkLayout->addWidget(m_linkStatusLabel);

    auto *linkUrlLayout = new QHBoxLayout;
    m_linkUrl = new QLineEdit(linkGroup);
    m_linkUrl->setReadOnly(true);
    m_linkUrl->setVisible(false);
    m_copyLinkButton = new QPushButton(QIcon::fromTheme(QStringLiteral("edit-copy")), i18nd("kio_protondrive", "Copy"), linkGroup);
    m_copyLinkButton->setVisible(false);
    connect(m_copyLinkButton, &QPushButton::clicked, this, &ShareDialog::copyLinkClicked);
    linkUrlLayout->addWidget(m_linkUrl, 1);
    linkUrlLayout->addWidget(m_copyLinkButton);
    linkLayout->addLayout(linkUrlLayout);

    auto *linkOptionsLayout = new QHBoxLayout;
    m_linkRole = new QComboBox(linkGroup);
    // `sharing set-url --help` only lists viewer/editor for a public link,
    // unlike invite's broader viewer/editor/admin/inherited.
    m_linkRole->addItem(i18nd("kio_protondrive", "Viewer"), QStringLiteral("viewer"));
    m_linkRole->addItem(i18nd("kio_protondrive", "Editor"), QStringLiteral("editor"));
    m_linkPassword = new QLineEdit(linkGroup);
    m_linkPassword->setPlaceholderText(i18nd("kio_protondrive", "Password (optional)"));
    m_linkPassword->setEchoMode(QLineEdit::Password);
    linkOptionsLayout->addWidget(m_linkRole);
    linkOptionsLayout->addWidget(m_linkPassword, 1);
    linkLayout->addLayout(linkOptionsLayout);

    auto *expirationLayout = new QHBoxLayout;
    m_linkHasExpiration = new QCheckBox(i18nd("kio_protondrive", "Expires on:"), linkGroup);
    m_linkExpiration = new QDateEdit(QDate::currentDate().addDays(7), linkGroup);
    m_linkExpiration->setCalendarPopup(true);
    m_linkExpiration->setEnabled(false);
    connect(m_linkHasExpiration, &QCheckBox::toggled, m_linkExpiration, &QDateEdit::setEnabled);
    expirationLayout->addWidget(m_linkHasExpiration);
    expirationLayout->addWidget(m_linkExpiration, 1);
    linkLayout->addLayout(expirationLayout);

    auto *linkButtonsLayout = new QHBoxLayout;
    auto *createOrUpdateButton = new QPushButton(QIcon::fromTheme(QStringLiteral("insert-link")),
                                                  i18nd("kio_protondrive", "Create/Update Link"),
                                                  linkGroup);
    connect(createOrUpdateButton, &QPushButton::clicked, this, &ShareDialog::createOrUpdateLinkClicked);
    m_removeLinkButton =
        new QPushButton(QIcon::fromTheme(QStringLiteral("list-remove")), i18nd("kio_protondrive", "Remove Link"), linkGroup);
    connect(m_removeLinkButton, &QPushButton::clicked, this, &ShareDialog::removeLinkClicked);
    linkButtonsLayout->addWidget(createOrUpdateButton);
    linkButtonsLayout->addWidget(m_removeLinkButton);
    linkLayout->addLayout(linkButtonsLayout);

    layout->addWidget(linkGroup);

    m_editorsCanShareLabel = new QLabel(this);
    m_editorsCanShareLabel->setWordWrap(true);
    layout->addWidget(m_editorsCanShareLabel);

    auto *buttons = new QDialogButtonBox(QDialogButtonBox::Close, this);
    connect(buttons, &QDialogButtonBox::rejected, this, &QDialog::reject);
    connect(buttons, &QDialogButtonBox::accepted, this, &QDialog::accept);
    layout->addWidget(buttons);

    reloadStatus();
}

void ShareDialog::reloadStatus()
{
    QGuiApplication::setOverrideCursor(Qt::WaitCursor);

    // Public-link presence: `sharing status` doesn't carry it at all (see
    // core/src/entry.rs's SharingStatus doc comment) — stat's own
    // isSharedByUrl is the only currently-known source (see this class's
    // header doc comment on why "Remove Link" stays available without
    // knowing the URL itself).
    try {
        const FfiEntry entry = stat_path(m_remotePath.toStdString());
        m_hasPublicLink = entry.is_shared_by_url;
    } catch (const rust::Error &) {
        // Non-fatal — sharing_status below still populates the rest of the
        // dialog; link state just falls back to "unknown", treated as
        // absent until the user creates/updates one.
    }

    m_memberList->clear();
    try {
        const FfiSharingStatus status = sharing_status(m_remotePath.toStdString());
        for (const FfiShareMember &member : status.members) {
            const QString email = toQString(member.email);
            QString label = QStringLiteral("%1 (%2)").arg(email, roleLabel(toQString(member.role)));
            if (member.pending) {
                label += QLatin1Char(' ') + i18nd("kio_protondrive", "— pending");
            }
            auto *item = new QListWidgetItem(label, m_memberList);
            item->setData(Qt::UserRole, email);
        }
        m_editorsCanShareLabel->setText(status.editors_can_share
                                             ? i18nd("kio_protondrive", "Editors can invite others to this item.")
                                             : i18nd("kio_protondrive", "Only you can invite others to this item."));
    } catch (const rust::Error &error) {
        QGuiApplication::restoreOverrideCursor();
        QMessageBox::warning(this, i18nd("kio_protondrive", "Sharing"), QString::fromUtf8(error.what()));
        QGuiApplication::setOverrideCursor(Qt::WaitCursor);
    }

    m_linkUrl->clear();
    m_linkUrl->setVisible(false);
    m_copyLinkButton->setVisible(false);
    m_removeLinkButton->setEnabled(m_hasPublicLink);
    m_linkStatusLabel->setText(
        m_hasPublicLink
            ? i18nd("kio_protondrive", "A public link is currently active. Use \"Create/Update Link\" to view or change it.")
            : i18nd("kio_protondrive", "No public link."));

    QGuiApplication::restoreOverrideCursor();
}

void ShareDialog::inviteClicked()
{
    const QString email = m_inviteEmail->text().trimmed();
    if (email.isEmpty()) {
        return;
    }
    const QString role = m_inviteRole->currentData().toString();
    const QString message = m_inviteMessage->text();

    QGuiApplication::setOverrideCursor(Qt::WaitCursor);
    try {
        sharing_invite(m_remotePath.toStdString(), email.toStdString(), role.toStdString(), message.toStdString());
        QGuiApplication::restoreOverrideCursor();
        m_inviteEmail->clear();
        m_inviteMessage->clear();
    } catch (const rust::Error &error) {
        QGuiApplication::restoreOverrideCursor();
        QMessageBox::warning(this, i18nd("kio_protondrive", "Sharing"), QString::fromUtf8(error.what()));
        return;
    }
    reloadStatus();
}

void ShareDialog::removeSelectedMemberClicked()
{
    const QList<QListWidgetItem *> selected = m_memberList->selectedItems();
    if (selected.isEmpty()) {
        return;
    }
    const QString email = selected.first()->data(Qt::UserRole).toString();

    QGuiApplication::setOverrideCursor(Qt::WaitCursor);
    try {
        sharing_remove_member(m_remotePath.toStdString(), email.toStdString());
        QGuiApplication::restoreOverrideCursor();
    } catch (const rust::Error &error) {
        QGuiApplication::restoreOverrideCursor();
        QMessageBox::warning(this, i18nd("kio_protondrive", "Sharing"), QString::fromUtf8(error.what()));
        return;
    }
    reloadStatus();
}

void ShareDialog::createOrUpdateLinkClicked()
{
    const QString role = m_linkRole->currentData().toString();
    const QString password = m_linkPassword->text();
    const QString expiration = m_linkHasExpiration->isChecked() ? m_linkExpiration->date().toString(Qt::ISODate) : QString();

    QGuiApplication::setOverrideCursor(Qt::WaitCursor);
    try {
        const FfiPublicLink link = sharing_set_link(
            m_remotePath.toStdString(), role.toStdString(), password.toStdString(), expiration.toStdString());
        QGuiApplication::restoreOverrideCursor();
        m_hasPublicLink = true;
        m_linkUrl->setText(toQString(link.url));
        m_linkUrl->setVisible(true);
        m_copyLinkButton->setVisible(true);
        m_removeLinkButton->setEnabled(true);
        m_linkStatusLabel->setText(i18nd("kio_protondrive", "Public link active."));
    } catch (const rust::Error &error) {
        QGuiApplication::restoreOverrideCursor();
        QMessageBox::warning(this, i18nd("kio_protondrive", "Sharing"), QString::fromUtf8(error.what()));
    }
}

void ShareDialog::removeLinkClicked()
{
    QGuiApplication::setOverrideCursor(Qt::WaitCursor);
    try {
        sharing_remove_link(m_remotePath.toStdString());
        QGuiApplication::restoreOverrideCursor();
        m_hasPublicLink = false;
        m_linkUrl->clear();
        m_linkUrl->setVisible(false);
        m_copyLinkButton->setVisible(false);
        m_removeLinkButton->setEnabled(false);
        m_linkStatusLabel->setText(i18nd("kio_protondrive", "No public link."));
    } catch (const rust::Error &error) {
        QGuiApplication::restoreOverrideCursor();
        QMessageBox::warning(this, i18nd("kio_protondrive", "Sharing"), QString::fromUtf8(error.what()));
    }
}

void ShareDialog::copyLinkClicked()
{
    QGuiApplication::clipboard()->setText(m_linkUrl->text());
}
