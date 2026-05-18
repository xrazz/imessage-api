# Architecture

## Runtime

```text
client -> api (Node.js) -> daemon (Rust) -> rustpush -> Apple APS/IDS
```

## Folder responsibilities

- `vendor/rustpush/`: upstream engine; keep close to upstream
- `daemon/`: our Rust service; owns session lifecycle and state
- `api/`: our public HTTP layer; owns auth, validation, and response shape

## V1 constraints

- Apple ID only
- one plain-text outbound message
- no inbound sync
- no attachments
- no groups
- no phone-number registration

