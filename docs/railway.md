# Railway v1 deployment

## Services

Create two Railway services from the same repository:

1. `api`
   - root directory: `api`
   - Dockerfile: `Dockerfile`
   - public HTTP service
2. `daemon`
   - root directory: repository root
   - Dockerfile: `daemon/Dockerfile`
   - private service only

## Variables

### api

- `API_KEY`
- `DAEMON_URL=http://daemon.railway.internal:8080`

### daemon

- `DATA_DIR=/app/data`

## Volume

Attach a volume to `daemon` and mount it at:

```text
/app/data
```

That is where the daemon will persist Apple/APS/IDS state.

The daemon image also builds the OpenBubbles validation helper from `vendor/openbubbles-build-modules`
and includes the released OpenBubbles shared library it needs to generate signed Mac validation data on Railway.

## First-time provisioning

After `hwconfig.plist` has been placed in the daemon volume, call the public API's guarded admin endpoint:

```http
POST /admin/provision
Authorization: Bearer $API_KEY
{
  "apple_id": "...",
  "password": "..."
}
```

That sends a verification code to trusted devices. Then complete provisioning:

```http
POST /admin/provision/complete
Authorization: Bearer $API_KEY
{
  "two_factor_code": "..."
}
```

That creates and persists the Apple/APS/IDS runtime state needed for later restarts.
The raw Apple ID password is used only for the provisioning request and is not written to disk by the daemon.

## Networking

Railway private networking lets services in the same project talk over internal DNS names
like `daemon.railway.internal`, so only `api` needs public exposure.
