# Transport & Encoding

## 1. Transport

All protocol traffic runs over **binary WebSocket** (RFC 6455) messages on top of TCP.

* Default port: **8423** on the server (controlled machine).
* A text message where a protocol frame is expected is an error. Text
  messages are used only for the JSON control messages of the rendezvous
  relay (see [Internet Remote Transport](../remote/internet-transport.md)).
* The transport is **reliable and ordered**, which the encryption layer depends on (see [Encryption](encryption.md)).

Implementation: `opendesk-rs/src/remote/transport.rs`.

---

## 2. Encoding

Every frame is **msgpack-encoded** with `rmp-serde` using named fields
(see `opendesk-rs/src/protocol/frames.rs`).

Key conventions:
* Byte payloads such as screenshot data travel as native msgpack `bin` —
  **never base64**.
* Method params and results are dynamic msgpack values (`rmpv::Value`).
* Missing optional fields take defaults (`v` defaults to the protocol version,
  RES `end` defaults to `true`).

---

Next: [Encryption & Identity →](encryption.md)
