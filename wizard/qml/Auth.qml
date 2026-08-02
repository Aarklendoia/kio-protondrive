import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.Page {
    id: page
    title: qsTr("Sign in")

    // Set explicitly by whoever pushes this page (see main.qml/Welcome.qml)
    // rather than read back via the Window.window attached property —
    // confirmed live that Window.window is still null at
    // Component.onCompleted time for a freshly-pushed PageRow page (even a
    // Qt.callLater tick later), so this page needs `app` available
    // synchronously since it calls the API right on load.
    property QtObject app: null

    property bool checking: true
    property bool authenticated: false
    property bool signingIn: false
    property string errorText: ""

    function checkStatus() {
        checking = true;
        page.app.apiGet("/session-status", function (ok, data) {
            checking = false;
            authenticated = ok && !!data.authenticated;
        });
    }

    Component.onCompleted: checkStatus()

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Kirigami.Units.largeSpacing * 2
        spacing: Kirigami.Units.largeSpacing

        Label {
            text: qsTr("Proton Drive needs you to sign in once. This opens your browser — the sync daemon uses the same session afterward.")
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }

        Kirigami.LoadingPlaceholder {
            visible: page.checking || page.signingIn
            text: page.signingIn ? qsTr("Waiting for you to finish signing in in your browser…") : qsTr("Checking…")
            Layout.fillWidth: true
        }

        RowLayout {
            visible: !page.checking && !page.signingIn && page.authenticated
            Layout.alignment: Qt.AlignHCenter
            spacing: Kirigami.Units.smallSpacing

            Kirigami.Icon {
                source: "checkmark"
                Layout.preferredWidth: Kirigami.Units.iconSizes.medium
                Layout.preferredHeight: Kirigami.Units.iconSizes.medium
            }
            Label {
                text: qsTr("Already signed in to Proton Drive.")
            }
        }

        Button {
            visible: !page.checking && !page.signingIn && !page.authenticated
            text: qsTr("Sign in with your browser")
            Layout.alignment: Qt.AlignHCenter
            onClicked: {
                page.signingIn = true;
                page.errorText = "";
                page.app.apiPost("/auth-login", function (ok, data) {
                    page.signingIn = false;
                    page.authenticated = ok && !!data.authenticated;
                    if (!page.authenticated)
                        page.errorText = data.error || qsTr("Sign-in did not complete.");
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
            text: qsTr("Next")
            enabled: page.authenticated
            Layout.alignment: Qt.AlignHCenter
            onClicked: page.app.pageStack.push(Qt.resolvedUrl("Credentials.qml"), {app: page.app})
        }
    }
}
