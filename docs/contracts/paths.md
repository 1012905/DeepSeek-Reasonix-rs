# Paths and Environment Contract — Frozen Inventory (M1)

Frozen inventory extracted from source on 2026-11-15. Crate targets per §0.2
(AGENTS.md 5.1 names reasonix-platform; the implementation spans a few crates
per ADR-0001). Reference sources: docs/CONFIG_PATHS.md, internal/contract/config
(path machinery), internal/base/{secrets,hostwrite,i18n,testenv,testscope},
internal/platform/environment, internal/state/migration/relocated_state.go,
internal/base/fileutil/vcs.go.

## 0. Overview

### 0.1 Scope

This contract freezes:

- where every user-visible file lives (config, credentials, sessions, caches,
  locks, logs) and the environment variables that redirect them;
- config file selection and config edit locking;
- dotenv + credential store file handling (cross-references config.md);
- legacy migration and state relocation;
- secret path deny lists and host-writable dirs;
- subprocess environment filtering and environment redaction;
- environment probes (the `## Environment` prompt section);
- language detection;
- test isolation.

### 0.2 Crate layout (ADR-0001)

| Go source | Rust crate | Note |
|---|---|---|
| internal/contract/config: paths.go, storage_roots.go, storage_config.go, dotenv.go, credentials.go, apikeyenv.go, migrate.go, mutate.go, path_access.go, workspace_roots.go, project_program_paths.go | reasonix-config | frozen in depth in config.md §10/§11/§15; this doc is the platform-side companion |
| internal/base/secrets (credpaths.go, hostsecrets.go, redact.go, endpoint.go, diagnostic.go) | reasonix-core | layer 0 — depended on by reasonix-config |
| internal/base/hostwrite | reasonix-core | leaf |
| internal/base/i18n/detect.go | reasonix-core | leaf |
| internal/base/fileutil/vcs.go | reasonix-core | leaf |
| internal/base/testenv, internal/base/testscope | reasonix-testkit | test-only |
| internal/platform/environment | reasonix-platform | AGENTS.md 5.1 primary target |
| internal/state/migration/relocated_state.go | reasonix-state | state relocation adoption |

### 0.3 Relationship to other frozen contracts

- config.md §11 — the authority for storage roots, RootID set, precedence,
  defaults, StateRootEntries, derived path functions, WorkspaceSlug,
  ConventionDirs, project config paths, and the docs/CONFIG_PATHS.md
  user-facing contract. This doc recaps and cross-references; it does not
  restate every table.
- config.md §10.15 (path access), §10.27 (legacy import), §10.29 (ExpandVars),
  §10.30 (dotenv), §10.31 (credential store), §10.32 (api key env naming),
  §15.2 (storage_config), §15.3 (save paths, config file perms).
- host.md §7 — desktop-side environment variables (REASONIX_STUDIO_HOST,
  REASONIX_STUDIO_PAGE, REASONIX_CUA_DRIVER, REASONIX_STUDIO_HANDED_OVER,
  REASONIX_STUDIO_HANDSHAKE_TIMEOUT_MS).
- tools.md §3.7/§7.1 — the tools that consume workspace roots, path
  resolution, and file views.

## 1. Storage roots and derived paths (authority: config.md §11)

### 1.1 The closed root set

| RootID | Env pin | Relocatable | Default |
|---|---|---|---|
| home | REASONIX_HOME | no | Windows %APPDATA%/`reasonix`; unix `~/.reasonix` |
| state | REASONIX_STATE_HOME | yes | = home |
| cache | REASONIX_CACHE_HOME | yes | Windows %LOCALAPPDATA%/reasonix/cache; macOS ~/Library/Caches/reasonix; Linux `$XDG_CACHE_HOME/reasonix` or ~/.cache/reasonix |
| worktrees | — | yes | pinned state → state/worktrees; Windows → cache/worktrees (non-roaming); else state/worktrees |
| locks | — | no | osCacheBase IGNORING the binding, on purpose: locks must converge across isolated instances (lock files outlive use) |

Precedence for every root: (1) pinned dir (env, stated home for home);
(2) [storage] table in <home>/config.toml (relocatable roots only; malformed
or unreadable → empty set so paths still resolve; relocation takes effect
next launch); (3) default. expandDirValue: TrimSpace → ExpandVars → bare
`~`/`~/...` expansion → Abs if relative → Clean.

### 1.2 StateRootEntries (29, top of the state root)

sessions, archive, stats, projects, appearance, themes, repair,
serve-workspaces.json, memory, schedules, peers, trusted, machine-id.key,
mcp-global-migration-v1, commands, rules, agent-presets, sandbox, workspaces,
REASONIX.md, AGENTS.md, CLAUDE.md, REASONIX.local.md, AGENTS.local.md,
CLAUDE.local.md. First-move subset [sessions, archive, stats, projects];
the remainder is StateRootEntriesEarlyMovesLeft (read by recovery and panel).

### 1.3 Derived paths (recap; full table in config.md §11)

UserConfigPath <home>/config.toml; UserCredentialsPath <home>/.env;
SessionDir <state>/sessions; ArchiveDir <state>/archive; StatsDir
<state>/stats; ProjectSessionDir <state>/projects/<slug>/sessions;
MemoryUserDir = Dir(state); ScheduleDir <state>/schedules; SandboxDir
<state>/sandbox; PeersDir <state>/peers; RemoteStateDir <home>/remote;
RemoteKnownHostsPath <home>/remote/known_hosts; WorkspaceLeaseDir
<locks>/workspace-leases (follows the cache root — a lease must never dirty
the repo it protects); RepairMutationLockDir <locks>/repair-mutation-locks
(deliberately ignores Reasonix overrides: isolated instances repair the same
project config, locks must converge); MemoryLockDir <locks>/memory-store-locks;
CacheDir Dir(cache); AutoWorkspaceDir <state>/workspaces/auto (caller creates);
DeliveryWorktreeDir Dir(worktrees).

