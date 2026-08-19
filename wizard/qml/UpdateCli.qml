import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// The *only* page shown when main.rs was launched with `--update-cli` (see
// main.qml) — spawned by daemon::version_check::offer_wizard_update once it
// detects a newer proton-drive release, instead of the full onboarding
// flow this wizard normally runs. Deliberately a dead end (no "Next" into
// Credentials.qml/Auth.qml/...): closing this window is how the user
// declines ("Later"), same as dismissing a desktop notification.
Kirigami.Page {
    id: page
    title: qsTr("Update available")

    property QtObject app: null

    property bool updating: false
    property bool updated: false
    property bool restarted: false
    property string errorText: ""

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Kirigami.Units.largeSpacing * 2
        spacing: Kirigami.Units.largeSpacing

        Kirigami.Icon {
            source: "system-software-update"
            Layout.preferredWidth: Kirigami.Units.iconSizes.huge
            Layout.preferredHeight: Kirigami.Units.iconSizes.huge
            Layout.alignment: Qt.AlignHCenter
        }

        Label {
            text: qsTr("A newer version of the Proton Drive CLI is available.")
            font.pixelSize: 22
            font.bold: true
            wrapMode: Text.WordWrap
            horizontalAlignment: Text.AlignHCenter
            Layout.alignment: Qt.AlignHCenter
            Layout.fillWidth: true
        }

        Kirigami.LoadingPlaceholder {
            visible: page.updating
            text: qsTr("Downloading and verifying the update…")
            Layout.fillWidth: true
        }

        RowLayout {
            visible: !page.updating && page.updated
            Layout.alignment: Qt.AlignHCenter
            spacing: Kirigami.Units.smallSpacing

            Kirigami.Icon {
                source: "checkmark"
                Layout.preferredWidth: Kirigami.Units.iconSizes.medium
                Layout.preferredHeight: Kirigami.Units.iconSizes.medium
            }
            Label {
                text: page.restarted
                      ? qsTr("Updated — Proton Drive sync has restarted.")
                      : qsTr("Updated.")
            }
        }

        Button {
            visible: !page.updating && !page.updated
            text: qsTr("Update now")
            Layout.alignment: Qt.AlignHCenter
            onClicked: {
                page.updating = true;
                page.errorText = "";
                page.app.apiPost("/cli-install", function (ok, data) {
                    page.updated = ok && !!data.ok;
                    if (!page.updated) {
                        page.updating = false;
                        page.errorText = (data && data.error) || qsTr("Update failed.");
                        return;
                    }
                    page.app.apiPost("/restart-daemon", function (restartOk, restartData) {
                        page.updating = false;
                        page.restarted = restartOk && !!restartData.ok;
                    });
                });
            }
        }

        Label {
            visible: page.errorText !== ""
            text: page.errorText
            color: Kirigami.Theme.negativeTextColor
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }

        Item {
            Layout.fillHeight: true
        }

        Button {
            visible: !page.updated
            text: qsTr("Later")
            Layout.alignment: Qt.AlignHCenter
            onClicked: Qt.quit()
        }

        Button {
            visible: page.updated
            text: qsTr("Close")
            Layout.alignment: Qt.AlignHCenter
            onClicked: Qt.quit()
        }
    }
}
