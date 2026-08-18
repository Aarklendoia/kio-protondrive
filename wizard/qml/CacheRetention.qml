import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.Page {
    id: page
    title: qsTr("Local cache")

    // See Auth.qml's comment on why this is passed explicitly instead of
    // read back via Window.window.
    property QtObject app: null

    property bool saving: false
    property string errorText: ""

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Kirigami.Units.largeSpacing * 2
        spacing: Kirigami.Units.largeSpacing

        Label {
            text: qsTr("Files you open through Dolphin stay available locally afterward, so reopening them is instant. A file not opened again within this many days is automatically removed from the local cache — pinned files are never removed this way.")
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }

        RowLayout {
            spacing: Kirigami.Units.smallSpacing

            Label {
                text: qsTr("Keep unused files locally for:")
            }

            SpinBox {
                id: retentionSpinBox
                from: 1
                to: 365
                value: page.app.chosenCacheRetentionDays
                onValueChanged: page.app.chosenCacheRetentionDays = value
            }

            Label {
                text: qsTr("days")
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
            text: qsTr("Next")
            enabled: !page.saving
            Layout.alignment: Qt.AlignHCenter
            onClicked: {
                page.saving = true;
                page.errorText = "";
                var params = "cache_retention_days=" + page.app.chosenCacheRetentionDays;
                page.app.apiPost("/save-config?" + params, function (ok, data) {
                    page.saving = false;
                    if (!ok || !data.ok) {
                        page.errorText = data.error || qsTr("Could not save the configuration.");
                        return;
                    }
                    page.app.pageStack.push(Qt.resolvedUrl("Finish.qml"), {app: page.app});
                });
            }
        }
    }
}
