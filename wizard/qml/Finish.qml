import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.Page {
    id: page
    title: qsTr("All set")

    // See Auth.qml's comment on why this is passed explicitly instead of
    // read back via Window.window.
    property QtObject app: null

    property bool working: true
    property string statusText: qsTr("Saving your settings…")
    property string errorText: ""

    Component.onCompleted: saveConfig()

    function saveConfig() {
        var root = page.app;
        var params = "";
        if (root.chosenCredentialsStore !== "")
            params = "credentials_store=" + encodeURIComponent(root.chosenCredentialsStore);

        root.apiPost("/save-config?" + params, function (ok, data) {
            if (!ok || !data.ok) {
                page.working = false;
                page.errorText = data.error || qsTr("Could not save the configuration.");
                return;
            }
            addFavoriteIfNeeded();
        });
    }

    function addFavoriteIfNeeded() {
        var root = page.app;
        if (!root.chosenAddFavorite) {
            restartDaemon();
            return;
        }
        page.statusText = qsTr("Adding Proton Drive to Dolphin's Places…");
        root.apiPost("/add-favorite", function () {
            restartDaemon();
        });
    }

    function restartDaemon() {
        page.statusText = qsTr("Starting the sync daemon…");
        page.app.apiPost("/restart-daemon", function (ok, data) {
            page.working = false;
            if (!ok || !data.ok) {
                // Not fatal — the service will pick up daemon.toml on its
                // own next start (or #37's StartLimitBurst backoff ends and
                // systemd retries it), the user just doesn't get an
                // immediate confirmation.
                page.statusText = qsTr("Setup complete. The sync daemon will pick up your settings shortly — restart it yourself if you'd rather not wait: systemctl --user restart kio-protondrive-sync-daemon");
                return;
            }
            page.statusText = qsTr("Setup complete — Proton Drive sync is running.");
        });
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Kirigami.Units.largeSpacing * 2
        spacing: Kirigami.Units.largeSpacing

        Kirigami.LoadingPlaceholder {
            visible: page.working
            text: page.statusText
            Layout.fillWidth: true
        }

        ColumnLayout {
            visible: !page.working && page.errorText === ""
            Layout.alignment: Qt.AlignHCenter
            spacing: Kirigami.Units.smallSpacing

            Kirigami.Icon {
                source: "checkmark"
                Layout.preferredWidth: Kirigami.Units.iconSizes.huge
                Layout.preferredHeight: Kirigami.Units.iconSizes.huge
                Layout.alignment: Qt.AlignHCenter
            }
            Label {
                text: page.statusText
                wrapMode: Text.WordWrap
                horizontalAlignment: Text.AlignHCenter
                Layout.fillWidth: true
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
            text: qsTr("Close")
            enabled: !page.working
            Layout.alignment: Qt.AlignHCenter
            onClicked: Qt.quit()
        }
    }
}
