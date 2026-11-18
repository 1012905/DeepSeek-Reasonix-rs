# State / SQLite Contract — Frozen Inventory (M1)

Status: **FROZEN** from source on 2026-11-15. Behavior oracle: `reference/DeepSeek-Reasonix-studio` (read-only). This inventory freezes the SQLite/data-layer contract of the kernel: the disposable projection engine (`internal/state/projectiondb`), the history FTS catalog (`internal/state/historycatalog`), the usage catalog (`internal/state/usagecatalog`), the cc-switch import reader (`internal/contract/config/ccswitch.go`), and a survey of the authoritative (non-SQLite) state formats. Rust target crate: `reasonix-state` (cc-switch import: `reasonix-config`).

Oracle priority for conflicts: reference behavior > reference tests > reference source/docs > AGENTS.md > inference with ADR (AGENTS.md §11.3).

## 0. Scope and philosophy

- **Authoritative data is never in SQLite.** Session JSONL, event logs, metadata sidecars, `desktop-projects.json`, and the daily usage-stats JSONL files are authoritative; every SQLite database in the kernel is a *disposable projection* that can be deleted and rebuilt from them (`projectiondb/projectiondb.go` package doc: "Business data must remain authoritative outside the database: callers are expected to be able to discard and rebuild every database opened here."). A Rust rewrite must preserve this: no user data may live only in SQLite.
- **Driver.** Reference links `modernc.org/sqlite v1.61.1` (pure Go, `CGO_ENABLED=0`); go.mod confirmed. Non-test importers: `internal/state/projectiondb/projectiondb.go` and `internal/contract/config/ccswitch.go`. The Rust side must produce byte-compatible SQLite files (same schema, pragmas, FTS5 features); the engine choice (e.g. bundled SQLite via rusqlite) is an implementation ADR, but FTS5 with `contentless_delete` and `unicode61 remove_diacritics 2` is required for the history catalog.
- **Only two SQLite files exist in the Go source** (grep `\.sqlite`):
  - `<CacheDir>/history-search/v1.sqlite` — `historycatalog/types.go:110` DefaultPath().
  - `<CacheDir>/usage-catalog/v<N>.sqlite` — `usagecatalog/catalog.go:145`, N = SchemaVersion = **8**.
  - Divergence: `docs/CONFIG_PATHS.md:43,46` also lists `session-catalog/v3.sqlite` and `task-catalog/v1.sqlite`, which have **zero** Go references (docs-only). See §6.
- **Filesystem posture** (identical for both catalogs): cache dir under the OS cache directory or `$REASONIX_HOME/cache` (see docs/contracts/config.md §11 for resolution); DB dir `0700`; DB file + `-wal` + `-shm` `0600`; WAL journal; private permissions; short busy timeout (150 ms).
- **Failure posture**: remote or unavailable cache directory falls back to an in-memory projection; integrity or migration failure quarantines and rebuilds; a schema newer than this build opens degraded read-only. Failures never block startup/shutdown/turn completion; queries fall back to the authoritative files (docs/HISTORY_SEARCH_CATALOG.md, docs/USAGE_CATALOG.md).

## 1. projectiondb — the disposable projection engine

Source: `internal/state/projectiondb/projectiondb.go` (read in full, 17,012 bytes). Everything below cites it unless noted.

### 1.1 Types

- `Mode`: `disk` | `memory`. `State`: `ready` | `degraded`.
- `Status` JSON: `{state, mode, path?, revision uint64, indexed/total/pending/failed int64, lastError?, quarantinedPath?}`.
- `Migration{Version int, Apply func(ctx, *sql.Tx) error}`; `OpenOptions{Path, MemoryName, Migrations, InMemory, RequireDisk, MaxOpenConns, Now, SecureDelete, AutoVacuum}`; `Handle{DB, Status}`.
- `FutureSchemaError{Found, Supported}`: `"projection schema %d is newer than supported %d"`.
- `ErrIntegrityCheckFailed` = `"projection integrity check failed"`.
- `Inspection{Exists, Path, Schema, Integrity, Size, Error}`; `Recorded{Failed, Pending, LagBytes, CorruptLines int64, Failures []Failure{Path, Health, LastError}}`.

### 1.2 Open() — mode selection and failure triage

1. Blank/whitespace `Path` forces memory. `PathLooksRemote` (Windows UNC `\\\\` prefix; `/net/`, `/nfs/`, `/afs/` prefixes) forces memory.
2. Disk: `MkdirAll(dir, 0700)` + `Chmod 0700`. A filesystem-level remote check demotes to memory with `LastError = "projection cache is on a remote filesystem; using memory"`.
3. Disk-open failure triage:
   - `FutureSchemaError` → open degraded in-memory; the newer file is kept intact (never deleted, never quarantined).
   - Corruption (`isCorruptionError`, §1.5) → **quarantine** (rename `path` → `<path>.corrupt-<UnixMilli>`, plus `-wal`/`-shm`) and retry open; quarantine failure degrades to memory.
   - Any other error (busy / permission / IO) → degraded in-memory fallback; a healthy database is NEVER renamed.
4. `RequireDisk` disables the memory fallback so a failed temporary open reports the real disk error (used by Rebuild; errors: `"projection requires disk storage"`, `"projection cache is on a remote filesystem"`, `"create projection directory: %w"`).

### 1.3 open() — DSNs, connections, pragmas

