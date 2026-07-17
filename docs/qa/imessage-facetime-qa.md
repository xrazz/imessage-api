# iMessage and FaceTime Q&A

This captures the practical behavior seen while testing the hosted daemon/API.

## Why can FaceTime work when iMessage to a phone number fails?

iMessage and FaceTime use different Apple IDS service topics.

- iMessage uses the Madrid service.
- FaceTime uses FaceTime/video service topics.

A phone number can validate for FaceTime even when iMessage sending to that same `tel:` handle fails. Treat those as separate checks.

## Why did email iMessage work but phone iMessage fail?

The current hosted line only has this sender handle:

```text
mailto:rajtripathi2580@gmail.com
```

It has no registered `tel:` sender handle. Email-to-email iMessage can work normally, while email-to-phone can fail because of Apple IDS lookup behavior, missing phone sender entitlement, stale cache, throttling, or target-side registration quirks.

## Can a phone-line iMessage account check or message an email-line iMessage account?

Yes. Apple IDS handles include both:

```text
tel:+13025551234
mailto:user@example.com
```

A phone-number sender can check or message an email iMessage handle if both handles are valid for iMessage.

## Can an email-line iMessage account message a phone-number iMessage account?

Sometimes, but it is less reliable in this project today.

If the daemon only has an email sender handle, Apple may reject or fail delivery to some `tel:` recipients even when those recipients do have iMessage. Warming up the conversation from a real linked Apple device can sometimes help, but it is not a guarantee.

## What are the 6005/send errors?

In our API, failed iMessage sends usually surface as `send_failed` with a message like:

```text
Could not deliver message. The recipient does not have iMessage or you are being rate-limited.
```

This does not always mean the target truly lacks iMessage. It can also mean Apple rejected this sender/target pair, the IDS cache is stale, the sender handle is not eligible, or the account/hardware is being throttled.

## Why does OpenBubbles work when this fails?

OpenBubbles may have a stronger local Mac bridge path, already-warmed Apple state, phone-number registration support, and more production hardening around IDS cache and handle behavior.

This daemon has the core pieces, but phone-number iMessage delivery is still more fragile than email-handle messaging and FaceTime signaling.

## Does availability always work if sending fails?

No. Availability checks and sends are separate flows.

Availability can return true while send fails, and availability can return false while a real Apple device still shows the recipient as blue. Apple caches and throttles IDS lookups, and phone-number handles are especially inconsistent.

## What happens when we receive a FaceTime call?

The daemon listens for FaceTime events and stores the latest events in memory. When an inbound FaceTime ring arrives, it can extract the FaceTime web join link and optionally POST the event to `FACETIME_WEBHOOK_URL`.

The event usually includes:

```json
{
  "type": "facetime.ring",
  "call_id": "...",
  "handle": "tel:+...",
  "join_link": "https://facetime.apple.com/join#..."
}
```

## What happens if someone joins from the link instead of the Mac?

Apple treats the browser user as a FaceTime web guest joining the session. The call should not cut just because the join is from a browser.

Possible outcomes:

- If the caller stays in the session, the browser guest should remain connected like a normal FaceTime web participant.
- If everyone leaves, the session ends.
- If the caller hangs up before anyone joins, the link can die.
- If Apple marks the call idle or unanswered for too long, it can time out.
- If the daemon restarts, it may stop seeing approval/event updates for that active call.

## Can the browser join without manual approval?

Sometimes yes. The daemon now attempts to auto-approve FaceTime web `LetMeInRequest` events. If Apple sends that request to the daemon and the current call is tracked, it can approve without a person tapping a button.

This does not remove Apple's guest-approval protocol. It just automates the approval when the daemon receives the request.

## How long does a FaceTime web link last?

Once a browser guest is inside the call, it should last like a normal FaceTime web call while participants remain connected.

The risky window is before joining: ringing, idle timeout, caller hangup, or missed approval can kill the session.

## Can Sendblue-style browser video be done without a Mac bridge?

For real FaceTime media, Apple still controls the call session. A browser can join through Apple's FaceTime web link, but fully proxying native FaceTime audio/video into a custom browser UI is much harder than just returning the join link.

Sendblue's public FaceTime material appears focused on audio-oriented calling/API flows rather than full custom browser-controlled FaceTime video.

## Can one backend have many iMessage lines?

Yes.

Current safest shape:

```text
one API
many isolated daemon states
one Apple account per daemon state
```

A future refactor can make one daemon process host multiple lines, but each line still needs isolated auth, FaceTime state, iMessage client, APS watcher, event buffer, and webhook config.
