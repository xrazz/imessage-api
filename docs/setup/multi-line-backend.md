# Multi-Line Backend Setup

This project can support multiple iMessage/FaceTime lines, but the current daemon runtime is single-line unless refactored.

## Current Safe Shape

The reliable production shape is:

```text
one repo
one API service
many daemon services
many daemon volumes
```

Each daemon service runs the same code, but owns one isolated Apple account and one isolated data directory.

## One Daemon With Multiple Lines

A single daemon process can be refactored to manage many lines. The data model should look like:

```text
/app/data/lines/raj/auth.plist
/app/data/lines/raj/facetime.plist
/app/data/lines/raj/id_cache.plist

/app/data/lines/client1/auth.plist
/app/data/lines/client1/facetime.plist
/app/data/lines/client1/id_cache.plist
```

The daemon runtime then becomes:

```text
line_id -> IMClient + FTClient + APS watcher + event buffer + webhook config
```

The API surface should become:

```text
POST /lines/:line_id/messages
POST /lines/:line_id/availability
POST /lines/:line_id/facetime/calls
GET  /lines/:line_id/facetime/events
POST /lines/:line_id/admin/provision
POST /lines/:line_id/admin/provision/complete
```

Keep the existing single-line routes as aliases to a default line while migrating.

## Capacity Guidance

Thirty lines in one daemon is technically possible, but high-risk.

Suggested ranges:

- 5-10 lines per daemon: safer starting point.
- 15-20 lines per daemon: stretch range.
- 30 lines per daemon: experimental; expect more APS disconnects, throttling, and restart blast radius.

Reasons:

- Apple IDS behavior and hardware identity reputation matter more than CPU.
- OpenBubbles guidance has historically treated roughly 20 users per hardware identity as the upper comfort zone.
- One daemon crash or deploy restarts all active lines.
- Every line needs its own Apple auth state, IDS cache, APS watcher, and FaceTime state.
- Rate limits and throttling can affect individual accounts or the shared hardware identity.

## Guardrails For One-Daemon Multi-Line

Before putting many lines in one daemon, add:

- per-line runtime isolation
- per-line health checks
- per-line restart/reconnect logic
- APS reconnect with backoff
- persistent event queues
- per-line rate limits
- line-scoped webhooks
- startup auto-load for all provisioned lines
- admin endpoints to stop, start, reprovision, and inspect one line without restarting all lines
