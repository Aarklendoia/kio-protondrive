import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.Page {
    id: page
    title: qsTr("Dolphin favorite")

    // See Auth.qml's comment on why this is passed explicitly instead of
    // read back via Window.window.
    property QtObject app: null

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Kirigami.Units.largeSpacing * 2
        spacing: Kirigami.Units.largeSpacing

        Label {
            text: qsTr("Add Proton Drive to Dolphin's Places panel, for quick access to protondrive:/my-files.")
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }

        CheckBox {
            text: qsTr("Add to Dolphin's Places panel")
            checked: page.app.chosenAddFavorite
            onCheckedChanged: page.app.chosenAddFavorite = checked
        }

        Item {
            Layout.fillHeight: true
        }

        Button {
            text: qsTr("Next")
            Layout.alignment: Qt.AlignHCenter
            onClicked: page.app.pageStack.push(Qt.resolvedUrl("CacheRetention.qml"), {app: page.app})
        }
    }
}
