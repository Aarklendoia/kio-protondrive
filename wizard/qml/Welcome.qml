import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.Page {
    id: page
    title: qsTr("Welcome")

    // Set explicitly by whoever pushes this page (see main.qml) rather than
    // read back via the Window.window attached property — that's still
    // null at Component.onCompleted time for a freshly-pushed page, which
    // matters for the other wizard pages that call the API on load; kept
    // consistent here even though Welcome itself only needs `app` once the
    // user clicks Next.
    property QtObject app: null

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Kirigami.Units.largeSpacing * 2
        spacing: Kirigami.Units.largeSpacing

        Kirigami.Icon {
            source: "folder-cloud"
            Layout.preferredWidth: Kirigami.Units.iconSizes.huge
            Layout.preferredHeight: Kirigami.Units.iconSizes.huge
            Layout.alignment: Qt.AlignHCenter
        }

        Label {
            text: qsTr("Set up Proton Drive")
            font.pixelSize: 22
            font.bold: true
            Layout.alignment: Qt.AlignHCenter
        }

        Label {
            text: qsTr("This will sign you in to Proton Drive and let you choose how the background sync daemon stores your session. Once set up, you can pin any file or folder in Dolphin to keep it available locally.")
            wrapMode: Text.WordWrap
            horizontalAlignment: Text.AlignHCenter
            Layout.fillWidth: true
        }

        Item {
            Layout.fillHeight: true
        }

        Button {
            text: qsTr("Get Started")
            Layout.alignment: Qt.AlignHCenter
            onClicked: page.app.pageStack.push(Qt.resolvedUrl("Credentials.qml"), {app: page.app})
        }
    }
}
