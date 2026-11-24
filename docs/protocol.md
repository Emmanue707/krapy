# krapy wire protocol v1

The desktop listens. The phone connects. One TCP connection carries control,
one UDP flow carries video and pings. Both use the same port number
(default 27183).

All integers are big endian.

## Pairing QR

```
krapy://pair?v=1&h=<desktop ip>&p=<port>&k=<key>&id=<desktop id>&n=<desktop name>
```

- `k`: 32 random bytes, base64url without padding. Single use.
- `id`: 16 hex chars, first 8 bytes of SHA-256 of the desktop identity secret.
- `n`: url-encoded desktop name.

Both sides derive the long-term pair secret:

```
S = HKDF-SHA256(ikm = k, salt = "krapy-v1", info = "pair", len = 32)
```

The desktop stores `S` against the phone id after the first good handshake and
forgets `k`. The phone stores `S`, desktop id, name and last host/port.

## Discovery

The desktop advertises `_krapy._tcp.local.` with TXT `id=<desktop id>` and
`v=1`. When a stored host stops answering, the phone browses for the stored
id and uses whatever address it resolves to.

## Handshake (TCP, plaintext)

Every TCP frame is `u16 len` then `len` bytes.

Phone -> desktop `Hello`:

```
"KRPY" | ver u8 = 1 | phone_id [8] | nonce_p [16] | name_len u8 | name utf8
```

Desktop -> phone `Welcome`:

```
"KRPY" | ver u8 = 1 | desktop_id [8] | nonce_d [16]
```

If the desktop does not know the phone (and is not in pair mode) it closes
the connection.

Session keys:

```
okm = HKDF-SHA256(ikm = S, salt = nonce_p || nonce_d, info = "krapy session", len = 128)
tcp_p2d = okm[0..32]   tcp_d2p = okm[32..64]
udp_p2d = okm[64..96]  udp_d2p = okm[96..128]
```

## Sealed frames

ChaCha20-Poly1305. Nonce is `00 00 00 00 | counter u64`. Each key has its own
counter starting at 0 and never reused.

TCP frame body after the handshake: `counter u64 | ciphertext+tag`, AAD is the
8 counter bytes. The first sealed frame from the phone must be `Start`;
decrypting it proves both sides hold `S`.

UDP datagram: `type u8 | counter u64 | ciphertext+tag`, AAD is the first 9
bytes. Receivers drop counters already seen or more than 1024 behind the
highest seen.

## Control messages (inside sealed TCP frames)

`type u8` then body.

| type | dir | name | body |
|------|-----|------|------|
| 0x01 | p->d | Start | width u16, height u16, fps u8, bitrate u32, codec u8 (1 = h264) |
| 0x02 | d->p | Quality | bitrate u32, fps u8, max_size u16 |
| 0x03 | d->p | Keyframe | - |
| 0x04 | p->d | Format | width u16, height u16 (sent when the stream size changes) |
| 0x10 | both | Clipboard | utf8 |
| 0x20 | d->p | Touch | action u8 (0 down, 1 move, 2 up), pointer u8, x u16, y u16 (video pixels) |
| 0x21 | d->p | Key | code u8 (1 back, 2 home, 3 recents, 4 power) |
| 0x22 | d->p | Text | utf8 |
| 0x30 | p->d | VideoChunk | same body as the UDP Video datagram, data up to 32 KiB |
| 0x31 | p->d | AudioChunk | seq u32, pts_us u64, PCM s16le stereo 48 kHz (max 960 bytes) |
| 0x40 | p->d | PhoneStats | enc_avg_us u32, enc_max_us u32, fps u8, bitrate u32 |
| 0x41 | d->p | DesktopStats | fps u8, latency_ms u16, decode_ms u16, loss_permille u16 |
| 0x42 | p->d | PresetRequest | preset u8 (0 smooth, 1 balanced, 2 sharp) |
| 0x43 | p->d | Paused | on u8 |
| 0x45 | p->d | RotateView | - |
| 0x46 | p->d | AudioState | on u8, rate u32 le, channels u8, phone_muted u8 (legacy 2-byte: rate 48000, channels 2, phone_muted 0) |
| 0x7f | both | Bye | - |

## UDP datagrams (plaintext after opening)

| type | dir | name | body |
|------|-----|------|------|
| 0x01 | p->d | Video | frame_id u32, idx u16, count u16, flags u8, capture_us u64, data |
| 0x02 | d->p | Ping | id u32, t_us u64 |
| 0x03 | p->d | Pong | id u32, t_us u64 (echoed), phone_us u64 optional (phone monotonic us) |
| 0x05 | p->d | Keepalive | - |
| 0x06 | p->d | Audio | seq u32, pts_us u64, PCM payload (max 960 bytes) |

Video flags: bit 0 keyframe, bit 1 codec config (SPS/PPS). `data` is at most
1100 bytes so a sealed datagram stays under 1200 bytes. A frame is the
fragments `0..count` of one `frame_id`, concatenated, as Annex B H.264.

The phone sends `Keepalive` right after `Start` and then every second so the
desktop learns its UDP address and hotspot NAT stays open. The desktop sends
`Ping` every 250 ms.

When the phone connects to a loopback address (USB through `adb reverse`,
which only tunnels TCP) it sends video as `VideoChunk` on the TCP connection
instead of UDP, and skips `Keepalive`.

Every keyframe carries SPS/PPS in front of the IDR. The desktop also keeps
the last SPS/PPS and adds them to an IDR that arrives without them.

## Late frames

The desktop never waits for a frame older than the newest complete one.
Incomplete older frames are dropped. Complete frames are decoded in order;
if three or more pile up the desktop drops them and waits for a keyframe
instead of falling behind. Codec config is never dropped. After a loss or a
decode error the decoder waits for the next keyframe and the desktop sends
`Keyframe` (at most every 300 ms). Periodic IDRs use a longer interval; the desktop requests keyframes after loss or decode errors.

## Liveness

Nothing from the phone for 3 s, or the TCP connection closing, ends the
session. The phone reconnects with backoff 250 ms doubling to 4 s, trying
the last host first and then mDNS.

## Test vectors

```
k        = 000102...1f (32 bytes, 0x00..0x1f)
S        = 652e39ae96eb24d8546fe67d80acb68c9710e974f062d7c4486e0be6657fb38b
nonce_p  = 01 x16, nonce_d = 02 x16
tcp_p2d  = c14af774e5c109ed4ef46e6b15c8718ac80b0d50eeb8f754def92a441e6fb3b4
udp_d2p  = f8ce0d3e603791fe2648a067c98a537969b190eb20a9379c20dd3e4d5f06ac7b
seal_tcp(tcp_p2d, counter 0, "krapy")
         = 0000000000000000d483e26a1b70807bc388d46700f5be3dac1fe382e4
```
