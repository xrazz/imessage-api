# New iMessage Line Checklist

Copy this for each new line.

## Mac

- [ ] Clone this repository on a Mac.
- [ ] Confirm the Apple ID can complete 2FA.
- [ ] Confirm iMessage/FaceTime works for the Apple ID or number.
- [ ] Run:

```bash
mkdir -p private
xcrun swift scripts/extract_hwconfig.swift > private/hwconfig.plist
plutil -p private/hwconfig.plist
base64 < private/hwconfig.plist | tr -d '\n' | pbcopy
```

- [ ] Save the copied base64 as `HWCONFIG_PLIST_BASE64` for the daemon.

## Hosting

- [ ] Create or select a daemon state for this line.
- [ ] If using the current single-line daemon, attach a fresh persistent volume at `/app/data`.
- [ ] Set daemon env:

```text
DATA_DIR=/app/data
HWCONFIG_PLIST_BASE64=<base64>
```

- [ ] Create or reuse an API service that points to this daemon.
- [ ] Set API env:

```text
API_KEY=<random-secret>
DAEMON_URL=http://<daemon-private-host>:8080
```

## Provision

- [ ] Start provisioning:

```bash
curl -X POST "<api-url>/admin/provision" \
  -H "Authorization: Bearer <api-key>" \
  -H "content-type: application/json" \
  -d '{"apple_id":"<apple-id-email>","password":"<apple-id-password>"}'
```

- [ ] Complete 2FA:

```bash
curl -X POST "<api-url>/admin/provision/complete" \
  -H "Authorization: Bearer <api-key>" \
  -H "content-type: application/json" \
  -d '{"two_factor_code":"<six-digit-code>"}'
```

## Test

- [ ] Handles:

```bash
curl "<api-url>/handles" \
  -H "Authorization: Bearer <api-key>"
```

- [ ] iMessage:

```bash
curl -X POST "<api-url>/messages" \
  -H "Authorization: Bearer <api-key>" \
  -H "content-type: application/json" \
  -d '{"to":"<recipient>","text":"hi"}'
```

- [ ] FaceTime:

```bash
curl -X POST "<api-url>/facetime/calls" \
  -H "Authorization: Bearer <api-key>" \
  -H "content-type: application/json" \
  -d '{"to":"<recipient>"}'
```

## Notes

- Email Apple IDs usually produce `mailto:` handles.
- Phone handles require the phone number to already be registered/entitled for iMessage.
- Do not run two daemon instances against the same data directory.
- Avoid aggressively provisioning many Apple IDs from the same hardware identity.