- Memory DSN: `file:reasonix-<MemoryName>-<UnixNano>?mode=memory&cache=shared`. Memory forces `MaxOpenConns=1`.
- Disk DSN: `file:///` URI (Windows drive path becomes `/C:/...`) + `?_pragma=busy_timeout%28150%29&_pragma=foreign_keys%281%29&_pragma=journal_size_limit%284194304%29` — busy timeout 150 ms, foreign keys ON, journal size limit 4 MiB (`walSizeLimit = 4<<20`), applied per connection.
- Pool: disk default `MaxOpenConns=4`, `MaxIdleConns=min(maxOpen,2)`.
- Disk pragmas in order: `PRAGMA journal_mode=WAL`; `synchronous=NORMAL`; `foreign_keys=ON`; `busy_timeout=150` (all four required — a failure fails the open). Then:
  - `SecureDelete` → best-effort `PRAGMA secure_delete=ON`.
  - `AutoVacuum` → `PRAGMA auto_vacuum=INCREMENTAL` (failures ignored on already-initialized files).
- `PRAGMA integrity_check` must return exactly `ok`, else `ErrIntegrityCheckFailed: <result>`.
- `ApplyMigrations` (§1.4), then `chmod 0600` on the DB path, `-wal`, and `-shm`.

### 1.4 Migrations

`CREATE TABLE IF NOT EXISTS schema_migrations(version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL)`; current = `COALESCE(MAX(version),0)`. A schema newer than supported → `FutureSchemaError` (degraded read mode). Migrations must be **consecutive**: each must be exactly `current+1`, else `"projection migration gap after version %d"`. Each migration runs in a transaction: Begin → Apply → INSERT `schema_migrations(version, applied_at=now().UnixMilli())` → Commit; errors wrap `"apply projection migration %d: %w"`. `SupportedVersion(migrations)` = max version.

### 1.5 Corruption detection

`isCorruptionError`: `FutureSchemaError` → false; modernc sqlite error with code&0xff in {SQLITE_CORRUPT, SQLITE_NOTADB} → true; `errors.Is(ErrIntegrityCheckFailed)` → true; everything else (busy/permission/IO) → false. Quarantine renames DB + `-wal` + `-shm` to `<path>.corrupt-<UnixMilli>`; returns the quarantined path or "" on rename failure.

### 1.6 Read-only open, Inspect

`OpenReadOnly(ctx, path)` never creates, migrates, repairs, or quarantines: it tries the disk DSN + `&mode=ro`, then + `&mode=ro&immutable=1` (immutable skips the live WAL — fallback for media where `-shm` cannot be created); `MaxOpenConns(1)`. `Inspect(ctx, path)`: stat → read-only open → integrity_check + MAX(version) → `Inspection` (deliberately read-only diagnostics).

### 1.7 Rebuild() — validated swap

1. Requires a disk path (`"projection rebuild requires a disk path"`); `MkdirAll 0700`.
2. `filelock.Acquire(ctx, path+".rebuild.lock")` (error `"lock projection rebuild: %w"`).
3. Temporary DB `<path>.rebuild-<UnixNano>` opened with `RequireDisk=true`; populate; integrity_check must be ok; `PRAGMA wal_checkpoint(TRUNCATE)`; Close.
4. Swap: backup `<path>.replaced-<UnixNano>`; rename old → backup (+`-wal`/`-shm`; failure → `"projection database is busy: %w"`); rename temporary → path (`"install projection replacement: %w"`); chmod 0600; remove backup; rollback restores the old file on failure.
5. Authoritative files are never touched.

## 2. historycatalog — the FTS search projection (v1.sqlite)

Sources: `internal/state/historycatalog/{schema,types,catalog,incremental,root_scan,documents,work,rebuild,inspect,catalog_adopt}.go` (all read in full) + `docs/HISTORY_SEARCH_CATALOG.md`.

### 2.1 Constants, path, options

- `SchemaVersion = 1`; `TokenizerVersion = 4` ("user turns are indexed without the host's prepended or appended blocks"); `DefaultLimit = 50`; `MaxLimit = 200`.
- `DefaultPath() = <CacheDir>/history-search/v1.sqlite`; empty CacheDir → memory.
- Options: `{Work *backgroundwork.Scheduler, Path, InMemory, QueueCapacity (default 1024), MissingGrace (default 30s), ReconcileInterval (default 5m), Now, OnRevision func(Status, []string, string)}`. Open uses projectiondb options `{MemoryName: "history-search", MaxOpenConns: 4, SecureDelete: true, AutoVacuum: true}`.
- Root: `{Path, Source, Scope, WorkspaceRoot, Subagents, Archive}`; RegisterRoot cleans the path and forces `scope != "project" → "global"` with empty WorkspaceRoot; the first registration of a root bumps Pending; `rootCh` is a non-blocking send (capacity 64), overflow marks the root dirty.
- Status JSON mirrors projectiondb plus the four counts (§2.7). Nil-catalog `Status()` returns `{state:"degraded", mode:"memory", lastError:"history catalog unavailable"}`.

### 2.2 Schema (migration v1, exact DDL — historycatalog/schema.go)

