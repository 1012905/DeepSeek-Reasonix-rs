# reasonix-studio-web

Serves the zero-install Reasonix Studio web page from `desktop-tauri/dist` and
runs the pairing service and relay that let that page drive a paired desktop.
Everything is one Cloudflare Worker plus one `RelayRoom` Durable Object per
device.

The relay forwards opaque ciphertext envelopes. It never decrypts, stores, or
buffers messages, so the desktop/browser crypto stays the only party that knows
the payload.

## Protocol

| Method | Path | Auth | Request / response |
| --- | --- | --- | --- |
| `POST` | `/v1/devices` | none | `{publicKey, capabilities}` → `201 {deviceId, deviceCredential}` |
| `POST` | `/v1/devices/{deviceId}/pair` | `Bearer <deviceCredential>` | `{code, expiresIn}`; code is 6 digits, single-use, 300 s TTL |
| `GET` | `/v1/pair/{code}` | none | `{deviceId, publicKey, relay, controlTicket}`; consumes the code; ticket lives 600 s |
| `POST` | `/v1/devices/{deviceId}/rotate` | `Bearer <deviceCredential>` | `{deviceId, deviceCredential}`; the old credential stops working |
| `DELETE` | `/v1/devices/{deviceId}` | `Bearer <deviceCredential>` | `204`; closes both sockets with `4403` |
| `GET` (WebSocket) | `/v1/devices/{deviceId}/connect` | `Bearer <deviceCredential>` | desktop socket; speaks the Go host's gateway envelope |
| `GET` (WebSocket) | `/v1/sessions/connect` | subprotocol `reasonix.auth.<controlTicket>` (or `?ticket=`) | browser socket |

`relay` is the request origin with the `wss:` scheme. Messages in both
directions are capped at 64 KiB (65536 bytes).

### Desktop gateway envelope

The desktop path keeps the envelope the Go host (`internal/platform/remotecloud`)
already speaks, one JSON object per text frame:

- relay → desktop on open: `{type:"relay_hello", controllers, heartbeat, heartbeatMs}`.
  The host echoes `heartbeat` back as a raw text frame and pings on its own tick.
- relay → desktop: `{type:"controller_connected"|"controller_message"|"controller_disconnected", connectionId, scopes, payload}`.
  `scopes` is `["desktop"]`; `payload` is the sealed controller frame.
- desktop → relay: `{"to": connectionId, "payload": sealed}` and
  `{type:"disconnect_controller", connectionId}`.

The browser socket carries sealed payloads only: the first frame it receives is
the desktop's sealed `ready`, and everything after is one sealed request or
response per frame.

### WebSocket close codes

| Code | Meaning | Sent by |
| --- | --- | --- |
| `1003` | binary frame on a text protocol | relay |
| `1009` | message exceeds 64 KiB | relay |
| `4004` | desktop disconnected, or the desktop closed this session | relay → controllers |
| `4401` | desktop replaced by a newer connection | relay → old desktop |
| `4403` | device revoked | relay → both roles, on `POST /revoke` |
| `4408` | idle | reserved; not emitted yet |

The room holds at most one desktop socket and any number of controller sockets.
Controller traffic is forwarded to the desktop and desktop traffic to the named
controller; a message with no peer is dropped.

## KV layout

| Key | Value | TTL |
| --- | --- | --- |
| `device:<deviceId>` | JSON `{publicKey, capabilities, credentialHash, createdAt}` | none |
| `pair:<code>` | `deviceId` | 300 s |
| `ticket:<sha256(controlTicket)>` | `deviceId` | 600 s |

Credentials and tickets are stored only as SHA-256 hashes, and a ticket is
resolved by hashing what the socket presents, so the KV contents alone cannot
authenticate a device or a browser.

## Deploy

Prerequisites:

- Build the page: `pnpm --dir desktop-tauri build` from the repository root.
  `desktop-tauri/dist` is gitignored, and the Worker uploads it as static
  assets, so a checkout without that build cannot deploy.
- Create the namespace once: `npx wrangler kv namespace create PAIRING`, then
  paste the printed id into `wrangler.jsonc` (the committed value is a
  placeholder).
- The `v1` migration in `wrangler.jsonc` creates `RelayRoom` on first deploy.

```bash
cd workers/remote
npx wrangler dev        # local Worker + Durable Objects
npx wrangler deploy     # production
```

Secrets:

- The Worker itself has none. `PAIRING` and `RELAY` are bindings.
- CI reads `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID` from repository
  secrets; `.github/workflows/deploy-remote.yml` passes them as environment
  variables to `wrangler deploy`.

## Deliberately unimplemented

- **JWT signing and verification.** `controlTicket` is JWT-shaped but unsigned;
  authority comes from the KV record, not from the token's claims.
- **Rate limits.** Registration, pairing, redemption, and socket upgrades are
  all unthrottled.
- **End-to-end tests.** No test runner is wired up; `tsc --noEmit` is the gate.
- **Code redemption atomicity.** `redeemPairCode` reads and deletes in two KV
  operations; KV is eventually consistent, so single-use is best effort.
