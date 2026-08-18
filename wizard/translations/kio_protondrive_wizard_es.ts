<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE TS>
<TS version="2.1" language="es">
<context>
    <name>Auth</name>
    <message>
        <location filename="../qml/Auth.qml" line="8"/>
        <source>Sign in</source>
        <translation>Iniciar sesión</translation>
    </message>
    <message>
        <location filename="../qml/Auth.qml" line="39"/>
        <source>Proton Drive needs you to sign in once. This opens your browser — the sync daemon uses the same session afterward.</source>
        <translation>Proton Drive necesita que inicies sesión una vez. Esto abrirá tu navegador — el servicio de sincronización usará después la misma sesión.</translation>
    </message>
    <message>
        <location filename="../qml/Auth.qml" line="46"/>
        <source>Waiting for you to finish signing in in your browser…</source>
        <translation>Esperando a que termines de iniciar sesión en tu navegador…</translation>
    </message>
    <message>
        <location filename="../qml/Auth.qml" line="46"/>
        <source>Checking…</source>
        <translation>Comprobando…</translation>
    </message>
    <message>
        <location filename="../qml/Auth.qml" line="61"/>
        <source>Already signed in to Proton Drive.</source>
        <translation>Ya has iniciado sesión en Proton Drive.</translation>
    </message>
    <message>
        <location filename="../qml/Auth.qml" line="67"/>
        <source>Sign in with your browser</source>
        <translation>Iniciar sesión con el navegador</translation>
    </message>
    <message>
        <location filename="../qml/Auth.qml" line="76"/>
        <source>Sign-in did not complete.</source>
        <translation>El inicio de sesión no se completó.</translation>
    </message>
    <message>
        <location filename="../qml/Auth.qml" line="94"/>
        <source>Next</source>
        <translation>Siguiente</translation>
    </message>
</context>
<context>
    <name>Credentials</name>
    <message>
        <location filename="../qml/Credentials.qml" line="8"/>
        <source>Credential storage</source>
        <translation>Almacenamiento de credenciales</translation>
    </message>
    <message>
        <location filename="../qml/Credentials.qml" line="53"/>
        <source>Could not save the configuration.</source>
        <translation>No se pudo guardar la configuración.</translation>
    </message>
    <message>
        <location filename="../qml/Credentials.qml" line="66"/>
        <source>By default the background sync daemon keeps your Proton Drive session in a plain file (readable only by you, but not encrypted). You can switch it to a GPG-encrypted store (`pass`) instead — or just skip this and change it later.</source>
        <translation>De forma predeterminada, el servicio de sincronización en segundo plano guarda tu sesión de Proton Drive en un archivo sin cifrar (solo tú puedes leerlo, pero no está cifrado). Puedes cambiarlo por un almacén cifrado con GPG («pass») — o simplemente omitir este paso y cambiarlo más tarde.</translation>
    </message>
    <message>
        <location filename="../qml/Credentials.qml" line="73"/>
        <source>Checking…</source>
        <translation>Comprobando…</translation>
    </message>
    <message>
        <location filename="../qml/Credentials.qml" line="83"/>
        <source>Keep the default (unsafe_file)</source>
        <translation>Mantener el valor predeterminado (unsafe_file)</translation>
    </message>
    <message>
        <location filename="../qml/Credentials.qml" line="89"/>
        <source>Use pass (GPG-encrypted)</source>
        <translation>Usar pass (cifrado con GPG)</translation>
    </message>
    <message>
        <location filename="../qml/Credentials.qml" line="97"/>
        <source>Requires `pass` and `gpg` to be installed first: sudo apt install pass gpg</source>
        <translation>Requiere que «pass» y «gpg» estén instalados previamente: sudo apt install pass gpg</translation>
    </message>
    <message>
        <location filename="../qml/Credentials.qml" line="111"/>
        <source>No usable GPG key found — enter an email to generate one (you&apos;ll be prompted for a passphrase separately):</source>
        <translation>No se encontró ninguna clave GPG utilizable — introduce un correo electrónico para generar una (se te pedirá una frase de contraseña por separado):</translation>
    </message>
    <message>
        <location filename="../qml/Credentials.qml" line="122"/>
        <source>Set up</source>
        <translation>Configurar</translation>
    </message>
    <message>
        <location filename="../qml/Credentials.qml" line="130"/>
        <source>Could not set up pass.</source>
        <translation>No se pudo configurar pass.</translation>
    </message>
    <message>
        <location filename="../qml/Credentials.qml" line="140"/>
        <source>Generating a key and initializing pass…</source>
        <translation>Generando una clave e inicializando pass…</translation>
    </message>
    <message>
        <location filename="../qml/Credentials.qml" line="160"/>
        <source>Saving…</source>
        <translation>Guardando…</translation>
    </message>
    <message>
        <location filename="../qml/Credentials.qml" line="169"/>
        <source>Skip for now</source>
        <translation>Omitir por ahora</translation>
    </message>
    <message>
        <location filename="../qml/Credentials.qml" line="178"/>
        <source>Next</source>
        <translation>Siguiente</translation>
    </message>
