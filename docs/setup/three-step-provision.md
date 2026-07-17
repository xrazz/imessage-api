# Three-Step Provisioning Quickstart

Use this as the shortest path for adding one new iMessage/FaceTime line.

## 1. Extract `hwconfig.plist` On A Mac

Run this from the repository root on the Mac that will provide the hardware identity:

```bash
mkdir -p private
xcrun swift scripts/extract_hwconfig.swift > private/hwconfig.plist
plutil -p private/hwconfig.plist
base64 < private/hwconfig.plist | tr -d '\n' | pbcopy
```

The final command copies the base64 plist value to the clipboard. Use it as `HWCONFIG_PLIST_BASE64` on the daemon.

## 2. Configure The Hosted Services

Set the daemon env:

```text
DATA_DIR=/app/data
HWCONFIG_PLIST_BASE64=<base64-from-step-1>
```

Attach the daemon volume at:

```text
/app/data
```

Set the API env:

```text
API_KEY=<random-secret>
DAEMON_URL=http://<daemon-private-host>:8080
```

## 3. Provision The Apple ID

Start provisioning:

```bash
curl -X POST "<api-url>/admin/provision" \
  -H "Authorization: Bearer <api-key>" \
  -H "content-type: application/json" \
  -d '{"apple_id":"<apple-id-email>","password":"<apple-id-password>"}'
```

Complete two-factor authentication:

```bash
curl -X POST "<api-url>/admin/provision/complete" \
  -H "Authorization: Bearer <api-key>" \
  -H "content-type: application/json" \
  -d '{"two_factor_code":"<six-digit-code>"}'
```

Verify the line:

```bash
curl "<api-url>/handles" \
  -H "Authorization: Bearer <api-key>"
```

Send the first test message:

```bash
curl -X POST "<api-url>/messages" \
  -H "Authorization: Bearer <api-key>" \
  -H "content-type: application/json" \
  -d '{"to":"<recipient-email-or-phone>","text":"hi"}'
```

Start a FaceTime test call:

```bash
curl -X POST "<api-url>/facetime/calls" \
  -H "Authorization: Bearer <api-key>" \
  -H "content-type: application/json" \
  -d '{"to":"<recipient-email-or-phone>"}'
```

## Files And State Created

The daemon stores Apple session state under `DATA_DIR`, usually `/app/data`.

Expected state files include:

```text
hwconfig.plist
config.plist
id_cache.plist
facetime.plist
```

Keep this directory private. Do not share it between unrelated Apple IDs unless the daemon has been refactored for multi-line isolation.
