# imessage-api

Small hosted iMessage send API built around OpenBubbles' `rustpush` engine.

## Layout

```text
imessage-api/
  vendor/rustpush/   upstream protocol engine
  daemon/            our Rust wrapper that maintains the Apple session
  api/               our public Node.js HTTP API
  docs/              architecture, setup, and Q&A notes
  infra/             deployment files
```

## V1 scope

- Apple ID only
- plain-text outbound iMessage
- iMessage availability checks
- FaceTime call links and inbound FaceTime event webhooks
- Railway-hosted runtime
- Mac used only once for provisioning hardware identity

## Key Docs

- [Setup docs](docs/setup/README.md)
- [iMessage and FaceTime Q&A](docs/qa/imessage-facetime-qa.md)
- [FaceTime audio browser bridge](docs/facetime-audio-bridge.md)
