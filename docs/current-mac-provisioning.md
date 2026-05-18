# Current Mac provisioning path

The old Beeper `mac-registration-provider` helper is version-offset based and fails on current macOS 26 builds.

For this project, use the OpenBubbles-style direct hardware extraction path instead:

```bash
cd "/Users/rajtripathi/Keep Repo and workers/imessage-api"
mkdir -p private
xcrun swift scripts/extract_hwconfig.swift > private/hwconfig.plist
plutil -p private/hwconfig.plist
base64 < private/hwconfig.plist | tr -d '\n' | pbcopy
```

This mirrors the direct I/O Registry extraction used by OpenBubbles' `Mac-Hardware-Info` app and writes the
`MacOSConfig` plist shape expected by our daemon.

At runtime, Railway uses the OpenBubbles build-modules validation helper to turn this saved hardware config into
fresh signed validation data when Apple registration needs it.

When pasting the environment value into Railway, use `pbcopy` as above rather than copying terminal output manually;
that avoids accidentally including a shell prompt character.