WorkspaceSlug: Windows lowercases the abs path first (case-fold matching
sessionstore.CanonicalSessionPath; existing mixed-case slugs need no
migration); separators and ':' → '-'; boundFilenameComponent caps at 255
bytes (inputs within budget pass byte-identical; over budget → truncate to
maxLen-17, back off to a rune boundary, append '-' + 16 hex FNV-1a64).

IsolatedHomeDir() = cleanEnvDir(REASONIX_HOME); IsolatedStateDir() =
cleanEnvDir(REASONIX_STATE_HOME). Both gate migration and relocation
behavior throughout this contract.

## 2. Config file selection

### 2.1 User config chain (userConfigLoadPath)

1. <home>/config.toml (primary) — used when os.Stat succeeds;
2. else legacyUserConfigPath = osUserConfigDir()/reasonix/config.toml when
   different from primary (empty when home is pinned);
3. else legacyXDGConfigPaths (unix only; skipped when home is pinned):
   XDG_CONFIG_HOME/reasonix/config.toml, ~/.config/reasonix/config.toml.

userConfigCandidatePaths() = [userConfigPath, legacyUserConfigPath] +
legacyXDGConfigPaths. isUserConfigPath compares Abs+Clean equality against
this list (used by save/edit scoping). userConfigDisplayPath renders
~-relative for config comments. legacyOSSupportDir: pinned home → '';
osUserConfigDir()/reasonix unless it is the same path as the current Home.

### 2.2 Project config

projectConfigFile = `reasonix.toml`. ProjectConfigPaths = 
[root/reasonix.toml, root/.reasonix/reasonix.toml]. ProjectConfigPath probes
with os.OpenRoot-confined Lstat; ONLY IsNotExist permits fallback (broken
link or access error stays on the higher-priority path); the root-level path
is the creation target. SourcePathForRoot: project TOML if it exists, else
userConfigLoadPath if it exists, else ''. Full resolution semantics
(symlink confinement, plainConfigEntry fallback for cloud drives) are frozen
in config.md §10.15.

### 2.3 Legacy config paths

- legacyConfigPath: ~/.reasonix/config.json (v0.x TypeScript-line config);
  '' when home is pinned. Parsed by loadLegacyMCP (mcpServers + legacy mcp
  string list; mcpServers wins name collisions; mcpDisabled skipped;
  absent or malformed → nil, the lowest-priority source).
- ReasonixManagedConfigPaths: the files model tools may repair (per-write
  human approval): userConfigPath + legacyUserConfigPaths + legacyConfigPath
  — files only, never directories (the home also holds .env, settings.json,
  skills, sessions, which must not ride along).

## 3. Config edit locks (mutate.go)

### 3.1 API

LockConfigFileEdits(path) / LockConfigFilesEdits(paths...) — in-process mutex
plus an advisory filelock.Acquire under configEditLockTimeout = 5s
(userConfigEditLockTimeout = 5s). Aliases resolving to the same final file
share ONE advisory lock; logical paths are pinned to the resolved target
until unlock. Lock order when both are needed (config.md §10.31): user
config edits first, then credential edits.

### 3.2 Lock file and registry directory

- Lock file name: sha256(lockKey) + `.lock`, where lockKey =
  Clean(resolved path), lowercased ToSlash on windows/darwin.
- configEditPathKey: TrimSpace (empty → `lock config edits: empty config path`),
  Abs+Clean, windows lowercase ToSlash.
- Registry dir when an isolated home holds the file (homeHolds: pathWithin
  by spelling OR EvalSymlinks(home) pathWithin):
  <home>/locks/config-edits-<sha256(identity)[:8]> — a self-contained runtime
  writes nothing outside its home.
- Production registry dir: <home>/.reasonix/locks/config-edits-<sha256(identity)[:8]>,
  where home is current.HomeDir overridden by testscope.Home(). home empty →
  windows error `lock config edits: OS user home unavailable`; unix
  fallback /tmp/reasonix-config-locks-<digest[:8]>. `Production locks use
  the account database, not HOME or TMPDIR.`
- Identity: osuser Uid → Username → HomeDir (all empty → error).

### 3.3 Lock directory safety (acquireConfigEditLockPath)

MkdirAll(lockDir, 0o700). Lstat: symlink or not-a-directory → `lock config
edits: unsafe lock directory`. Foreign owner (unix: Stat_t.Uid differs
from os.Getuid; Windows: no-op, there is no uid ownership check) →
ErrLockDirWrongOwner `lock directory is owned by another user` +
`lock directory %s is owned by uid %d, not the current user: %w`.
Chmod 0o700. filelock.Acquire with a 5s context.

## 4. Dotenv and credential store files (authority: config.md §10.30/§10.31)

### 4.1 File-level facts

- UserCredentialsPath = <home>/.env. Writes: MkdirAll 0700, CreateTemp
  `credentials.*.tmp` in the same dir, Chmod 0600, fileutil.ReplaceFile
  (atomic).
- Cleared-key tombstone line: `# reasonix-cleared KEY`. Re-setting a
  key drops its tombstone; RemoveCredential removes the key line and appends
  the tombstone + os.Unsetenv. envFileHasClearedKey scans tombstones.
- CredentialStoreRevision() = `sha256:<hex>` of the file bytes, or the
  sentinels `missing` / `unreadable` / `unavailable` when
  there is no path. SetCredentialIfRevision is compare-and-apply under the
  lock.
- Project .env control-key filter (isProjectDotEnvControlKey): any REASONIX_*
  prefix plus HOME, USERPROFILE, APPDATA, XDG_CONFIG_HOME, XDG_CACHE_HOME,
  XDG_STATE_HOME are never taken from a project file.
- CredentialEnvNames(): provider api_key_envs + bot passthrough names + remote
  host passphrase/password envs + ALL keys present in .env (stale entries
  stay out of child envs too).
