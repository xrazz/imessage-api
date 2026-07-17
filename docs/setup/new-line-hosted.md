# New Line: Hosted Daemon/API Setup

Use one isolated daemon state per iMessage line.

## Rule

One line should have:

- one daemon state directory
- one Apple ID provisioned into that daemon state
- one `config.plist`, `id_cache.plist`, `facetime.plist`, and keystore set

Do not reuse the same daemon data directory for multiple Apple IDs unless the daemon has been refactored to support multiple isolated lines.

## Hosted Services

Deploy two services for the current single-line setup:

1. `api`
   - public HTTP service
   - forwards requests to the daemon
2. `daemon`
   - owns the Apple session and persisted IDS/APS state

For many lines, see [multi-line-backend.md](multi-line-backend.md).

For the shortest copy-paste flow, see [three-step-provision.md](three-step-provision.md).

## Environment

Set these on the public API service:

```text
API_KEY=<random-secret>
DAEMON_URL=http://<daemon-private-host>:8080
```

Set these on the daemon service:

```text
DATA_DIR=/app/data
HWCONFIG_PLIST_BASE64=<base64-from-new-line-mac.md>
```

Attach a persistent volume to the daemon at:

```text
/app/data
```

If you do not use `HWCONFIG_PLIST_BASE64`, place `hwconfig.plist` directly at:

```text
/app/data/hwconfig.plist
```

## Provision Apple ID

Start provisioning:

```bash
curl -X POST "<api-url>/admin/provision" \
  -H "Authorization: Bearer <api-key>" \
  -H "content-type: application/json" \
  -d '{
    "apple_id": "<apple-id-email>",
    "password": "<apple-id-password>"
  }'
```

Complete provisioning with the Apple two-factor code:

```bash
curl -X POST "<api-url>/admin/provision/complete" \
  -H "Authorization: Bearer <api-key>" \
  -H "content-type: application/json" \
  -d '{
    "two_factor_code": "<six-digit-code>"
  }'
```

The daemon stores session state in its data directory. The Apple ID password is only used for the provisioning request.

## Verify

Check daemon readiness:

```bash
curl "<api-url>/health" \
  -H "Authorization: Bearer <api-key>"
```

List available sender handles:

```bash
curl "<api-url>/handles" \
  -H "Authorization: Bearer <api-key>"
```

Send a test iMessage:

```bash
curl -X POST "<api-url>/messages" \
  -H "Authorization: Bearer <api-key>" \
  -H "content-type: application/json" \
  -d '{
    "to": "<recipient-email-or-phone>",
    "text": "hi"
  }'
```

Start a FaceTime call and get the browser join link:

```bash
curl -X POST "<api-url>/facetime/calls" \
  -H "Authorization: Bearer <api-key>" \
  -H "content-type: application/json" \
  -d '{
    "to": "<recipient-email-or-phone>"
  }'
```

Expected FaceTime response includes:

```json
{
  "ok": true,
  "call_id": "...",
  "join_link": "https://facetime.apple.com/join#..."
}
```