```sql
CREATE TABLE history_state(
    id INTEGER PRIMARY KEY CHECK(id=1),
    revision INTEGER NOT NULL DEFAULT 0,
    tokenizer_version INTEGER NOT NULL
);
INSERT INTO history_state(id,revision,tokenizer_version) VALUES(1,0,1);

CREATE TABLE history_roots(
    path TEXT PRIMARY KEY,
    source TEXT NOT NULL,
    scope TEXT NOT NULL,
    workspace_root TEXT NOT NULL DEFAULT '',
    signature TEXT NOT NULL DEFAULT '',
    scan_generation INTEGER NOT NULL DEFAULT 0,
    scan_cursor TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL DEFAULT 'pending',
    error TEXT NOT NULL DEFAULT '',
    indexed INTEGER NOT NULL DEFAULT 0,
    total INTEGER NOT NULL DEFAULT 0,
    completed_at INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE history_sources(
    path TEXT PRIMARY KEY,
    root TEXT NOT NULL,
    source TEXT NOT NULL,
    scope TEXT NOT NULL,
    workspace_root TEXT NOT NULL DEFAULT '',
    content_revision INTEGER NOT NULL DEFAULT 0,
    content_digest TEXT NOT NULL DEFAULT '',
    content_fingerprint TEXT NOT NULL DEFAULT '',
    meta_fingerprint TEXT NOT NULL DEFAULT '',
    message_count INTEGER NOT NULL DEFAULT 0,
    indexed_message_count INTEGER NOT NULL DEFAULT 0,
    custom_title TEXT NOT NULL DEFAULT '',
    topic_id TEXT NOT NULL DEFAULT '',
    topic_title TEXT NOT NULL DEFAULT '',
    preview TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL DEFAULT 0,
    last_activity_at INTEGER NOT NULL DEFAULT 0,
    health TEXT NOT NULL DEFAULT 'ok',
    missing_since INTEGER NOT NULL DEFAULT 0,
    seen_generation INTEGER NOT NULL DEFAULT 0,
    last_error TEXT NOT NULL DEFAULT ''
);

CREATE TABLE history_documents(
    id INTEGER PRIMARY KEY,
    source_path TEXT NOT NULL REFERENCES history_sources(path) ON DELETE CASCADE,
    message_index INTEGER NOT NULL,
    part_index INTEGER NOT NULL DEFAULT 0,
    role TEXT NOT NULL,
    kind TEXT NOT NULL,
    tool_name TEXT NOT NULL DEFAULT '',
    token_count INTEGER NOT NULL DEFAULT 0,
    UNIQUE(source_path,message_index,kind,part_index)
);

CREATE VIRTUAL TABLE history_fts USING fts5(
    terms,
    content='',
    contentless_delete=1,
    tokenize='unicode61 remove_diacritics 2'
);

CREATE INDEX idx_history_sources_scope_activity
ON history_sources(scope,workspace_root,last_activity_at DESC,path ASC);
CREATE INDEX idx_history_documents_source
ON history_documents(source_path,message_index,part_index,id);
```

### 2.3 Indexing pipeline

- A single worker goroutine: drains pending queue entries first, then dirty roots, then selects on {ctx.Done, reconcile ticker (marks all roots dirty), flushCh (drains + acks), queue (indexPath), rootCh (reconcileRoot)}.
- Queue coalescing by path: an existing queued entry merges (appendFrom -1 wins, else min). Queue full → drop from map + mark that root dirty + return false (EnqueuePersist's Rewrite maps appendFrom to -1; a persist event's appendFrom is the display-index position).
- `indexPathOnce`: non-archive + not `sessionstore.IsVisibleSession(path)` → Purge. Content fingerprint = `"<size>:<mtimeUnixNano>"` for the transcript and its branch-meta sidecar; `sessionstore.SessionContentIdentity` yields digest + revision. Skip when `sourceProjectionUnchanged` (§2.4). Fast path `tryAppendPath` (§2.4). Otherwise: load session context, derive documents (§2.5), mark the row `health='stale'` *first*, then in one transaction DELETE the source's FTS rows (`rowid IN (SELECT id FROM history_documents WHERE source_path=?)`) and documents, UPSERT `history_sources` (`health='ok', missing_since=0`), INSERT each document + `history_fts(rowid, terms)`, bump revision, publish reason `"source-indexed"`.
- Revision: `history_state.revision` += 1 per commit (`bump`); every publish refreshes status counts and calls `OnRevision`.
- `RecordReadFailure` (a reader could not open a transcript): UPDATE `health='corrupt', last_error=`cause **only where health='ok'**; bump; publish `"source-unreadable"`; re-enqueue via EnqueueExisting. Read errors on non-ok rows do not requeue each search.
- Tokenizer version guard: on open, `tokenizer_version != 4` → transaction wipes `history_fts`, `history_documents`, `history_sources`, `history_roots`, sets `tokenizer_version=4`, bumps revision (a full reindex follows). A schema bump of TokenizerVersion therefore invalidates the whole projection, never mixes tokenizers.

### 2.4 Incremental correctness

- `sourceProjectionUnchanged`: last query succeeded AND `oldHealth == "ok"` AND content + meta fingerprints match AND (digest empty OR equal). A non-ok health never licenses a size/mtime skip.
- `tryAppendPath` (append fast path) requires `appendFrom == oldMessageCount && revision == oldRevision+1`; validates `store.SessionDisplayIndex(path)` (`RevisionKnown && index.Revision == revision && index.ContentDigest == digest && index.MessageCount >= appendFrom`) and re-checks the tail via `LoadSessionDisplayMessageRange`. One transaction: UPDATE `history_sources` ... WHERE `path=? AND content_revision=? AND indexed_message_count=?` (RowsAffected != 1 → rollback → full reindex), then INSERT the tail documents at `message_index = appendFrom + doc.message`; publish `"source-appended"`.
- `rootProjectionHealthy`: no row for the root with `health<>'ok' OR missing_since<>0`.

### 2.5 Documents and tokenization (documents.go)

Per message: user → `UserSaidText` kind `user_text`; assistant → `assistant_text` plus one `tool_input` per tool call (tool name; terms = name + " " + arguments); tool role → always `tool_output` (terms = name + " " + content) plus `tool_error` when the lowercased content starts with `"error:"`/`"blocked:"` or contains `"permission denied"`. Terms = `retrieval.Tokens(trim(text))` (FTS vocabulary: English lowercase/code-symbol, CJK overlapping bigrams — docs/HISTORY_SEARCH_CATALOG.md); empty term lists are skipped. `documentDigest` = sha256 over each document's terms joined with NUL.

