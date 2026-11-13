import assert from "node:assert/strict";
import { test } from "node:test";
import { TOKEN_COOKIE, parseHandshake, requestFor, scrub, wantsOrigin } from "./record-golden.mjs";

const token = "a".repeat(64);
const origin = "http://127.0.0.1:52731";

test("the handshake line parses when it names a loopback origin and a wide token", () => {
  assert.deepEqual(parseHandshake(JSON.stringify({ version: 1, origin, token })), { origin, token });
});

test("the handshake refuses a foreign shape", () => {
  assert.throws(() => parseHandshake("not json"), /not JSON/);
  assert.throws(
    () => parseHandshake(JSON.stringify({ version: 2, origin, token })),
    /unsupported handshake version/,
  );
  assert.throws(
    () => parseHandshake(JSON.stringify({ version: 1, origin, token: "short" })),
    /no usable credential/,
  );
  assert.throws(
    () => parseHandshake(JSON.stringify({ version: 1, origin: "https://127.0.0.1:52731", token })),
    /does not load/,
  );
  assert.throws(
    () => parseHandshake(JSON.stringify({ version: 1, origin: "http://10.0.0.1:8080", token })),
    /not this machine/,
  );
  assert.throws(() => parseHandshake(JSON.stringify({ version: 1, origin: "http://127.0.0.1", token })), /no port/);
});

test("scrub replaces the origin before the credential it contains", () => {
  const text = `cookie ${TOKEN_COOKIE}=${token}; site ${origin}/x`;
  assert.equal(scrub(text, { token, origin }), `cookie ${TOKEN_COOKIE}=__CREDENTIAL__; site http://127.0.0.1:__PORT__/x`);
});

test("a read names no origin, a write does unless the case says otherwise", () => {
  assert.equal(wantsOrigin({ method: "GET", path: "/runtimes" }), false);
  assert.equal(wantsOrigin({ method: "HEAD", path: "/events" }), false);
  assert.equal(wantsOrigin({ method: "POST", path: "/runtimes" }), true);
  assert.equal(wantsOrigin({ method: "POST", path: "/runtimes", origin: false }), false);
  assert.equal(wantsOrigin({ method: "GET", path: "/runtimes", origin: true }), true);
});

test("requestFor carries the cookie and only bends the Host header when told", () => {
  const req = requestFor({ method: "GET", path: "/runtimes" }, origin, token);
  assert.equal(req.headers.cookie, `${TOKEN_COOKIE}=${token}`);
  assert.equal(req.headers.host, undefined);
  assert.equal(req.method, "GET");
  assert.equal(req.path, "/runtimes");

  const bent = requestFor({ method: "GET", path: "/runtimes", host: "example.com:80" }, origin, token);
  assert.equal(bent.headers.host, "example.com:80");
});

test("requestFor sizes the body it sends", () => {
  const req = requestFor({ method: "POST", path: "/runtimes", body: { a: 1 } }, origin, token);
  assert.equal(req.headers["content-type"], "application/json");
  assert.equal(req.headers["content-length"], Buffer.byteLength(JSON.stringify({ a: 1 })));
  assert.equal(req.body, JSON.stringify({ a: 1 }));
});
