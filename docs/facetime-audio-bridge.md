# FaceTime Audio Browser Bridge

This is the practical path for a Sendblue-style flow:

```text
sales rep browser
  <-> WebRTC audio room
  <-> our bridge worker
  <-> Apple-side FaceTime Audio participant
  <-> lead's Apple device
```

The browser is not directly inside FaceTime. The browser joins our audio room, and a real Apple-side participant keeps the FaceTime call alive while audio is bridged in both directions.

## What Sendblue Appears To Offer

Sendblue's public FaceTime calling docs describe connecting to the audio stream through WebRTC using Agora's SDK:

```text
browser/client <-> Agora WebRTC audio <-> Sendblue call bridge <-> FaceTime Audio
```

Their public product wording focuses on FaceTime Audio, not full FaceTime video.

References:

- https://docs.sendblue.com/calling/facetime/
- https://www.sendblue.com/products/calling

## Why Audio-Only Is Easier Than Video

Audio-only still needs an Apple-side media participant, but it avoids the hardest parts of video:

- no browser FaceTime video decoding
- no camera track bridging
- no layout/composition
- lower bandwidth
- simpler device routing
- easier echo cancellation
- easier recording/transcription later

It is still not possible with rustpush alone. Rustpush can create the FaceTime session, create links, approve web joins, and manage signaling. It does not expose FaceTime audio packets as a browser stream.

## Required Components

### 1. Existing Rustpush Daemon

Use the current daemon for:

- checking FaceTime availability
- starting outbound FaceTime calls
- catching inbound FaceTime rings
- creating FaceTime join links
- auto-approving `LetMeInRequest`
- sending call events to the API/webhook

### 2. Apple-Side Media Host

This is the piece that actually joins FaceTime Audio.

Options:

- macOS FaceTime app controlled by automation
- macOS CallKit/FaceTime private API bridge
- iPhone/iPad automation bridge
- future reverse-engineered FaceTime media client

The easiest realistic version is a Mac mini worker:

```text
Mac mini logged into Apple ID
FaceTime Audio call active
virtual audio device for input/output
small local bridge app streams audio to/from WebRTC
```

### 3. Browser Audio Room

The sales rep opens a normal browser link and joins a WebRTC audio room.

Options:

- Agora
- Twilio Video/Voice SDK
- LiveKit
- Daily
- custom WebRTC SFU

For fastest build, use Agora or LiveKit.

### 4. Audio Bridge Worker

The bridge worker connects:

```text
FaceTime output audio -> WebRTC microphone track
WebRTC speaker audio -> FaceTime input audio
```

On macOS this usually means:

- a virtual audio driver/device
- system audio capture
- microphone injection
- WebRTC client process
- call state monitor

Possible macOS audio tools:

- BlackHole
- Loopback
- Audio Hijack
- CoreAudio custom driver

## Recommended MVP

Build the MVP as audio-only with a Mac worker.

### Flow

1. API receives `POST /facetime-audio/calls`.
2. API asks daemon to start a FaceTime Audio call to the lead.
3. API creates a WebRTC/Agora/LiveKit room for the sales rep.
4. Mac worker joins/answers the FaceTime Audio call as the Apple ID.
5. Mac worker joins the WebRTC room as a hidden participant.
6. Mac worker bridges audio both ways.
7. API returns the sales rep browser URL.

```text
POST /facetime-audio/calls
{
  "to": "+13025551234",
  "rep_id": "rep_123"
}
```

Response:

```json
{
  "ok": true,
  "call_id": "facetime-call-id",
  "rep_join_url": "https://app.example.com/calls/abc123",
  "status": "ringing"
}
```

## Why The Current Link-Handoff Fails

The current system does this:

```text
daemon creates FaceTime session
lead joins/answers
sales rep joins FaceTime web link as guest
daemon is not a real media participant
Apple ends the session when the Apple-side participant leaves
```

That is why the call can say the Apple ID left even though the user never manually joined. Apple is tracking the Apple account/daemon participant, not the browser guest as the real call owner.

## Can We Fake A Blank Participant With Rustpush?

Maybe at the signaling layer, but not as a true audio participant.

Rustpush has conversation propping logic that can tell FaceTime the Apple ID is active enough for some transitions. This can help with guest approval and one-on-one-to-group conversion, but it does not receive or transmit real audio.

So the safe expectation is:

```text
rustpush call keeper: may reduce instant drops
rustpush audio bridge: not enough
Mac/media bridge: required for reliable audio
```

## Implementation Phases

### Phase 1: Call Keeper Experiment

Goal: see if we can keep FaceTime web-link calls alive longer without media.

Tasks:

- add a daemon endpoint to mark a FaceTime call as kept alive
- periodically re-prop active sessions
- when guests join, keep the Apple participant active
- when our own handle leaves, attempt to re-prop once
- log all join/leave/responded-elsewhere events

Expected result:

- may stop some instant call drops
- will not provide audio bridging
- may still fail because Apple expects real media presence

### Phase 2: Browser Audio Room

Goal: sales rep joins a browser audio room.

Tasks:

- create `POST /audio-rooms`
- generate short-lived room tokens
- build simple browser page with mute/unmute/hangup
- emit room join/leave events

### Phase 3: Mac Audio Bridge

Goal: bridge audio between FaceTime Audio and browser room.

Tasks:

- run a bridge agent on a Mac
- keep the Mac logged into the Apple ID
- place or answer FaceTime Audio calls
- route FaceTime output into WebRTC
- route WebRTC output into FaceTime microphone
- monitor call state and recover on disconnect

### Phase 4: Unified API

Goal: make it feel like one API.

Endpoints:

```text
POST /facetime-audio/calls
GET  /facetime-audio/calls/:id
POST /facetime-audio/calls/:id/hangup
GET  /facetime-audio/calls/:id/events
```

Webhook events:

```text
facetime_audio.ringing
facetime_audio.answered
facetime_audio.rep_joined
facetime_audio.lead_joined
facetime_audio.bridge_connected
facetime_audio.bridge_failed
facetime_audio.ended
```

## Reliability Notes

The Mac/media bridge should be treated as stateful infrastructure:

- keep Mac awake
- disable automatic sleep
- use wired internet if possible
- run one Apple line per isolated Mac user account or worker
- restart bridge worker between calls if audio devices get stuck
- expose health checks for FaceTime app state and WebRTC connection state

## Bottom Line

Audio-only can work and is much easier than video, but it still needs a real Apple-side media participant.

The fastest production-ish plan is:

```text
rustpush daemon for call setup/events
Mac worker for FaceTime Audio media
Agora/LiveKit for browser rep audio
API returns rep browser URL
```

The fastest experiment is:

```text
add rustpush call keeper
test if two FaceTime web guests can stay connected longer
then add Mac audio bridge if keeper is not enough
```