### 2.6 Root scan (root_scan.go, catalog_adopt.go)

- `reconcileRoot`: scheduler(Rebuild) → `startRootScan` → for each path {indexPending; indexPath(generation)} → scheduler(Rebuild) → `finishRootScan`.
- `startRootScan`: `os.ReadDir`; keep non-dir entries with `store.IsSessionTranscriptName` and not `store.IsSubagentTranscriptName` and (root.Archive OR `IsVisibleSession`); sort; signature = sha256 over each path + branch-meta stat ("size NUL mtime" or "missing"). **Skip** the whole scan when the previous state is `'ready'` AND signature unchanged AND non-empty AND `indexed == total` AND `rootProjectionHealthy`. Otherwise UPSERT `history_roots` RETURNING `scan_generation` (incremented, state 'scanning').
- `measureUnseen` (outside the write transaction — a slow stat must not hold the write lock): sources with `seen_generation <> generation` split into live (stat succeeded, or non-archive visible) and absent (stat returned ErrNotExist). "A path that cannot be stat'd is not evidence of absence."
- `adoptLiveSources` marks measured-live rows seen; `sweepAbsent` sets `missing_since` (first time only) + `health='missing'` for stat-confirmed-gone rows; a row another process indexed during the scan is never swept.
- `finishRootScan`: applyScanResult; DELETE FTS rows, documents, sources for the root where `seen_generation<>generation AND missing_since>0 AND missing_since <= now-MissingGrace` (30 s default); UPDATE root `state='ready', signature, indexed, completed_at, scan_cursor=''`; bump; publish `"reconcile"`.

### 2.7 Search (catalog.go)

- `SearchRequest{Query, Scope, WorkspaceRoot, Kinds, ToolName, Limit, Roots, After *SearchCursor}`; `SearchCursor{Rank, SessionPath, MessageIndex, PartIndex, RowID}` — stable keyset over BM25 rank + deterministic tie-breaks (lower BM25 first).
- Terms: `retrieval.QueryTerms`; MATCH expression = terms quoted (embedded `"` doubled) joined with ` OR`. Base predicate: `history_fts MATCH ? AND s.health='ok' AND s.missing_since=0`, plus optional `scope='project' AND workspace_root=?`, `d.kind IN (...)`, `d.tool_name=?`, `s.root IN (...cleaned...)`. Join: FTS → `history_documents d ON d.id = history_fts.rowid` → `history_sources s ON s.path = d.source_path`. ORDER BY `bm25(history_fts), d.source_path, d.message_index, d.part_index, d.id` LIMIT (default 50, max 200).
- Keyset pagination: `WITH ranked AS MATERIALIZED (base query)` + a 5-key tuple comparison (rank, source_path, message_index, part_index, id).
- Post-filter: `catalogPathWithin` via filepath.Rel containment against requested roots. Score = `-Rank` when Rank < 0 else `1/(1+Rank)`. `Partial = Pending > 0`; the caller may merge the exact reader's results. Revision is snapshotted.
- Purge: transaction DELETEs the source's FTS + documents + `history_sources`, bumps, commits, then `PRAGMA wal_checkpoint(TRUNCATE)` + `PRAGMA incremental_vacuum(64)`; publish `"purge"`.

### 2.8 Rebuild, Inspect, status counts (rebuild.go, inspect.go)

- `Rebuild(ctx, opts, roots)`: memory → open + ReconcileRoot per root; disk → `projectiondb.Rebuild` with `{MemoryName: "history-search-rebuild", SecureDelete: true, AutoVacuum: true}`, populating by ReconcileRoot per root and recording the tokenizer version; then re-Open to report the published status. `rebuildOptions` forces Path default / memory fallback, disables `OnRevision`.
- Status counts (one read shared by Status and Inspect): Indexed = sources with `health='ok'`; Total = `SUM(history_roots.total)`; Pending = roots `state<>'ready'`; Failed = sources `health='corrupt'`.
- `Inspect(ctx, path)`: read-only open; returns Recorded{Failed, Pending} + up to `reportedFailures = 5` corrupt sources ordered by path. `SupportedSchema()` = projectiondb.SupportedVersion(migrations()).
- Flush/Close: Flush waits for a worker ack via a queue sentinel; Close is once-only, cancels, waits, closes the DB, honors ctx.

### 2.9 Budgets and degraded modes (docs/HISTORY_SEARCH_CATALOG.md — behavioral contract)

- FTS stores normalized tokens, never full text; snippets/around-context load from the authoritative transcript only after SQLite selects candidates.
- Replay budgets (encoded JSON bytes): whole encoded log 512 MiB; one encoded record 64 MiB; aggregate retained payloads 64 MiB; legacy 1.x DAG metadata 64 MiB. Exceeding → `SessionReplayLimitError`; sources unchanged; never falls back to an older checkpoint.
- Non-blocking hints after commit + after file locks released; path-coalesced; worker replays one source at a time; per-source checkpoints; replay failures isolated.
- Legacy summary cache format 2 verifies manifest identity + log signature. `/tree?progressive=1` returns `listingState/listingRevision/countsComplete/pendingMetadata/failedMetadata`; `retryMetadata=1` retries failed metadata once.
- Diagnostics never print queries, tokens, snippets, messages, tool args, or provider content.
- Commands: `reasonix doctor catalogs [--json]`, `reasonix catalogs reindex history [--dir PATH ...] [--json]`.

## 3. usagecatalog — the usage rollup projection (v8.sqlite)