- API key env naming: config.md §10.32 (APIKeyEnvFor; CUSTOM_<fnv1a32Hex>_API_KEY
  for names that sanitize to nothing; CUSTOM_ prefix for leading digits;
  FreeAPIKeyEnvFor → REASONIX_CONNECTION_<32 uppercase hex>_KEY;
  IsPrivateCredentialSlot; ErrInvalidCredentialKey
  `credential slot name is not a usable environment variable`).

### 4.2 Credential store mode

auto | keyring | file. Resolution: env REASONIX_CREDENTIALS_STORE → [credentials_store]
in the user config load path → normalize (keyring|file else auto).
credentialsKeyringService = `reasonix`.

## 5. Legacy migration and state relocation

### 5.1 v1.8.1 one-time migration (MigrateLegacyIfNeededForRoot)

Skipped when pinnedHomeDir() != `` (no migration under REASONIX_HOME).
Credentials migration runs FIRST (its error is carried). LockConfigFileEdits(dest).
dest exists → nil (one-time). migrateLegacyTOMLIfNeeded first:
legacyTOMLPaths = legacyUserConfigPath, legacyXDG paths + sibling
reasonix.toml, Dir(dest)/reasonix.toml, home/.reasonix/reasonix.toml (deduped,
dest excluded); first existing wins: mergeFile into Default(); ConfigVersion
reset to default; CloseBehavior fold (Desktop.CloseBehavior empty +
UI.CloseBehavior set → fold); write dest; migrateSupportData(legacyDir, newDir).

Else ~/.reasonix/config.json (UTF-8 BOM EF BB BF tolerated):
- lang → Language + SetDesktopLanguage; model → ResolveModel fold or raw.
- migrateLegacyBaseURL: official DeepSeek → kind anthropic +
  deepSeekAnthropicBaseURL; non-official → kind openai + baseURL, clearing
  Thinking/WebSearch/SupportedEfforts/DefaultEffort/ModelOverrides.
- legacyPlugins: mcp string list, then mcpServers sorted by name (mcpServers
  override the list on name collision); disabled → AutoStart=false;
  anonymous → anonymousMCPName(i) = `mcp-%d` (i+1).
- apiKey → `DEEPSEEK_API_KEY=` line + KeyToEnv + warning when base_url
  is non-deepseek: `your previous base_url was %s — it was applied to the
  built-in DeepSeek providers; verify models if this endpoint is not
  DeepSeek-compatible`.
- MkdirAll 0755; writeCredentialsEnv fallback: home/.env 0600 ONLY when
  UserCredentialsPath() is empty (never a project .env).

### 5.2 MigrationResult and notice

MigrationResult{From, To, KeyToEnv, Plugins, Warnings}. Notice():
`migrated your previous configuration: %s → %s` + optional
` (%d MCP server(s))` + `; API key saved to reasonix's credentials
store` + `. The old files were left untouched.` + per-warning
`\n  note: ` + text.

### 5.3 migrateSupportData

Items: sessions, projects, skills, archive (directories, recursive copy) +
hooks.json, settings.json (files; an existing destination is NEVER
overwritten — warning `kept existing file %s`). Warnings:
`successfully migrated directory %s` / `successfully migrated file %s` /
`failed to migrate directory %s: %v` / `failed to migrate file %s: %v` /
`failed to read legacy item %s: %v`. copyFile: parent 0755 (0700 when
source perm &0077==0); source perm 0 → 0600; copyDir perm 0 → 0700.

### 5.4 v1.9.1 MCP backfill (MigrateMCPToUserConfigOnUpgrade)

Marker <state>/mcp-global-migration-v1, content `v1\n` mode 0644; skip
when the marker exists (one-time, so user-deleted servers are not recreated).
migrateMCPToUserConfig: load global for edit strict; existing global plugins
win by name; validatePlugin; entry.Source = MCPSourceUserConfig; sources:
legacy TOML paths (excluding dest), project roots (ProjectConfigPath +
root mcp JSON file), legacy config.json; SaveTo only when Added > 0; result
{To, Added, Sources}.

### 5.5 Legacy keyring migration (migrateLegacyCredentialsIfNeededForRoot)

