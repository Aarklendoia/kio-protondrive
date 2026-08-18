import QtQuick
import org.kde.kirigami as Kirigami

// Kirigami is already installed as part of the standard KDE/Qt6 stack this
// package targets (qml6-module-org-kde-kirigami) — no new dependency, same
// as linux-hello-rust's own wizard-style GUI.
Kirigami.ApplicationWindow {
    id: root
    title: qsTr("Proton Drive Setup")
    width: 520
    height: 640
    minimumWidth: 460
    minimumHeight: 560
    visible: true

    // Read from owner-only files under $XDG_RUNTIME_DIR, written by main.rs
    // before it launches qml6 — see main.rs's module doc comment for why
    // (Qt.environmentVariable isn't reliably readable from plain QML).
    property string ctrlPort: "0"
    property string ctrlToken: ""

    // Carried between pages as the user moves forward through the wizard —
    // simpler than threading them through each pushed page's constructor
    // arguments, and there's only ever one of these wizards open at a time.
    property string chosenCredentialsStore: ""
    property bool chosenAddFavorite: true
    property int chosenCacheRetentionDays: 30

    function apiUrl(path) {
        return "http://127.0.0.1:" + ctrlPort + path;
    }

    // Shared by apiGet/apiPost — the server's routes don't actually branch
    // on HTTP method (every route takes its parameters from the query
    // string, see main.rs's `route` dispatch), but using the right verb is
    // still the honest thing to send.
    function apiCall(method, path, onDone) {
        var xhr = new XMLHttpRequest();
        xhr.open(method, apiUrl(path), true);
        xhr.setRequestHeader("X-Kio-Protondrive-Wizard-Token", root.ctrlToken);
        xhr.onreadystatechange = function () {
            if (xhr.readyState !== XMLHttpRequest.DONE)
                return;
            var data = {};
            try {
                data = JSON.parse(xhr.responseText);
            } catch (e) {
                data = {error: "invalid response from the setup wizard's backend"};
            }
            onDone(xhr.status === 200, data);
        };
        xhr.send();
    }

    function apiGet(path, onDone) {
        apiCall("GET", path, onDone);
    }

    function apiPost(path, onDone) {
        apiCall("POST", path, onDone);
    }

    // Placeholder until Component.onCompleted below has read ctrlPort/
    // ctrlToken and replaces it — pushing Welcome.qml declaratively here
    // instead would risk it (and Auth.qml right after it) trying to call
    // the API before the token is known, since a child Item's
    // Component.onCompleted runs before its parent's.
    pageStack.initialPage: Kirigami.Page {}

    Component.onCompleted: {
        // Passed by main.rs as the trailing argument after "--" — the
        // already-resolved $XDG_RUNTIME_DIR (or its /run/user/<uid>
        // fallback), not a bare UID: reconstructing "/run/user/" + uid
        // here would silently diverge from main.rs's own fallback logic on
        // any system where $XDG_RUNTIME_DIR isn't exactly that (containers,
        // some display managers), leaving this page stuck below with no
        // visible error.
        var args = Qt.application.arguments;
        var runtimeDir = args.length > 0 ? args[args.length - 1] : "/run/user/0";

        var portXhr = new XMLHttpRequest();
        portXhr.open("GET", "file://" + runtimeDir + "/kio-protondrive-wizard-ctrl.port", false);
        portXhr.send();
        if (portXhr.responseText !== "")
            root.ctrlPort = portXhr.responseText.trim();

        var tokenXhr = new XMLHttpRequest();
        tokenXhr.open("GET", "file://" + runtimeDir + "/kio-protondrive-wizard-ctrl.token", false);
        tokenXhr.send();
        if (tokenXhr.responseText !== "")
            root.ctrlToken = tokenXhr.responseText.trim();

        // Passed explicitly as an initial property (applied before the
        // pushed page's Component.onCompleted runs) rather than relying on
        // each page reading it back via the Window.window attached
        // property — confirmed live that Window.window is still null at
        // Component.onCompleted time for a freshly-pushed PageRow page (and
        // even one Qt.callLater tick later), for reasons not fully pinned
        // down; passing `app` through explicitly sidesteps the question
        // entirely.
        root.pageStack.replace(Qt.resolvedUrl("Welcome.qml"), {app: root});
    }
}