Sources: `internal/state/usagecatalog/{catalog,costview,roots,rebuild,inspect,unpriced}.go` (all read in full) + `docs/USAGE_CATALOG.md`, `docs/BILLING.md`.

### 3.1 Version, path, retirement

- `SchemaVersion = 8` ("Version 8 adds the reason a row was unpriced; opening it reindexes the daily files, so an older projection's rows regain their reason from the source"). A version bump **retires** the old file rather than migrating it: every column is replayable from the JSONL.
- `DefaultPath() = <CacheDir>/usage-catalog/v8.sqlite` (v%d of SchemaVersion); empty CacheDir → memory.
- `retireOlderProjections` (on every disk Open): deletes every `v*.sqlite` in the directory except the current name and its `-wal`/`-shm` siblings (`strings.HasPrefix(name, keep+"-")`). Docs/BILLING.md: "A build older than this one uses its own v5.sqlite and, on start, removes the files of versions it does not open. Alternating between builds therefore rebuilds the cache each time."
- Open options: `{MemoryName: "usage-catalog", MaxOpenConns: 4}`; queue capacity 1024.

### 3.2 Schema (migration v1, exact DDL — usagecatalog/catalog.go)

```sql
CREATE TABLE usage_state(id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL DEFAULT 0);
INSERT INTO usage_state(id,revision) VALUES(1,0);
CREATE TABLE usage_files(
 path TEXT PRIMARY KEY,day TEXT NOT NULL,size INTEGER NOT NULL DEFAULT 0,mtime_ns INTEGER NOT NULL DEFAULT 0,
 indexed_offset INTEGER NOT NULL DEFAULT 0,state TEXT NOT NULL DEFAULT 'pending',error TEXT NOT NULL DEFAULT '',
 corrupt_lines INTEGER NOT NULL DEFAULT 0,completed_at INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE usage_records(
 file_path TEXT NOT NULL,byte_offset INTEGER NOT NULL,byte_length INTEGER NOT NULL,line_hash TEXT NOT NULL,
 day TEXT NOT NULL,source TEXT NOT NULL,model_ref TEXT NOT NULL,provider TEXT NOT NULL,
 prompt INTEGER NOT NULL,completion INTEGER NOT NULL,reasoning INTEGER NOT NULL,cache_hit INTEGER NOT NULL,
 cache_miss INTEGER NOT NULL,total INTEGER NOT NULL,requests INTEGER NOT NULL,turns INTEGER NOT NULL,
 cost INTEGER NOT NULL DEFAULT 0,cost_currency TEXT NOT NULL DEFAULT '',cost_estimated INTEGER NOT NULL DEFAULT 0,
 cost_incomplete INTEGER NOT NULL DEFAULT 0,incomplete_reason TEXT NOT NULL DEFAULT '',
 valuation_cny INTEGER,valuation_usd INTEGER,billing_mode TEXT NOT NULL DEFAULT '',
 PRIMARY KEY(file_path,byte_offset)
);
CREATE TABLE usage_rollups(
 day TEXT NOT NULL,source TEXT NOT NULL,model_ref TEXT NOT NULL,provider TEXT NOT NULL,
 prompt INTEGER NOT NULL,completion INTEGER NOT NULL,reasoning INTEGER NOT NULL,cache_hit INTEGER NOT NULL,
 cache_miss INTEGER NOT NULL,total INTEGER NOT NULL,requests INTEGER NOT NULL,turns INTEGER NOT NULL,
 cost_incomplete INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(day,source,model_ref)
);
CREATE TABLE usage_cost(
 day TEXT NOT NULL,source TEXT NOT NULL,model_ref TEXT NOT NULL,view TEXT NOT NULL,
 currency TEXT NOT NULL,mode TEXT NOT NULL DEFAULT '',cost INTEGER NOT NULL,estimated INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(day,source,model_ref,view,currency,mode)
);
CREATE INDEX idx_usage_rollups_range ON usage_rollups(day,source,model_ref);
CREATE INDEX idx_usage_records_unpriced ON usage_records(day,source,cost_incomplete,incomplete_reason);
```

### 3.3 Authoritative daily JSONL line shape (rawRecord)

The daily usage-stats files (`<StatsDir>/YYYY-MM-DD.jsonl`, StatsDir from `contract/config/paths.go:449`) are authoritative; each line:

```
{"ts":<RFC3339>,"model":<model_ref "provider/model">,"source":<source>,"prompt":n,"completion":n,
 "reasoning":n,"cache_hit":n,"cache_miss":n,"total":n,"requests":n,"turn":bool,
 "cost_amount":"<pricing.Amount fixed-point string>","cost_currency":"CNY","cost_estimated":bool,
 "cost_complete":bool|null,"incomplete_reason":"...","valuation_cny":"...","valuation_usd":"...",
 "billing_mode":"payg|subscription_equivalent"}
```

