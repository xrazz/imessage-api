# imessage-api

Small hosted iMessage send API built around OpenBubbles' `rustpush` engine.

## Layout

```text
imessage-api/
  vendor/rustpush/   upstream protocol engine
  daemon/            our Rust wrapper that maintains the Apple session
  api/               our public Node.js HTTP API
  docs/              setup and provisioning notes
  infra/             deployment files
```

## V1 scope

- Apple ID only
- plain-text outbound iMessage
- Railway-hosted runtime
- Mac used only once for provisioning hardware identity

