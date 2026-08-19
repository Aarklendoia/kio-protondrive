import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// Only ever pushed by Welcome.qml, and only when its own /cli-status check
// already found no `proton-drive` on $PATH — Credentials.qml/Auth.qml right
// after this need the real CLI present (Auth.qml runs `proton-drive auth
// login`), so this page is the gate that makes sure it exists before the
// rest of the flow can proceed.
Kirigami.Page {
    id: page
    title: qsTr("Install the Proton Drive CLI")

    property QtObject app: null

    property bool installing: false
    property bool installed: false
    property string errorText: ""

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Kirigami.Units.largeSpacing * 2
        spacing: Kirigami.Units.largeSpacing

        Label {
            text: qsTr("kio-protondrive needs the official Proton Drive CLI, which isn't installed yet. It can be downloaded and installed to ~/.local/bin automatically.")
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }

        Kirigami.LoadingPlaceholder {
            visible: page.installing
            text: qsTr("Downloading and verifying the Proton Drive CLI…")
            Layout.fillWidth: true
        }

        RowLayout {
            visible: !page.installing && page.installed
            Layout.alignment: Qt.AlignHCenter
            spacing: Kirigami.Units.smallSpacing

            Kirigami.Icon {
                source: "checkmark"
                Layout.preferredWidth: Kirigami.Units.iconSizes.medium
                Layout.preferredHeight: Kirigami.Units.iconSizes.medium
            }
            Label {
                text: qsTr("Proton Drive CLI installed.")
            }
        }

        Button {
            visible: !page.installing && !page.installed
            text: qsTr("Install now")
            Layout.alignment: Qt.AlignHCenter
            onClicked: {
                page.installing = true;
                page.errorText = "";
                page.app.apiPost("/cli-install", function (ok, data) {
                    page.installing = false;
                    page.installed = ok && !!data.ok;
                    if (!page.installed)
                        page.errorText = (data && data.error) || qsTr("Installation failed.");
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

        Label {
            visible: page.errorText !== ""
            text: qsTr("You can also download it yourself from proton.me/drive/download and run this wizard again.")
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
            font.italic: true
        }

        Item {
            Layout.fillHeight: true
        }

        Button {
            text: qsTr("Next")
            enabled: page.installed
            Layout.alignment: Qt.AlignHCenter
            onClicked: page.app.pageStack.push(Qt.resolvedUrl("Credentials.qml"), {app: page.app})
        }
    }
}