Provider is derived: `model[:IndexByte(model,'/')]{PH} else `"default"`. Turns = 1 when `turn` true. A line with `total>0 && requests<=0` is stored with requests=1. Unparseable amounts are dropped, never guessed ("a missing total beats a wrong one"). A turn marker counts no usage, so it is never an unpriced call.

### 3.4 Append receipts and idempotency

- `AppendReceipt{Path, Day, Offset, Length, LineHash}`; the stats recorder emits a receipt only after a **successful JSONL append** (non-blocking enqueue; queue overflow records a dirty file → ReconcileFile).
- `applyReceipt`: if a record already exists at (file_path, byte_offset): same line_hash → no-op; different hash → ReconcileFile (byte-offset reconciliation). Otherwise INSERT record (INSERT OR IGNORE), project rollups/costs (§3.5), UPSERT `usage_files` (indexed_offset advanced, state 'ready', mtime recorded only when the receipt's offset is the file's frontier), bump revision.
- Cross-process safe: duplicates cannot double-count because (file_path, byte_offset) is the primary key and the line hash gates replays.

### 3.5 Rollup and cost projection

- `projectRecord`: `usage_rollups` UPSERT adds the row's tokens/requests/turns/cost_incomplete under (day, source, model_ref). Cost rows are written per cost view (§3.7) only when the line was priced: `usage_cost` UPSERT adds `cost`, `estimated=MAX(estimated, excluded.estimated)`. "A currency means the line was priced; its amount may genuinely be zero... Only a line with no currency carries no price at all."
- `rederiveDays` (reindex path): DELETE the day's rollups + costs, then rebuild rollups by GROUP BY over `usage_records` (`MAX(provider)`, SUMs), and per view via `viewCostSelect`: `INSERT INTO usage_cost ... SELECT ... SUM(COALESCE(valuation_<view>, cost)) , MAX(cost_estimated) FROM usage_records WHERE day=? AND cost_currency<>'' GROUP BY day,source,model_ref,<currency CASE>,<mode CASE>`; currency = view currency when the vendor valuation exists, else the billed currency; mode = 'estimate' iff `billing_mode='subscription_equivalent'`.

### 3.6 Reconciliation (roots.go, catalog.go)

- `ReconcileDir(ctx, dir)`: the projection has exactly **one** authoritative stats root; indexes every `*.jsonl` file in it (day = basename minus suffix) and **forgets every indexed file outside it** — a relocated root would otherwise "sum every shared day twice on every launch". Forgetting happens first: interrupted passes read as unindexed (Ready falls back to JSONL) rather than complete-with-stale-rows.
- `forgetOutsideRoot`: scans `usage_files` (one row per file), deletes stale `usage_records` + `usage_files` by primary key, rederives affected days. Path comparisons in `absoluteClean` form; `underRoot` via filepath.Rel containment (a vanished root contributes nothing).
- `ReconcileFile`: full-file replay — DELETE the file's records, stream lines (bufio), skip blank lines, count unparseable JSON lines as `corrupt_lines` (not stored as records), insert parseable ones with sha256 line hashes, rederive `recordDays` (every day the file's records were filed under plus the day now), UPSERT `usage_files` ready with corrupt_lines and mtime.
- `Ready(ctx, dir, days)`: per day, the file must exist (a vanished-but-indexed day is NOT ready — stale rows force JSONL fallback; a never-indexed day reads as an empty, complete day) and `usage_files` must show `state='ready' && size == info.Size() && indexed_offset == info.Size() && mtime_ns == info.ModTime().UnixNano()` — same-size in-place rewrites invalidate Ready so callers fall back to the authoritative JSONL until the rescan catches up.

### 3.7 Cost views and unpriced vocabulary (costview.go, unpriced.go)

- `costViews = ["", "CNY", "USD"]` — "" is each row's billed currency; CNY/USD are vendor-table valuations. Every view partitions the same rows; no row is counted twice within one. `CostView(currency)` normalizes and falls back to "".
- `ModeEstimate = "estimate"`; `CostMode(billingMode)` = "estimate" iff billing_mode is `subscription_equivalent`, else "" (billed). Estimates are kept apart from billed cost and never added to it (docs/BILLING.md).
- `CostIncomplete(costComplete *bool, total, amount)`: true when the quote said incomplete OR `total > 0 && amount is blank`. `ReasonNoPrice = "no_price"`; `UnpricedReason` returns the line's recorded reason, or `no_price` only when there is no cost fact; an unpriced row with no recorded reason stays unpriced with an empty reason. A total folding such rows is a floor, not a bill.
- `UnpricedInRange`: counts `cost_incomplete=1` records in [fromDay, toDay] (optional source filter), groups by `incomplete_reason`, returns sorted distinct non-empty reasons.
- Entry cost fields: `Cost int64` is fixed-point (pricing.Amount), never a float; `Valuations map[string]int64` for CNY/USD.

### 3.8 Query, status, rebuild, inspect

- `Query(ctx, fromDay, toDay, source, view)`: reads `usage_rollups` in range (source optional; `"all"` = all), ORDER BY day, source, model_ref; `attachCosts` folds `usage_cost` rows for the view via a second query (not a join — a join would multiply token columns by currency count).
- Status JSON: `{state, mode, path?, revision, indexedFiles, lagBytes, corruptLines, fallbacks, lastError?}`; counts (one read shared by Status and Inspect): files = COUNT(usage_files), lag = SUM(MAX(size-indexed_offset,0)), corrupt = SUM(corrupt_lines). `NoteFallback()` counts JSONL fallbacks.
- `Rebuild(ctx, path, statsDir)`: memory → open + ReconcileDir; disk → `projectiondb.Rebuild` with `{MemoryName: "usage-catalog-rebuild"}` populating via ReconcileDir, then re-Open for the published status. docs/BILLING.md perf datum: a rebuild read ~18,000 records (5.4 MB, 30 daily files) in 0.7 s (~26k records/s).
- `Inspect`: read-only; Recorded{LagBytes, CorruptLines}. Commands: `reasonix doctor catalogs [--json]`, `reasonix catalogs reindex usage [--json]`.

## 4. cc-switch import (external SQLite, read-only)

Source: `internal/contract/config/ccswitch.go` (read in full). Rust target: `reasonix-config`. This is not a Reasonix-owned database: the kernel *reads* a third-party tool's database to import MCP servers.

- Location: `~/.cc-switch/cc-switch.db` (`ccSwitchDir = ".cc-switch"` under `os.UserHomeDir()`); skipped entirely when `IsolatedHomeDir()` is set. Legacy fallback files: `config.json`, `config.json.migrated`, `config.json.bak`.
- Read-only DSN: file URI (`\` → `/`; drive path prefixed with `/`) + `?mode=ro&_pragma=busy_timeout%28150%29`; `MaxOpenConns(1)`. Never writes the external DB.
- Schema detection: `SELECT COUNT(*) FROM pragma_table_info('mcp_servers') WHERE name='enabled_reasonix'` → 0 (pre-v16): `SELECT id,name,server_config FROM mcp_servers WHERE enabled_codex=1 ORDER BY name,id`; 1: same with `enabled_reasonix=1`; any other count → error "unexpected enabled_reasonix column count %d". All errors prefix `"cc-switch import: ..."`; a DB missing both shapes and no legacy entries → `"cc-switch import: no Reasonix-enabled MCP servers found in %s"`.
- Row mapping: `server_config` JSON → mcpServerSpec; name = trimmed id, else row name. Legacy JSON: `mcp.servers{key}=`{id,name,server,apps{codex,reasonix}}`; enabled = apps.reasonix when present else apps.codex; name = server.name, else id, else key; sorted keys.
- Import: upsert into user-global config under `LockUserConfigEdits` with `Source=MCPSourceUserConfig`; counts total/added/updated.
- Candidate classification (`classifyMCPImportCandidate`, recommended=false reasons): "unsupported sse" (type sse), "browser/heavy" (name contains chrome-devtools), "@latest" (npx/uvx args), "auth/env" (headers or env; still recommended for context7/exa/fetch names), "personal" (flomo/dida/ynote/youdao); "recommended" prefixes the reasons of a recommended candidate.