Legacy FILES first (healthy import independent of Secret Service/D-Bus,
issue #7507): legacyCredentialsPaths() parsed via parseCredentialLines;
missing keys collected unless the current store has or cleared the key
(keyring markers do NOT skip files, only keyring probes). Then keyring batch
probe under legacyKeyringLookupTimeout = 1s shared budget: found → stored by
probe; absent → markLegacyKeyringMigrationDone; error or timeout → NO marker
(retry next launch). Keyring marker path:
<home>/state/legacy-keyring-checked/<base64 RawURL of key>, content `v1\n`
mode 0644 (env names are identifiers, not secrets).

### 5.6 State relocation adoption (relocated_state.go)

AdoptRelocatedStateEntries(sink) — skipped when config.IsolatedHomeDir() or
config.IsolatedStateDir() is set (a redirected run did not ask to move the
production install). For each StateRootEntriesEarlyMovesLeft entry:
src = <home>/<name>, dst = <state>/<name>; Lstat src, skip symlinks and
ModeIrregular (`a link or junction points at data this run does not own`);
serve-workspaces.json → workspacelist.MergeOnce; else copyMissingTree;
n==0 && !isEmptyDir(src) → leave in place; else fileutil.RenameAside(src,
`.adopted` suffix = workspacelist.AdoptedSuffix) so a user-deleted file
is not resurrected. Nothing in the new root is overwritten. Warnings
`could not bring %s across from the previous storage location: %v`;
info `brought %v across from the previous storage location`.

### 5.7 /migrate rescue (user-facing; docs/CONFIG_PATHS.md)

Checks legacy config/credentials, scans legacy memory + session dirs,
imports, prints a summary. /migrate --from 'D:\OldReasonix' imports v0.x
sessions ONLY; source-specific marker; never overwrites an existing
config.toml; memory copied only when the destination is absent; respects
session import markers. Version limits: auto-migration starts at v1.8.1;
/migrate ships in Go builds only (M6 removes it with the sidecar policy).

## 6. Secrets path deny lists (internal/base/secrets)

### 6.1 [secrets] toggles (user-global; config.md §2 defaults)

| Key | Default | Toggle | Effect |
|---|---|---|---|
| filter_subprocess_env | off | SetFilterSubprocessEnv / FilterSubprocessEnv | off: child envs keep credential-like inherited vars (gh/git/npm workflows), only credential-store values removed; on: FilterEnv strips every sensitive or registered key |
| protect_sensitive_files | off | SetProtectSensitiveFiles / ProtectSensitiveFiles | the name-pattern half: deny reading SensitiveFileName matches (§6.2) |
| protect_credential_files | on | SetProtectCredentialFiles / ProtectCredentialFiles | the anchored-path half (§6.2); inverted atomic (disableCredentialProtection), so unwired callers stay confined |

Wiring (assembly/boot/build.go + boot.go): SetFilterSubprocessEnv,
SetProtectCredentialFiles, RegisterCredentialEnvKeys(cfg.CredentialEnvNames()).
Toggles are atomic and set once by the composition root — [secrets] cannot
be overridden per-project.

### 6.2 Credential paths (credpaths.go)

- sshPublicNames = {config, known_hosts, known_hosts.old, authorized_keys,
  environment, rc} — ~/.ssh is judged by exception (keys take user-chosen
  names). CredentialReadPath(abs): within ~/.ssh → deny unless public name
  or .pub; plus the exact paths below.
- credentialExactPaths: ~/.aws/credentials, ~/.pypirc, ~/.netrc, _netrc,
  ~/.git-credentials, ~/.config/gcloud/application_default_credentials.json,
  ~/.config/gcloud/credentials.db, ~/.config/gcloud/access_tokens.db.
- homeCandidates = resolved home + raw home both (fail-closed for callers
  that skip EvalSymlinks).
- CredentialFilePaths(): snapshot of EXISTING credential files, for OS
  sandbox deny lists (a key written later is denied only from the next
  build; CredentialReadPath still hides it from read tools).
- CredentialDirs() = ~/.ssh, ~/.gnupg + parent dirs of the exact paths
  (~/.aws, ~/.config/gcloud). SensitiveHomeDirs() = [~/.ssh].
- SensitiveFileName: .env, .git-credentials, .netrc, id_rsa, id_dsa,
  id_ecdsa, id_ed25519, .env.* (except .example/.sample/.template
  suffixes), *.pem, *.key, *.p12, *.pfx.
- foldPaths = windows || darwin (deny-side case folding).
- ForbiddenReadPaths(broad) = CredentialFilePaths + SensitiveHomeDirs when
  broad (the name-pattern half cannot come along — only anchored paths can
  be masked or denied).

### 6.3 Host secret paths (hostsecrets.go)

RegisterHostSecretPath records files THIS process reads secrets from
(serve --token-file etc.) so sandboxes and read tools deny them;
HostSecretPaths() lists them.

## 7. Host-writable dirs (internal/base/hostwrite)

Dirs() = host dirs a jailed command may write besides its write roots.
Windows: nil (no jail). darwin: /tmp, /private/tmp, /private/var/folders,
os.TempDir, + home {Library/Caches, .cache, .npm, .cargo, go}. linux:
os.TempDir when != /tmp, + home {.cache, .cargo, .npm, go}. Caches()
(toolchain caches on every platform): ~/.cache, ~/.cargo, ~/.npm, ~/go,
+ darwin ~/Library/Caches.

## 8. Subprocess environment (internal/base/secrets/redact.go)

### 8.1 Credential env keys

RegisterCredentialEnvKeys: process-lifetime UNION of names whose values came
from the credential store (uppercased, trimmed) — concurrent workspaces
cannot make each other's keys visible. EnvKeySensitive(key) = CredentialKey(key)
(§9.2). FilterEnv drops KEY=value items with sensitive or registered keys.

### 8.2 ProcessEnv()

- !FilterSubprocessEnv() → markAgentChild(filterRegisteredCredentialEnv(os.Environ())):
  credential-store values are ALWAYS removed; other credential-like
  inherited vars are kept by default (preserves gh/git/npm workflows).
- FilterSubprocessEnv() → markAgentChild(FilterEnv(os.Environ())).
- AgentChildEnv = `REASONIX_AGENT_CHILD`; markAgentChild strips any
  existing value and appends `REASONIX_AGENT_CHILD=1` — friction
  against terminal-needing commands, not a boundary.
- Rule: every child spawn (probes included) must pass an explicit cmd.Env
  built from ProcessEnv() + additions — a nil Env would inherit unfiltered.

## 9. Environment redaction (internal/base/secrets)

### 9.1 redact.go — patterns and mask

redactedValue = `[redacted]`. Patterns, applied in order by Redact(s):
urlUserInfo (scheme://user@ → ${1${[redacted]@) → redactKeyValues →
cookie headers → Bearer (masked) → known key shapes
(openAI/github/slack/aws/jwt).

| Pattern | Shape |
|---|---|
| bearerToken | (?i)Bearer\s+ followed by [A-Za-z0-9._~+/=-]{16,} |
| openAIKey | (sk|rk)-(proj-)?[A-Za-z0-9_-]{12,} |
| githubToken | gh[pousr]_ / github_pat_ + 20+ |
| slackToken | xox[baprs]- |
| awsAccessKey | (AKIA|ASIA)[0-9A-Z]{16} |
| jwt | eyJ… (three-part dot shape) |
| urlUserInfo | scheme://user@ → ${1${[redacted]@ |
| cookieHeader | Cookie/Set-Cookie: every pair value masked; valueless flags untouched |
| maskedCredential | runs of ** collapsed |
| credentialContext | (?i)(api key|access key|secret|token|authorization|bearer|credential)s? + separator + 12+ value → ${1${${2${**** |
| credentialToken | [A-Za-z0-9_-]{16,} fallback — masked only with a digit or mixed case; single-case digit-free identifiers stay readable |
| endpoint | scheme://… (feeds RedactEndpoint) |

redactKeyValues: scans ':'/'=' separators; key extracted back over
credentialKeyByte chars + optional quotes; credentialTextKeySensitive
(CredentialKey minus cookie/set-cookie); authorizationScheme
{bearer, basic, digest, negotiate, ntlm, token, bot, apikey} skips the
scheme word; authorizationKey (keys ending AUTHORIZATION) → full
[redacted]; already-masked values kept; else mask(value).

mask(value): trim; empty or len ≤ 12 → [redacted]; head 4 tail 4 (head 6
for sk-/rk- shapes); value[:head] + '*'*(len-head-tail) + value[len-tail:].

RedactCredentials(s): endpoints → RedactEndpoint(trimmed) + suffix; Redact;
credentialContext → `****`; maskedCredential → `****`; opaque
tokens with digit or mixed case → `****`. RedactError mirrors it for
error strings.

### 9.2 endpoint.go — CredentialKey

CredentialKey(key): loop url.QueryUnescape (decode error or 32 levels →
true) until stable; NFKC normalize; insert space at camelCase boundaries;
case fold; folded `pwd`/`oldpwd` → false; split on non-letter/
non-digit; ANY part in {token, password, passwd, pwd, secret, key, auth,
authorization, credential, credentials, signature, sig, session, cookie,
bearer, jwt, apikey, access, private} → true.

### 9.3 endpoint.go — projection

EndpointRedacted = `<redacted>`; projectionDepth = 32.
RedactEndpoint (projectEndpoint, depth 0): empty or already redacted →
as-is; url.Parse failure / no Host / no Scheme / Opaque → <redacted>; strip
u.User; a path segment where CredentialKey(segment) && not the last segment
→ truncate the path at that segment + `/<redacted>`; RawQuery
containing ';' → scheme://host/<redacted>; projectQuery (ParseQuery; per
key: CredentialKey(key) → every value <redacted>, empty stays empty; else
projectValue(value); keys themselves projected); fragment: contains '=' →
projectQuery, '://' → projectEndpoint, else <redacted>; unchanged → raw.

CredentialValue(v) = RedactConfigValue('', v) != v. RedactConfigValue(key,
value): CredentialKey(key) → <redacted>; projectValue: '//'-containing
no-space URL-ish → projectEndpoint; `key:nested` → redacted when
CredentialKey(key) or the nested value redacts; `key=value` →
<redacted> when CredentialKey(key); query-shaped → projectQuery; %xx decode
retry; whitespace-containing → parse as a static command (shellparse) and
projectArgs-compare. RedactArgs/projectArgs: `-H*`/`-e*` (len>2)
→ arg[:2]+'<redacted>'; carriers {-H, --header, --headers, --env, -e}:
inline `k=v` → `k=<redacted>`, else the next arg <redacted>;
`-<CredentialKey>:` header form → all remaining args <redacted>; flag
keys where CredentialKey(trimmed dashes); Basic/Bearer scheme keys.

### 9.4 diagnostic.go

DiagnosticError(err) — keeps identity, omits untrusted prose: a
DiagnosticFacts() interface wins; *url.Error → `request failed: <RedactEndpoint(url)>: <DiagnosticError(err)>`;
{context.Canceled, context.DeadlineExceeded, io.EOF, io.ErrUnexpectedEOF}
verbatim; else `dependency failure (%T)`. OmittedText(text) →
`dependency output omitted (%d bytes)`.

## 10. Environment probes (internal/platform/environment)

### 10.1 Constants

ProbeTimeout 2s; probeWaitDelay 1s; probeCacheTTL 5min; probeSnapshotTTL
24h; probeSnapshotVersion 2; maxRenderedTools 24.

### 10.2 Default probe set (11)

`go version`, `python3 --version`, `python --version`,
`node --version`, `npm --version`, `rustc --version`,
`cargo --version`, `git version`, `make --version`,
`rg --version`, `docker --version`.

### 10.3 Types and error taxonomy

ProbeResult{Command, Binary, Output, Found, Error, Path}.
ProbeOptions{Overrides map, DenyRoots []string, SnapshotDir}.

Error taxonomy: `not found` (exec.LookPath miss, or an override that
is not an executable file) — definitive; `not trusted` (override not
absolute, or blockedExecutable: the resolved binary lies inside DenyRoots) —
definitive; `timeout` (2s) — transient; `exit <err>` when the
command failed with no output — transient; firstLine(output) as the error
text when output exists.

### 10.4 Resolution (runOne)

shellparse.ParseStaticCommand (AllowEnvAssignments,
AllowStderrToStdout). Override → must be absolute (else `not trusted`)
and fileExecutable (else `not found`); else exec.LookPath (else
`not found`); blockedExecutable → `not trusted`.
cmd.Env = secrets.ProcessEnv() + probe.Env — ALWAYS explicit (nil would
bypass [secrets] filter_subprocess_env). prepareProbeCommand: proc.HideWindow,
SetProcessGroupKill, Cancel=proc.KillTree, WaitDelay=1s; stdout and stderr
combined; success → Found=true, Output=firstLine. sortResults: Found first,
then Binary ascending.

### 10.5 Caching layers

1. In-process cache, TTL 5min, keyed by fingerprint.
2. Singleflight: beginProbe/releaseProbe — one caller probes, the others
   wait (probeWaitDelay); denied answers are handed to waiters without
   being recorded.
3. Snapshot, TTL 24h, at <SnapshotDir>/environment/probes-<sha256(fingerprint)[:8]>.json,
   JSON {version, fingerprint, stored_at, results}. Version 2 only: v1
   files are not read; v2 records the resolved Path so an answer can be
   re-checked against another workspace's deny roots; the fingerprint keys
   on PATH, not deny roots. saveProbeSnapshot: MkdirAll 0755, CreateTemp
   `.probes.*.tmp`, fileutil.ReplaceFile.

probeFingerprint = `v1` + trimmed commands (\x00 separator) + sorted
overrides name+'='+expandHome(value) + `path=` + os.Getenv(`PATH`).

### 10.6 Snapshot freshness rules

reusableHere: no `not trusted` result; every found result has a
non-empty Path and !blockedExecutable(Path, denyRoots). denied(results)
(any `not trusted`) → releaseProbe, NOT saved. An expired snapshot
still anchors flap merging: mergeProbeSnapshot keeps a previous found
result for the same Command when the fresh result is a
transientProbeFailure (!Found && (Error==`timeout` || prefix
`exit `)); `not found`/`not trusted` are definitive and
adopted as-is.

### 10.7 FormatSection — exact output

`## Environment` header; `- OS: <osName>` (default
GOOS+'/'+GOARCH); `- Shell used by the bash tool: <redactHome(shellPath)>`;
`Configured tools:` sorted, capped at 24,
`- ... %d more configured tools omitted`; `Detected tools:`
found entries with Output or `available`, capped + omitted line;
`Not found or unavailable:` Error text or `not found`, capped +
omitted; closing `Use detected tools when appropriate. Do not try
unavailable tools unless the user installs or configures them.`;
TrimRight newlines; empty → ''. redactHome: ~ substitution + ToSlash.

### 10.8 Workspace VCS (workspace_vcs.go + fileutil/vcs.go)

WorkspaceVCS(root): walk up from Clean(root) (''/'.' → Getwd); the first
existing fileutil.VCSStores() marker wins (a worktree .git is a file);
MarkerOf → Marker{Path, Dir bool}; store.Name reported. vcsStores probe
order: .jj (jj) → .git (git) → .hg (hg) → .svn (svn). jj before git
because a jj repo colocated with git keeps both, and the one the user
drives is the one a plain git checkout does not have. A store is not the
work product — `git status` alone rewrites the index — so nothing
downstream may read a change under one as evidence the tree was touched.
The per-project VCS fact rides the turn block, not this section
(cache-stability).

### 10.9 Assembly integration (assembly/boot/prompt.go)

buildPromptAssembly folds the environment section INSIDE the cache-stable
prompt prefix. When cfg.EnvironmentEnabled(): shellLabel = shell.Kind.String();
cfg.Tools.Shell.Path non-empty → shell.Path; shell.Description() non-empty →
it. The section is environment.FormatSection(
RunProbesWithOptions(ctx, DefaultProbes(), ProbeOptions{Overrides:
cfg.Environment.Tools, DenyRoots: []string{root}, SnapshotDir:
opts.roots().CacheDir()}), GOOS+'/'+GOARCH, shellLabel,
cfg.Environment.Tools) — appended only when the persona sets
IncludeRuntimeContext (the probe still runs — the snapshot keeps boots
stable). Prompt prefix order: base prompt → persona prefix → output style
→ core policies → persona suffix → environment section → offline note →
memory → skills.

## 11. Language detection (internal/base/i18n/detect.go)

DetectLanguage(override) candidate order: override → REASONIX_LANG →
LC_ALL → LC_MESSAGES → LANG → REASONIX_SYSTEM_LANG → OS probe
(detectOSLanguage) → `en`. REASONIX_SYSTEM_LANG is deliberately LAST
among env candidates: explicit variables win, and it is only what the
machine was installed as (the Studio host sets it from Electron's preferred
system languages — host.md §6).

normalize: lowercase + trim; '_' → '-'; C/POSIX (including `c.`/
`posix.` prefixes) → `en`; zh-tw*/zh-hant*/`chinese
traditional`/繁體 → `zh-TW`; zh*/`chinese`/中文 → `zh`;
en*/`english` → `en`; else '' (falls through to the next
candidate). setLanguage: zh-TW → ChineseTraditional; zh → Chinese; default
English; returns the current language.

## 12. Test isolation (internal/base/testenv + testscope)

IsolateUserState(): MkdirTemp(origHome, `.reasonix-test-home-*`)
(fallback: system temp); sets HOME, USERPROFILE, REASONIX_TEST_HOME to it,
XDG_CONFIG_HOME=<home>/.config, XDG_CACHE_HOME=<home>/.cache,
XDG_STATE_HOME=<home>/.local/state, AppData=<home>/AppData/Roaming,
LocalAppData=<home>/AppData/Local; UNSETS REASONIX_HOME, REASONIX_STATE_HOME,
REASONIX_CACHE_HOME; testscope.InstallHome(home); restore on cleanup +
RemoveAll. RunWithIsolatedUserState(m) wraps a test main.

testscope: an atomic pointer holding the explicitly installed user-state
home; Home() / InstallHome(home) (nested scopes restore in reverse
installation order). Consulted by configEditLockRegistryDir (§3.2) so tests
can pin the lock-registry home.

## 13. Environment variable inventory

Kernel-side REASONIX_* variables (non-test internal + cmd .go):

| Variable | Meaning | Defining source |
|---|---|---|
| REASONIX_HOME | pins the user home (all config/state/cache under it; migration, OS-convention scanning, fallbacks skipped) | contract/config storage_roots.go (17 files read it) |
| REASONIX_STATE_HOME | moves runtime state only (sessions, archives, memory); NOT global config/credentials | storage_roots.go |
| REASONIX_CACHE_HOME | pins the cache root; wins over <home>/cache when both set | storage_roots.go |
| REASONIX_CREDENTIALS_STORE | auto / keyring / file | contract/config credentials.go |
| REASONIX_CONNECTION_<32HEX>_KEY | private credential slot minted by FreeAPIKeyEnvFor | contract/config apikeyenv.go |
| REASONIX_AGENT_CHILD | appended to every child env (=1) | base/secrets redact.go |
| REASONIX_LANG | user-set language override (2nd candidate) | contract/config; base/i18n detect.go |
| REASONIX_SYSTEM_LANG | machine-installed language (LAST env candidate) | base/i18n detect.go |
| REASONIX_WORKSPACE_ROOT | workspace root value the host owns; outranks process env in scoped expansion | contract/config expand.go, plugin_packages.go; ext/hook env_contract.go |
| REASONIX_PROXY_PASSWORD | proxy password expansion | contract/config render.go |
| REASONIX_REMOTE_* | remote credential expansions | contract/config remote_credentials.go |
| REASONIX_OTEL, REASONIX_OTEL_* | OpenTelemetry enablement + config | contract/config observability_config.go; frontend/serve observability.go |
| REASONIX_TELEMETRY | telemetry policy override | platform/telemetry policy.go |
| REASONIX_ACCOUNT_TOKEN | account token | platform/account token.go; ext/configbackup collect.go |
| REASONIX_ACCOUNTS_URL | accounts service URL override | frontend/serve account.go; frontend/cli account.go; cmd/reasonix-studio-host main.go |
| REASONIX_FEEDBACK_URL | feedback service URL override | platform/feedback service.go |
| REASONIX_DNS | DNS resolver override | base/netdns netdns.go |
| REASONIX_EGRESS_BRIDGE | sandbox egress bridge entry | safety/sandbox egress_bridge_entry.go |
| REASONIX_EXPERIMENT_NO_MEMORY | disable the memory store (experiment) | state/memory memory.go; cmd/e2ebench memorybench.go |
| REASONIX_DISABLE_MOUSE | disable TUI mouse capture | frontend/tui screen.go |
| REASONIX_THEME, REASONIX_THEME_STYLE | terminal theme overrides | frontend/termrender theme.go; contract/config render_ui.go |
| REASONIX_UPDATE_PHASE | deb update phase marker | platform/update deb.go |
| REASONIX_ASKPASS_MARK | marks the askpass child | platform/gitpush push.go |
| REASONIX_BASH_PATH__ | test-only bash path override | tools/shellrun loginenv.go; base/testenv loginshell.go |
| REASONIX_LAYA_RESULT__ | test-only laya result injection | model/laya client.go |
| REASONIX_NO_CLIPBOARD_IMAGE__ | test-only clipboard image disable | platform/clipimage clipimage_darwin.go |
| REASONIX_PATH__ | test-only PATH override for stdio plugin transport | ext/plugin transport_stdio.go |
| REASONIX_TEST_HOME | records the isolated test home | base/testenv home.go |
| REASONIX_PLUGIN_NAME | injected plugin name (hook/extension env contract) | ext/hook env_contract.go, composition.go; ext/extension/sidecar process.go, doc.go |
| REASONIX_PLUGIN_ROOT | injected plugin root | ext/pluginpkg manifest_v1.go; ext/hook hook.go, env_contract.go |
| REASONIX_PLUGIN_VERSION | injected plugin version | ext/hook env_contract.go; ext/extension/sidecar process.go, doc.go |
| REASONIX_REMOTE_DEVICE_IDENTITY | remote device identity | platform/remotecloud identity.go |
| REASONIX_REMOTE_URL, REASONIX_REMOTE_GATEWAY_ID | remote host overrides (studio-host) | cmd/reasonix-studio-host main.go |

Convention: the double-underscore suffix __ marks test-only overrides
(REASONIX_BASH_PATH__, REASONIX_LAYA_RESULT__, REASONIX_NO_CLIPBOARD_IMAGE__,
REASONIX_PATH__). Desktop-side variables (REASONIX_STUDIO_HOST,
REASONIX_STUDIO_PAGE, REASONIX_CUA_DRIVER, REASONIX_STUDIO_HANDED_OVER,
REASONIX_STUDIO_HANDSHAKE_TIMEOUT_MS) are frozen in host.md §7.

## 14. Studio logs and kernel caches (recap; authority config.md §11 + docs/CONFIG_PATHS.md)

Kernel caches: macOS ~/Library/Caches/reasonix; Linux
`$XDG_CACHE_HOME/reasonix` or ~/.cache/reasonix; Windows
%LOCALAPPDATA%\reasonix\cache. REASONIX_CACHE_HOME overrides. When
REASONIX_HOME is set, the cache lives under <home>/cache unless
REASONIX_CACHE_HOME is also set (it wins).

Studio logs are NOT in the Reasonix home: macOS ~/Library/Application
Support/Reasonix Studio/io.reasonix.studio.<instance>/logs; Windows
%APPDATA%\Reasonix Studio\io.reasonix.studio.<instance>\logs; Linux
~/.config/Reasonix Studio/io.reasonix.studio.<instance>\logs. Files:
shell.log, host.log, crash/host-<time>-<version>-<pid>.log (newest 5 kept,
each cut to the last 256 KiB, none on clean exit); rotation 1 MiB / 3 files
except crash files. After an unexpected kernel exit the next launch says so
once. The <instance> suffix is the instanceid (host.md §5.1).

## 15. Divergences and notes

No breaks for this area — nothing for docs/parity/BREAKS.md.

Notes:

1. config.md §11 remains the frozen authority for the storage-root machinery
   and derived paths; this doc recaps and cross-references (§1) rather than
   duplicating every table.
2. AGENTS.md 5.1 names reasonix-platform as the Rust target for this
   contract; per ADR-0001 the implementation spans reasonix-core (base/
   secrets, hostwrite, i18n, fileutil), reasonix-config (contract/config
   path machinery), reasonix-platform (platform/environment),
   reasonix-state (relocated_state), reasonix-testkit (testenv). Ledger
   rows name the accurate crate.
3. Every child process must receive an explicit environment built from
   ProcessEnv() — inheriting nil would bypass [secrets]
   filter_subprocess_env (§8.2, §10.4).
4. [secrets] defaults: filter_subprocess_env off (breaks gh/git/npm token
   workflows), protect_sensitive_files off, protect_credential_files on
   (config.md §2; §6.1).
5. REASONIX_SYSTEM_LANG is the LAST env candidate in language detection by
   design (§11).
6. vcsStores probe order puts jj before git (§10.8).
7. The environment probe section is prompt content: it sits in the
   cache-stable prefix and must stay byte-stable across boots within a
   snapshot TTL window (§10.5, §10.9).
8. The locks default dir ignores the storage-root binding on purpose so
   lock files converge across isolated instances (§1.1).

## 16. Proposed ledger rows

| ID | Area | Reference | Rust | Status | Evidence |
|---|---|---|---|---|---|
| PATHS-001 | Paths | Storage roots, env pins, precedence (home/state/cache/worktrees/locks) | reasonix-config | todo | M1: docs/contracts/paths.md §1 |
| PATHS-002 | Paths | StateRootEntries (29) + first-move subset | reasonix-config | todo | M1: docs/contracts/paths.md §1 |
| PATHS-003 | Paths | Derived path functions (config, credentials, sessions, leases, worktrees) | reasonix-config | todo | M1: docs/contracts/paths.md §1 |
| PATHS-004 | Paths | Default dirs incl. locks-dir convergence across isolated instances | reasonix-config | todo | M1: docs/contracts/paths.md §1 |
| PATHS-005 | Paths | User config selection chain (primary → legacy OS-support → legacy XDG) | reasonix-config | todo | M1: docs/contracts/paths.md §2 |
| PATHS-006 | Paths | Project config paths; fallback only on IsNotExist | reasonix-config | todo | M1: docs/contracts/paths.md §2 |
| PATHS-007 | Paths | userConfigCandidatePaths / isUserConfigPath / managed config paths | reasonix-config | todo | M1: docs/contracts/paths.md §2 |
| PATHS-008 | Paths | Config edit locks (aliasing, 5s timeout, unsafe-dir and owner checks) | reasonix-config | todo | M1: docs/contracts/paths.md §3 |
| PATHS-009 | Paths | Lock registry dirs (isolated home vs production, identity, unix tmp fallback) | reasonix-config | todo | M1: docs/contracts/paths.md §3 |
| PATHS-010 | Paths | Credential store file format (0600/0700, tombstones, revision, CAS) | reasonix-config | todo | M1: docs/contracts/paths.md §4 |
| PATHS-011 | Paths | API key env naming (APIKeyEnvFor, private slots, CUSTOM_ forms) | reasonix-config | todo | M1: docs/contracts/paths.md §4 |
| PATHS-012 | Paths | Legacy keyring migration + marker (1s budget, four-state) | reasonix-config | todo | M1: docs/contracts/paths.md §5 |
| PATHS-013 | Paths | v1.8.1 legacy migration (TOML then config.json, BOM tolerated, support data) | reasonix-config | todo | M1: docs/contracts/paths.md §5 |
| PATHS-014 | Paths | MigrationResult notice format | reasonix-config | todo | M1: docs/contracts/paths.md §5 |
| PATHS-015 | Paths | v1.9.1 MCP backfill + one-time marker | reasonix-config | todo | M1: docs/contracts/paths.md §5 |
| PATHS-016 | Paths | State relocation adoption (.adopted aside, MergeOnce, no overwrite) | reasonix-state | todo | M1: docs/contracts/paths.md §5 |
| PATHS-017 | Paths | /migrate rescue semantics | reasonix-cli | todo | M1: docs/contracts/paths.md §5 |
| PATHS-018 | Paths | Credential path deny list (exact paths, ~/.ssh rules, snapshot) | reasonix-core | todo | M1: docs/contracts/paths.md §6 |
| PATHS-019 | Paths | Sensitive file names + dirs + ForbiddenReadPaths | reasonix-core | todo | M1: docs/contracts/paths.md §6 |
| PATHS-020 | Paths | Host secret path registration | reasonix-core | todo | M1: docs/contracts/paths.md §6 |
| PATHS-021 | Paths | Host-writable dirs per platform (hostwrite) | reasonix-core | todo | M1: docs/contracts/paths.md §7 |
| PATHS-022 | Paths | Subprocess env filtering (ProcessEnv, FilterEnv, toggles) | reasonix-core | todo | M1: docs/contracts/paths.md §8 |
| PATHS-023 | Paths | Credential env key registration (process-lifetime union) | reasonix-core | todo | M1: docs/contracts/paths.md §8 |
| PATHS-024 | Paths | REASONIX_AGENT_CHILD marker | reasonix-core | todo | M1: docs/contracts/paths.md §8 |
| PATHS-025 | Paths | Env redaction patterns + mask rules | reasonix-core | todo | M1: docs/contracts/paths.md §9 |
| PATHS-026 | Paths | CredentialKey algorithm | reasonix-core | todo | M1: docs/contracts/paths.md §9 |
| PATHS-027 | Paths | Endpoint projection (RedactEndpoint, RedactConfigValue, RedactArgs) | reasonix-core | todo | M1: docs/contracts/paths.md §9 |
| PATHS-028 | Paths | Diagnostic errors (identity kept, prose omitted) | reasonix-core | todo | M1: docs/contracts/paths.md §9 |
| PATHS-029 | Paths | Environment probes (default set, error taxonomy, 2s timeout) | reasonix-platform | todo | M1: docs/contracts/paths.md §10 |
| PATHS-030 | Paths | Probe caching (5min in-process, singleflight, 24h snapshot v2, fingerprint) | reasonix-platform | todo | M1: docs/contracts/paths.md §10 |
| PATHS-031 | Paths | FormatSection exact output | reasonix-platform | todo | M1: docs/contracts/paths.md §10 |
| PATHS-032 | Paths | Workspace VCS detection (jj/git/hg/svn probe order) | reasonix-core | todo | M1: docs/contracts/paths.md §10 |
| PATHS-033 | Paths | Language detection chain + normalization | reasonix-core | todo | M1: docs/contracts/paths.md §11 |
| PATHS-034 | Paths | Test isolation (IsolateUserState, testscope) | reasonix-testkit | todo | M1: docs/contracts/paths.md §12 |
| PATHS-035 | Paths | REASONIX_* env var inventory | reasonix-core | todo | M1: docs/contracts/paths.md §13 |
| PATHS-036 | Paths | Studio logs + kernel caches layout | reasonix-platform | todo | M1: docs/contracts/paths.md §14 |
