# New Line: Mac Prep

Use this on the Mac that will provide the hardware identity for a new iMessage line.

## Requirements

- macOS
- Xcode command line tools installed
- This repository cloned locally
- The Apple ID for the line can successfully use Messages/FaceTime on Apple services

## Extract `hwconfig.plist`

From the repository root:

```bash
mkdir -p private
xcrun swift scripts/extract_hwconfig.swift > private/hwconfig.plist
plutil -p private/hwconfig.plist
```

Copy the base64 value for hosted deployment:

```bash
base64 < private/hwconfig.plist | tr -d '\n' | pbcopy
```

Keep `private/hwconfig.plist` and the copied base64 value secret. They identify the Mac-like hardware profile used by the daemon.

## Before Provisioning

Confirm the Apple ID works normally:

1. Sign into the Apple ID on an Apple device or trusted browser.
2. Confirm two-factor authentication is available.
3. Confirm the Apple ID is allowed to use iMessage/FaceTime.
4. If this line needs a phone number handle, confirm the phone number is actually registered with iMessage. Email/password alone will usually only give an email handle.
