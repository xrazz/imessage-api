# Apple-ID send path

This is the smallest `rustpush` flow needed for v1:

1. Load `MacOSConfig`
2. Open `APSConnectionResource`
3. Sign into Apple ID
4. Request IDS delegates
5. Call `authenticate_apple(...)`
6. Register `MADRID_SERVICE` if needed
7. Construct `IMClient`
8. Pick the sender handle from `client.identity.get_handles()`
9. Build:

```rust
let mut msg = NormalMessage::new(text, MessageType::IMessage);
let mut inst = MessageInst::new(
    ConversationData {
        participants: vec![recipient],
        cv_name: None,
        sender_guid: Some(Uuid::new_v4().to_string()),
        after_guid: None,
    },
    &handle,
    Message::Message(msg),
);
client.send(&mut inst).await?;
```

The daemon now lifts this path and persists the resulting state under `/app/data`.
