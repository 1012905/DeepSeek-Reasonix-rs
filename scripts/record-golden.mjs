// Records golden fixtures against the Go kernel: build cmd/reasonix-studio-host,
// spawn it under an isolated state home, read the launch handshake from stdout,
// replay each case through the loopback gate, and write normalized pairs to
// crates/serve/fixtures/golden/manifest.json. The Rust serve crate replays the
// same cases; a byte that differs fails.

import { spawn, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import http from "node:http";

export const HANDSHAKE_VERSION = 1;
export const HANDSHAKE_TIMEOUT_MS = 30000;
export const HANDSHAKE_LIMIT = 4096;
export const TOKEN_COOKIE = "reasonix_token";

// parse refuses anything it cannot account for, the same shape Electron's
// host.js insists on: one JSON line naming the launch's version, origin and
// credential. An origin that is not a loopback IP literal with a port names a
// socket the gate could never have opened.
export function parseHandshake(line) {
  let body;
  try {
    body = JSON.parse(line);
  } catch {
    throw new Error("the handshake was not JSON");
  }
  if (body.version !== HANDSHAKE_VERSION) {
    throw new Error(`unsupported handshake version ${String(body.version)}`);
  }
  if (typeof body.token !== "string" || body.token.length < 32) {
    throw new Error("the handshake carried no usable credential");
  }
  let url;
  try {
    url = new URL(String(body.origin));
  } catch {
    throw new Error("the handshake carried no usable origin");
  }
  if (url.protocol !== "http:") {
    throw new Error(`the handshake named ${url.protocol}, which this recorder does not load`);
  }
  if (url.hostname !== "127.0.0.1") {
    throw new Error(`the handshake named ${url.hostname}, which is not this machine`);
  }
  if (!url.port) throw new Error("the handshake named no port");
  return { origin: url.origin, token: body.token };
}

// scrub replaces every launch-specific value so a fixture replays against any
// port and credential. The origin contains the port, so it goes first.
export function scrub(text, { token, origin }) {
  return String(text).split(origin).join("http://127.0.0.1:__PORT__").split(token).join("__CREDENTIAL__");
}

// wantsOrigin mirrors the gate: a read may arrive without an origin, anything
// that changes state must name the listener. A case can force either way.
export function wantsOrigin(caze) {
  if (caze.origin === true) return true;
  if (caze.origin === false) return false;
  return caze.method !== "GET" && caze.method !== "HEAD";
}

// requestFor builds the raw request a case names. The connection target stays
// 127.0.0.1; only the Host header can be bent, which is how a wrong-host case
// reaches the gate's first check.
export function requestFor(caze, origin, token) {
  const headers = {};
  if (caze.host) headers.host = caze.host;
  if (caze.credential !== false) headers.cookie = `${TOKEN_COOKIE}=${token}`;
  if (wantsOrigin(caze)) headers.origin = origin;
  let body = null;
  if (caze.body !== undefined) {
    body = JSON.stringify(caze.body);
    headers["content-type"] = "application/json";
    headers["content-length"] = Buffer.byteLength(body);
  }
  return { method: caze.method, path: caze.path, headers, body };
}

export function sendHttp(req, { port, timeoutMs = 10000 } = {}) {
  return new Promise((resolve, reject) => {
    const r = http.request(
      { host: "127.0.0.1", port, method: req.method, path: req.path, headers: req.headers },
      (res) => {
        const chunks = [];
        res.on("data", (c) => chunks.push(c));
        res.on("end", () =>
          resolve({
            status: res.statusCode,
            headers: res.headers,
            body: Buffer.concat(chunks).toString("utf8"),
          }),
        );
      },
    );
    r.setTimeout(timeoutMs, () => r.destroy(new Error("the response ran past its deadline")));
    r.on("error", reject);
    if (req.body) r.write(req.body);
    r.end();
  });
}

// collectStream reads an SSE response until the deadline closes in, keeping
// the first frames: the connect frame is part of the contract, later chatter
// is not.
export function collectStream(port, cazePath, headers, { deadlineMs = 2000, maxFrames = 40 } = {}) {
  return new Promise((resolve) => {
    const frames = [];
    let status = 0;
    let done = false;
    const finish = () => {
      if (done) return;
      done = true;
      try {
        r.destroy();
      } catch {
        // already torn down
      }
      resolve({ status, frames });
    };
    const r = http.request({ host: "127.0.0.1", port, path: cazePath, headers }, (res) => {
      status = res.statusCode;
      let buffered = "";
      res.setEncoding("utf8");
      res.on("data", (chunk) => {
        buffered += chunk;
        const lines = buffered.split("\n");
        buffered = lines.pop() ?? "";
        for (const line of lines) {
          frames.push(line);
          if (frames.length >= maxFrames) return finish();
        }
      });
      res.on("end", finish);
    });
    r.setTimeout(deadlineMs, finish);
    r.on("error", finish);
    r.end();
    setTimeout(finish, deadlineMs);
  });
}

// buildHost compiles the Go host first and runs the binary: killing `go run`
// on Windows leaves the child exe behind, killing the binary does not.
export function buildHost(root, outDir) {
  const out = path.join(outDir, process.platform === "win32" ? "reasonix-studio-host.exe" : "reasonix-studio-host");
  const res = spawnSync("go", ["build", "-o", out, "./cmd/reasonix-studio-host"], {
    cwd: root,
    stdio: ["ignore", "ignore", "inherit"],
  });
  if (res.status !== 0) throw new Error("the Go kernel failed to build");
  return out;
}

export function readHandshake(child, timeoutMs = HANDSHAKE_TIMEOUT_MS) {
  return new Promise((resolve, reject) => {
    let buffered = "";
    const timer = setTimeout(() => {
      reject(new Error(`the kernel was still running but sent no handshake within ${Math.round(timeoutMs / 1000)} s`));
    }, timeoutMs);
    const onData = (chunk) => {
      buffered += chunk;
      const end = buffered.indexOf("\n");
      if (end < 0) {
        if (buffered.length > HANDSHAKE_LIMIT) {
          clearTimeout(timer);
          reject(new Error("the handshake ran past its limit"));
        }
        return;
      }
      clearTimeout(timer);
      resolve({ line: buffered.slice(0, end) });
    };
    child.stdout.setEncoding("utf8");
    child.stdout.on("data", onData);
    child.on("exit", (code, signal) => {
      clearTimeout(timer);
      reject(new Error(`the kernel exited with ${code ?? `signal ${signal}`} before saying anything`));
    });
    child.on("error", (err) => {
      clearTimeout(timer);
      reject(new Error(`the kernel could not be started: ${err.message}`));
    });
  });
}

// The first cases need no model and no state: the gate's refusals are contract
// too, and each names its invariant with a dotted code the Rust gate must
// reproduce. Business cases start the wider set and grow per layer.
export const CASES = [
  { slug: "get-runtimes", method: "GET", path: "/runtimes" },
  { slug: "get-events-sse", kind: "sse", method: "GET", path: "/events" },
  { slug: "get-unknown-route", method: "GET", path: "/definitely-not-a-route" },
  { slug: "post-runtimes-no-origin", method: "POST", path: "/runtimes", origin: false },
  { slug: "get-bad-host", method: "GET", path: "/runtimes", host: "example.com:80" },
  { slug: "get-no-credential", method: "GET", path: "/runtimes", credential: false },
  { slug: "post-runtimes", method: "POST", path: "/runtimes", body: {} },
];

export async function recordAll(root, outDir) {
  const work = mkdtempSync(path.join(tmpdir(), "reasonix-golden-"));
  const stateHome = path.join(work, "state");
  mkdirSync(stateHome);
  const child = spawn(buildHost(root, work), [], {
    cwd: root,
    env: { ...process.env, REASONIX_STATE_HOME: stateHome, REASONIX_HOME: stateHome },
    stdio: ["ignore", "pipe", "inherit"],
  });
  try {
    const { line } = await readHandshake(child);
    const { origin, token } = parseHandshake(line);
    const port = Number(new URL(origin).port);
    const cases = [];
    for (const caze of CASES) {
      if (caze.kind === "sse") {
        const headers = { cookie: `${TOKEN_COOKIE}=${token}` };
        const { status, frames } = await collectStream(port, caze.path, headers);
        cases.push({
          slug: caze.slug,
          kind: "sse",
          request: { method: caze.method, path: caze.path, headers: { cookie: `${TOKEN_COOKIE}=__CREDENTIAL__` } },
          response: { status, frames: frames.map((f) => scrub(f, { token, origin })) },
        });
      } else {
        const req = requestFor(caze, origin, token);
        const res = await sendHttp(req, { port });
        const shown = { ...req.headers };
        if (shown.cookie) shown.cookie = `${TOKEN_COOKIE}=__CREDENTIAL__`;
        if (shown.origin) shown.origin = "http://127.0.0.1:__PORT__";
        cases.push({
          slug: caze.slug,
          kind: "http",
          request: { method: req.method, path: req.path, headers: shown, body: req.body },
          response: {
            status: res.status,
            headers: JSON.parse(scrub(JSON.stringify(res.headers), { token, origin })),
            body: scrub(res.body, { token, origin }),
          },
        });
      }
    }
    writeFileSync(
      path.join(outDir, "manifest.json"),
      `${JSON.stringify({ handshakeVersion: HANDSHAKE_VERSION, cases }, null, 2)}\n`,
    );
    return cases.length;
  } finally {
    child.kill();
    // Windows holds the exe open until the process is reaped, so rm's first
    // pass lands EPERM; wait out the exit, then retry through the lock.
    await new Promise((resolve) => {
      if (child.exitCode !== null) return resolve();
      child.once("exit", resolve);
      setTimeout(resolve, 10000);
    });
    rmSync(work, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 });
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(realpathSync(process.argv[1])).href) {
  const root = path.resolve(import.meta.dirname, "..");
  const out = process.argv[2] ?? path.join(root, "crates", "serve", "fixtures", "golden");
  mkdirSync(out, { recursive: true });
  const n = await recordAll(root, out);
  console.log(`recorded ${n} golden cases into ${path.relative(root, out)}`);
}
