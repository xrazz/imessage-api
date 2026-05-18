# Provisioning

The Mac is used once only.

## One-time Mac job

1. Obtain Mac validation data.
2. Extract the hardware config used by `rustpush`.
3. Store the resulting hardware config securely for Railway runtime use.

`rustpush` already demonstrates this in `src/test.rs`: it can derive `HardwareConfig`
from Mac validation data, persist `hwconfig.plist`, and reuse those hardware identifiers
for later registrations without requiring the Mac again.

## Railway state

The runtime service must persist:

- hardware config
- APS push state
- IDS identity state
- registered users / cert material
- key cache

Use a Railway volume mounted at `/app/data` for these files.

