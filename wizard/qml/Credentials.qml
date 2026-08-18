import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.Page {
    id: page
    title: qsTr("Credential storage")

    // See Auth.qml's comment on why this is passed explicitly instead of
    // read back via Window.window.
    property QtObject app: null

    property bool checking: true
    property bool passInstalled: false
    property bool gpgInstalled: false
    property bool hasKey: false
    property bool settingUp: false
    property bool saving: false
    property string errorText: ""
    property bool useUnsafeFile: true

    function refreshStatus() {
        checking = true;
        page.app.apiGet("/credentials-status", function (ok, data) {
            checking = false;
            passInstalled = ok && !!data.pass_installed;
            gpgInstalled = ok && !!data.gpg_installed;
            hasKey = ok && !!data.has_key;
        });
    }

    Component.onCompleted: refreshStatus()

    // Saved here rather than at Finish.qml — this choice has to be
    // committed (both to daemon.toml and to this wizard process's own
    // PROTON_DRIVE_CREDENTIALS_STORE, see route_save_config's doc comment)
    // before Auth.qml runs `proton-drive auth login`, so the login lands in
    // the same store the daemon will actually read from. Otherwise it'd
    // always go to the CLI's own default (the desktop keyring) no matter
    // what's picked here, and the user would have to sign in a second time
    // to fix it.
    function goNext() {
        page.app.chosenCredentialsStore = page.useUnsafeFile ? "" : "pass";
        page.saving = true;
        page.errorText = "";
        // Sent explicitly even when empty (meaning "clear it" / unsafe_file)
        // — route_save_config only overwrites a setting whose param is
        // present at all, to avoid this call clobbering settings saved by
        // other pages (e.g. CacheRetention.qml's cache_retention_days).
        var params = "credentials_store=" + encodeURIComponent(page.app.chosenCredentialsStore);
        page.app.apiPost("/save-config?" + params, function (ok, data) {
            page.saving = false;
            if (!ok || !data.ok) {
                page.errorText = data.error || qsTr("Could not save the configuration.");
                return;
            }
            page.app.pageStack.push(Qt.resolvedUrl("Auth.qml"), {app: page.app});
        });
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Kirigami.Units.largeSpacing * 2
        spacing: Kirigami.Units.largeSpacing

        Label {
            text: qsTr("By default the background sync daemon keeps your Proton Drive session in a plain file (readable only by you, but not encrypted). You can switch it to a GPG-encrypted store (`pass`) instead — or just skip this and change it later.")
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }

        Kirigami.LoadingPlaceholder {
            visible: page.checking
            text: qsTr("Checking…")
            Layout.fillWidth: true
        }

        ColumnLayout {
            visible: !page.checking
            Layout.fillWidth: true
            spacing: Kirigami.Units.smallSpacing

            RadioButton {
                text: qsTr("Keep the default (unsafe_file)")
                checked: page.useUnsafeFile
                onCheckedChanged: if (checked) page.useUnsafeFile = true
            }
            RadioButton {
                id: passOption
                text: qsTr("Use pass (GPG-encrypted)")
                checked: !page.useUnsafeFile
                enabled: page.passInstalled && page.gpgInstalled
                onCheckedChanged: if (checked) page.useUnsafeFile = false
            }

            Label {
                visible: !page.passInstalled || !page.gpgInstalled
                text: qsTr("Requires `pass` and `gpg` to be installed first: sudo apt install pass gpg")
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
                Layout.leftMargin: Kirigami.Units.largeSpacing * 2
                opacity: 0.8
            }

            ColumnLayout {
                visible: passOption.checked && page.passInstalled && page.gpgInstalled && !page.hasKey
                Layout.fillWidth: true
                Layout.leftMargin: Kirigami.Units.largeSpacing * 2
                spacing: Kirigami.Units.smallSpacing

                Label {
                    text: qsTr("No usable GPG key found — enter an email to generate one (you'll be prompted for a passphrase separately):")
                    wrapMode: Text.WordWrap
                    Layout.fillWidth: true
                }
                RowLayout {
                    TextField {
                        id: emailField
                        Layout.fillWidth: true
                        placeholderText: "you@example.com"
                    }
                    Button {
                        text: qsTr("Set up")
                        enabled: !page.settingUp && emailField.text.indexOf("@") > 0
                        onClicked: {
                            page.settingUp = true;
                            page.errorText = "";
                            page.app.apiPost("/setup-pass?email=" + encodeURIComponent(emailField.text), function (ok, data) {
                                page.settingUp = false;
                                if (!ok || !data.ok) {
                                    page.errorText = data.error || qsTr("Could not set up pass.");
                                    return;
                                }
                                page.refreshStatus();
                            });
                        }
                    }
                }
                Kirigami.LoadingPlaceholder {
                    visible: page.settingUp
                    text: qsTr("Generating a key and initializing pass…")
                    Layout.fillWidth: true
                }
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

        Kirigami.LoadingPlaceholder {
            visible: page.saving
            text: qsTr("Saving…")
            Layout.fillWidth: true
        }

        RowLayout {
            Layout.alignment: Qt.AlignHCenter
            spacing: Kirigami.Units.largeSpacing

            Button {
                text: qsTr("Skip for now")
                flat: true
                enabled: !page.saving
                onClicked: {
                    page.useUnsafeFile = true;
                    page.goNext();
                }
            }
            Button {
                text: qsTr("Next")
                enabled: !page.saving && (page.useUnsafeFile || page.hasKey)
                onClicked: page.goNext()
            }
        }
    }
}
