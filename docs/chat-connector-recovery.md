# Chat connector recovery

The Unix installer configures a saved Chat connection to recover in the signed-in
user's session. It does not configure GPU inference services or change model support.

* Linux: a systemd user service and a desktop `mundusx://reconnect` URL handler.
  A desktop session with a working systemd user manager is required for automatic
  recovery. Headless machines can run `mundusx connect` in their own service setup.
* macOS: a LaunchAgent and a user Applications/MundusX Reconnect.app URL handler.
  The package includes the Chat connector and agent server. With no signed-in GUI
  user at package installation, run the registration command below after login.

For existing binaries, register without downloading them again:

```sh
INSTALL_DIR=/path/to/installed/binaries bash install.sh --configure-chat-service
```

On a new installation, run `mundusx connect` once and approve the computer in Chat.
Existing installations reuse `.mundusx/chat-connection.json`. Reconnect starts a
stopped service; it does not kill a running connector, rotate credentials, or download
an installer. Browsers may require permission to open the registered application.
The Unix process lock prevents a second connector using the same data directory.

Services start at user login and restart after failures. This does not keep a laptop
online while asleep or a user service alive after logout. The connector handles
network retries after connectivity returns. Automatic registration can be skipped
with `MUNDUSX_SKIP_CHAT_SERVICE=1`.

Native CI checks service generation, macOS URL bundle compilation, pairing
preservation, URI validation, and the Unix process lock. Actual browser launch,
sleep/wake, and logout/login recovery still need device acceptance testing before
claiming a released installer supports those scenarios.