## 5. Authoritative state formats (survey)

- Authoritative session data (docs/CONFIG_PATHS.md, quoted in §0): "Session JSONL, event logs, metadata sidecars, and desktop-projects.json are the authoritative session data, and listing or opening a conversation reads nothing else." Session JSONL lives under `REASONIX_STATE_HOME`/sessions (or the Reasonix home); ACP transcripts are `<dir>/id.jsonl` (see docs/contracts/acp.md §10.5).
- Daily usage stats JSONL: `<StatsDir>/YYYY-MM-DD.jsonl`; line shape §3.3 (contract/config/paths.go:449 StatsDir).
- `internal/state` subpackage inventory (survey by walk + names; roles inferred where marked):
  - `projectiondb` — disposable SQLite engine (§1, read in full).
  - `historycatalog` — FTS5 search projection (§2, read in full).
  - `usagecatalog` — usage rollup projection (§3, read in full).
  - `sessionstore` — session JSONL store: save pipeline (save.go 76K), migrations (migrate.go), event vocabulary (session_events.go 27K), branching (branch.go), leases (session_lease.go), recovery GC (recovery_gc.go), display index, DAG read, v4 import (inferred from names).
  - `checkpoint` — checkpoint transactions + file revert (inferred). `sessioninbox` — durable inbox store + manifests + idempotency (inferred). `workspacelease` — workspace claim/lock/yield (inferred). `memory` — memory store v1/v2 + auto-recall (inferred). `stats` — daily JSONL recorder + usage catalog wiring. `history` — in-memory exact history reader (the non-SQLite fallback). `trajectory`, `execjournal`, `sessiontemp`, `instruction`, `trustedstate`, `peermail`, `mutation`, `sessionv4`, `schedule`, `workgroup`, `agentpreset`, `execgraph`, `usagereport`, `workspacelist`, `storage` (volume move/preflight), `migration` (desktop registry, relocated state) — roles inferred from names.
- **Follow-up gate (M2 entry):** before implementing `reasonix-state` read/write paths, freeze `docs/contracts/session-format.md`: the session JSONL event vocabulary (sessionstore/session_events.go), save pipeline ordering, branch/meta sidecars, display index, leases, checkpoint format, memory v2, inbox manifest formats. This document freezes the SQLite layer; the session-format contract is a separate, later freeze.

## 6. Divergences (docs vs source) and open decisions

1. **Catalog inventory mismatch.** docs/CONFIG_PATHS.md:43–46 documents four disposable catalogs; the Go source has exactly two (history-search/v1.sqlite, usage-catalog/v8.sqlite). `session-catalog/v3.sqlite` and `task-catalog/v1.sqlite` have **zero** references anywhere in the repository (repo-wide grep) — stale documentation. Rust rewrite: do NOT create them; behavior parity is the two real catalogs.
2. **usage-catalog version drift.** docs/USAGE_CATALOG.md and docs/CONFIG_PATHS.md say `v1.sqlite`; docs/BILLING.md says `v6.sqlite`; the source says **v8** (SchemaVersion=8). The path is derived from the constant, so the source wins; Rust must derive the filename from its schema version constant identically (`v%d.sqlite`).
3. **Rust engine choice (open ADR, not a behavior break).** Reference uses pure-Go modernc.org/sqlite (CGO_ENABLED=0). The Rust side needs a static-linkable SQLite with FTS5 (`contentless_delete`, `unicode61 remove_diacritics 2`), WAL, and the same on-disk format; rusqlite with bundled SQLite is the likely choice — decision + security review in an ADR before M2 state work. Corruption-classification (SQLITE_CORRUPT/SQLITE_NOTADB) must map to equivalent rusqlite/sqlite3 error codes so quarantine-vs-degrade triage keeps working.
4. **modernc-specific DSN pragmas.** The reference encodes per-connection pragmas in the DSN (`_pragma=busy_timeout(150)` etc.). In Rust these become explicit `PRAGMA` statements per connection — behavior must be identical (busy_timeout 150 ms, foreign_keys ON, journal_size_limit 4 MiB).
5. **Disposable means disposable.** Any Rust implementation detail that makes a projection non-rebuildable from authoritative files is a contract break. The parity tests must include: delete both catalogs → all functionality still correct (slower paths).