</context>
<context>
    <name>Favorite</name>
    <message>
        <location filename="../qml/Favorite.qml" line="8"/>
        <source>Dolphin favorite</source>
        <translation>Favorito de Dolphin</translation>
    </message>
    <message>
        <location filename="../qml/Favorite.qml" line="20"/>
        <source>Add Proton Drive to Dolphin&apos;s Places panel, for quick access to protondrive:/my-files.</source>
        <translation>Añadir Proton Drive al panel de lugares de Dolphin, para acceder rápidamente a protondrive:/my-files.</translation>
    </message>
    <message>
        <location filename="../qml/Favorite.qml" line="26"/>
        <source>Add to Dolphin&apos;s Places panel</source>
        <translation>Añadir al panel de lugares de Dolphin</translation>
    </message>
    <message>
        <location filename="../qml/Favorite.qml" line="36"/>
        <source>Next</source>
        <translation>Siguiente</translation>
    </message>
</context>
<context>
    <name>CacheRetention</name>
    <message>
        <location filename="../qml/CacheRetention.qml" line="8"/>
        <source>Local cache</source>
        <translation>Caché local</translation>
    </message>
    <message>
        <location filename="../qml/CacheRetention.qml" line="23"/>
        <source>Files you open through Dolphin stay available locally afterward, so reopening them is instant. A file not opened again within this many days is automatically removed from the local cache — pinned files are never removed this way.</source>
        <translation>Los archivos que abre a través de Dolphin permanecen disponibles localmente después, para que volver a abrirlos sea instantáneo. Un archivo que no se vuelva a abrir dentro de este número de días se elimina automáticamente de la caché local — los archivos anclados nunca se eliminan de esta manera.</translation>
    </message>
    <message>
        <location filename="../qml/CacheRetention.qml" line="32"/>
        <source>Keep unused files locally for:</source>
        <translation>Mantener los archivos no utilizados localmente durante:</translation>
    </message>
    <message>
        <location filename="../qml/CacheRetention.qml" line="44"/>
        <source>days</source>
        <translation>días</translation>
    </message>
    <message>
        <location filename="../qml/CacheRetention.qml" line="61"/>
        <source>Next</source>
        <translation>Siguiente</translation>
    </message>
    <message>
        <location filename="../qml/CacheRetention.qml" line="71"/>
        <source>Could not save the configuration.</source>
        <translation>No se pudo guardar la configuración.</translation>
    </message>
</context>
<context>
    <name>Finish</name>
    <message>
        <location filename="../qml/Finish.qml" line="8"/>
        <source>All set</source>
        <translation>Todo listo</translation>
    </message>
    <message>
        <location filename="../qml/Finish.qml" line="15"/>
        <source>Finishing setup…</source>
        <translation>Finalizando la configuración…</translation>
    </message>
    <message>
        <location filename="../qml/Finish.qml" line="29"/>
        <source>Adding Proton Drive to Dolphin&apos;s Places…</source>
        <translation>Añadiendo Proton Drive a los lugares de Dolphin…</translation>
    </message>
    <message>
        <location filename="../qml/Finish.qml" line="36"/>
        <source>Starting the sync daemon…</source>
        <translation>Iniciando el servicio de sincronización…</translation>
    </message>
    <message>
        <location filename="../qml/Finish.qml" line="44"/>
        <source>Setup complete. The sync daemon will pick up your settings shortly — restart it yourself if you&apos;d rather not wait: systemctl --user restart kio-protondrive-sync-daemon</source>
        <translation>Configuración completada. El servicio de sincronización aplicará tus ajustes en breve — reinícialo tú mismo si prefieres no esperar: systemctl --user restart kio-protondrive-sync-daemon</translation>
    </message>
    <message>
        <location filename="../qml/Finish.qml" line="47"/>
        <source>Setup complete — Proton Drive sync is running.</source>
        <translation>Configuración completada — la sincronización de Proton Drive está en marcha.</translation>
    </message>
    <message>
        <location filename="../qml/Finish.qml" line="94"/>
        <source>Close</source>
        <translation>Cerrar</translation>
    </message>
</context>
<context>
    <name>Welcome</name>
    <message>
        <location filename="../qml/Welcome.qml" line="8"/>
        <source>Welcome</source>
        <translation>Bienvenido</translation>
    </message>
    <message>
        <location filename="../qml/Welcome.qml" line="31"/>
        <source>Set up Proton Drive</source>
        <translation>Configurar Proton Drive</translation>
    </message>
    <message>
        <location filename="../qml/Welcome.qml" line="38"/>
        <source>This will sign you in to Proton Drive and let you choose how the background sync daemon stores your session. Once set up, you can pin any file or folder in Dolphin to keep it available locally.</source>
        <translation>Esto te conectará a Proton Drive y te permitirá elegir cómo almacena tu sesión el servicio de sincronización en segundo plano. Una vez configurado, podrás anclar cualquier archivo o carpeta en Dolphin para mantenerlo disponible localmente.</translation>
    </message>
    <message>
        <location filename="../qml/Welcome.qml" line="49"/>
        <source>Get Started</source>
        <translation>Empezar</translation>
    </message>
</context>
<context>
    <name>main</name>
    <message>
        <location filename="../qml/main.qml" line="9"/>
        <source>Proton Drive Setup</source>
        <translation>Configuración de Proton Drive</translation>
    </message>
</context>
</TS>
