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
| `GET` (WebSocket) | `/v1/devices/{deviceId}/connect` | `Bearer <deviceCredential>` | desktop socket |
| `GET` (WebSocket) | `/v1/devices/{deviceId}/control` | `Bearer <controlTicket>` | browser socket |

`relay` is the request origin with the `wss:` scheme. Messages in both
directions are capped at 64 KiB (65536 bytes).

### WebSocket close codes

| Code | Meaning | Sent by |
| --- | --- | --- |
| `1009` | message exceeds 64 KiB | relay |
| `4004` | desktop disconnected | relay → controllers |
| `4401` | desktop replaced by a newer connection | relay → old desktop |
| `4403` | device revoked | relay → both roles, on `POST /revoke` |
| `4408` | idle | reserved; not emitted yet |

The room holds at most one desktop socket and any number of controller sockets.
Controller traffic is forwarded to the desktop and desktop traffic to every
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
- **Revocation UX.** `RelayRoom` answers `POST /revoke` with `4403`, but there
  is no authenticated route to call it and no device or session management.
- **End-to-end tests.** No test runner or test script is wired up.
- **Code redemption atomicity.** `redeemPairCode` reads and deletes in two KV
  operations; KV is eventually consistent, so single-use is best effort.
- **Browser ticket carrier.** The control route reads `Authorization: Bearer`.
  The browser WebSocket API cannot set request headers, so the web client port
  still has to settle how it carries the ticket (query parameter or
  subprotocol) before this route can serve it.