## 7. Extraction provenance

Read in full: projectiondb/projectiondb.go; historycatalog/{schema,types,catalog,incremental,root_scan,documents,work,rebuild,inspect,catalog_adopt}.go; usagecatalog/{catalog,costview,roots,rebuild,inspect,unpriced}.go; contract/config/ccswitch.go; docs/{CONFIG_PATHS,USAGE_CATALOG,HISTORY_SEARCH_CATALOG,BILLING}.md. Survey: directory walk of internal/state (270 product .go + 299 test files, sizes recorded), repo-wide greps for `.sqlite`, `modernc.org/sqlite`, `session-catalog`/`task-catalog` (`reference/DeepSeek-Reasonix-studio`, read-only).

## 8. Proposed ledger rows

| ID | Area | Reference | Rust | Status | Evidence |
|---|---|---|---|---|---|
| STATE-001 | State | disposable-projection philosophy; authoritative data outside DB | reasonix-state | todo | M1: docs/contracts/state.md §0 |
| STATE-002 | State | SQLite driver surface + file locations | reasonix-state | todo | M1: docs/contracts/state.md §0 |
| STATE-003 | State | projectiondb open modes, DSNs, pragmas, permissions | reasonix-state | todo | M1: docs/contracts/state.md §1.2-§1.3 |
| STATE-004 | State | projectiondb migrations, gap error, FutureSchema degraded read | reasonix-state | todo | M1: docs/contracts/state.md §1.4 |
| STATE-005 | State | corruption triage + quarantine rename | reasonix-state | todo | M1: docs/contracts/state.md §1.2, §1.5 |
| STATE-006 | State | OpenReadOnly + Inspect (no repair) | reasonix-state | todo | M1: docs/contracts/state.md §1.6 |
| STATE-007 | State | projectiondb Rebuild: lock, temp, integrity, swap, rollback | reasonix-state | todo | M1: docs/contracts/state.md §1.7 |
| STATE-008 | State | historycatalog schema v1 DDL | reasonix-state | todo | M1: docs/contracts/state.md §2.2 |
| STATE-009 | State | tokenizer version 4 wipe on open | reasonix-state | todo | M1: docs/contracts/state.md §2.3 |
| STATE-010 | State | indexing pipeline: queue coalescing, worker, revision bump | reasonix-state | todo | M1: docs/contracts/state.md §2.3 |
| STATE-011 | State | append fast path (tryAppendPath) + fingerprint skip | reasonix-state | todo | M1: docs/contracts/state.md §2.4 |
| STATE-012 | State | root scan: signature skip, generations, adopt/sweep, grace | reasonix-state | todo | M1: docs/contracts/state.md §2.6 |
| STATE-013 | State | document kinds + tokenization + digests | reasonix-state | todo | M1: docs/contracts/state.md §2.5 |
| STATE-014 | State | search: BM25 ordering, keyset pagination, filters, score | reasonix-state | todo | M1: docs/contracts/state.md §2.7 |
| STATE-015 | State | purge + record-read-failure + publish reasons | reasonix-state | todo | M1: docs/contracts/state.md §2.3 |
| STATE-016 | State | historycatalog rebuild/inspect/status counts | reasonix-state | todo | M1: docs/contracts/state.md §2.8 |
| STATE-017 | State | replay budgets + degraded modes + diagnostics redaction | reasonix-state | todo | M1: docs/contracts/state.md §2.9 |
| STATE-018 | State | usagecatalog schema v8 + older-version retirement | reasonix-state | todo | M1: docs/contracts/state.md §3.1-§3.2 |
| STATE-019 | State | daily stats JSONL line shape (rawRecord) | reasonix-state | todo | M1: docs/contracts/state.md §3.3 |
| STATE-020 | State | append receipts: idempotency, line hash, offset frontier | reasonix-state | todo | M1: docs/contracts/state.md §3.4 |
| STATE-021 | State | rollup/cost projection + rederiveDays | reasonix-state | todo | M1: docs/contracts/state.md §3.5 |
| STATE-022 | State | ReconcileDir/File + forgetOutsideRoot + underRoot | reasonix-state | todo | M1: docs/contracts/state.md §3.6 |
| STATE-023 | State | Ready semantics (vanish, same-size rewrite, frontier) | reasonix-state | todo | M1: docs/contracts/state.md §3.6 |
| STATE-024 | State | cost views, estimate mode, unpriced vocabulary | reasonix-state | todo | M1: docs/contracts/state.md §3.7 |
| STATE-025 | State | Query/attachCosts/UnpricedInRange | reasonix-state | todo | M1: docs/contracts/state.md §3.8 |
| STATE-026 | State | usagecatalog status/rebuild/inspect + perf datum | reasonix-state | todo | M1: docs/contracts/state.md §3.8 |
| STATE-027 | State | cc-switch import reader (external DB, read-only) | reasonix-config | todo | M1: docs/contracts/state.md §4 |
| STATE-028 | State | authoritative state formats survey | reasonix-state | todo | M1: docs/contracts/state.md §5 |
| STATE-029 | State | session-format contract follow-up gate (M2) | reasonix-state | todo | M1: docs/contracts/state.md §5 |
| STATE-030 | State | divergences: catalog inventory, version drift, engine ADR | reasonix-state | todo | M1: docs/contracts/state.md §6 |
