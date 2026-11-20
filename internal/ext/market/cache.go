package market

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"net/url"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	"reasonix/internal/base/fileutil"
)

// Why a listing came from disk instead of the registry.
const (
	CacheCauseUnreachable = "unreachable"
	CacheCauseBadResponse = "bad_response"
)

const (
	cacheEntryVersion = 1
	cacheMaxEntries   = 200
	cacheMaxBytes     = 8 << 20
	cacheMaxAge       = 7 * 24 * time.Hour
	cacheFreshFor     = time.Minute
)

// CacheNote marks an answer that is the last good copy of what the registry
// said, not what it says now.
type CacheNote struct {
	CachedAt string `json:"cachedAt"`
	Cause    string `json:"cause"`
}

// cacheNow is the clock the cache reads, replaced by tests.
var cacheNow = time.Now

var cacheFile = regexp.MustCompile(`^[0-9a-f]{64}\.json$`)

// pruneMu serialises trimming across every cache in the process; readers and
// writers need no lock because each file is replaced atomically.
var pruneMu sync.Mutex

type cacheEntry struct {
	Version   int             `json:"v"`
	Key       string          `json:"key"`
	FetchedAt time.Time       `json:"fetchedAt"`
	Body      json.RawMessage `json:"body"`
}

// diskCache keeps the last good answer per anonymous browse request.
type diskCache struct {
	dir        string
	maxEntries int
	maxBytes   int64
	maxAge     time.Duration
	freshFor   time.Duration
}

func newDiskCache(dir string) *diskCache {
	return &diskCache{dir: dir, maxEntries: cacheMaxEntries, maxBytes: cacheMaxBytes, maxAge: cacheMaxAge, freshFor: cacheFreshFor}
}

func (d *diskCache) fresh(e cacheEntry) bool {
	age := cacheNow().Sub(e.FetchedAt)
	return age >= 0 && age < d.freshFor
}

type refreshKey struct{}

// WithRefresh marks a read the person asked to repeat: it skips the fresh
// window of a stored copy and goes to the registry.
func WithRefresh(ctx context.Context) context.Context {
	return context.WithValue(ctx, refreshKey{}, true)
}

func refreshAsked(ctx context.Context) bool { return ctx.Value(refreshKey{}) == true }

func (d *diskCache) key(host, path string, query url.Values) string {
	sum := sha256.Sum256([]byte("market-cache/v1\n" + host + "\n" + path + "\n" + query.Encode()))
	return hex.EncodeToString(sum[:])
}

func (d *diskCache) path(key string) string { return filepath.Join(d.dir, key+".json") }

func (d *diskCache) load(key string) (cacheEntry, bool) {
	f, err := os.Open(d.path(key))
	if err != nil {
		return cacheEntry{}, false
	}
	defer f.Close()
	raw, err := io.ReadAll(io.LimitReader(f, d.maxBytes+1))
	if err != nil || int64(len(raw)) > d.maxBytes {
		return cacheEntry{}, false
	}
	var e cacheEntry
	if json.Unmarshal(raw, &e) != nil || e.Version != cacheEntryVersion || e.Key != key || !json.Valid(e.Body) {
		return cacheEntry{}, false
	}
	if age := cacheNow().Sub(e.FetchedAt); age > d.maxAge || age < -time.Hour {
		return cacheEntry{}, false
	}
	return e, true
}

func (d *diskCache) store(key string, body []byte) {
	e := cacheEntry{Version: cacheEntryVersion, Key: key, FetchedAt: cacheNow().UTC(), Body: body}
	d.write(e)
}

func (d *diskCache) write(e cacheEntry) {
	raw, err := json.Marshal(e)
	if err != nil || int64(len(raw)) > d.maxBytes {
		return
	}
	if os.MkdirAll(d.dir, 0o700) != nil {
		return
	}
	if fileutil.AtomicWriteFile(d.path(e.Key), raw, 0o600) != nil {
		return
	}
	d.prune()
}

func (d *diskCache) prune() {
	pruneMu.Lock()
	defer pruneMu.Unlock()
	items, err := os.ReadDir(d.dir)
	if err != nil {
		return
	}
	type held struct {
		name string
		at   time.Time
		size int64
	}
	var kept []held
	var total int64
	for _, it := range items {
		if it.IsDir() || !cacheFile.MatchString(it.Name()) {
			continue
		}
		info, err := it.Info()
		if err != nil {
			continue
		}
		if cacheNow().Sub(info.ModTime()) > d.maxAge {
			_ = os.Remove(filepath.Join(d.dir, it.Name()))
			continue
		}
		kept = append(kept, held{it.Name(), info.ModTime(), info.Size()})
		total += info.Size()
	}
	sort.Slice(kept, func(i, j int) bool { return kept[i].at.Before(kept[j].at) })
	for len(kept) > 0 && (len(kept) > d.maxEntries || total > d.maxBytes) {
		_ = os.Remove(filepath.Join(d.dir, kept[0].name))
		total -= kept[0].size
		kept = kept[1:]
	}
}

// cachedRegistry answers browse reads from disk only when the registry cannot.
type cachedRegistry struct {
	Registry
	cache *diskCache
}

// WithCacheRegistry wraps a registry so browse reads can fall back to a
// previously verified answer. Installs use the unwrapped source.
func WithCacheRegistry(reg Registry, root string) Registry {
	if strings.TrimSpace(root) == "" {
		return reg
	}
	return &cachedRegistry{Registry: reg, cache: newDiskCache(filepath.Join(root, "market"))}
}

func (c *cachedRegistry) List(ctx context.Context, q Query) (Page, error) {
	v := url.Values{}
	v.Set("kind", q.Kind)
	v.Set("q", q.Q)
	v.Set("sort", q.Sort)
	v.Set("offset", strconv.Itoa(q.Offset))
	if q.Pinned {
		v.Set("pinned", "1")
	}
	key := c.cache.key("registry", "/packages", v)
	held, have := c.cache.load(key)
	if have && !refreshAsked(ctx) && c.cache.fresh(held) {
		var page Page
		if json.Unmarshal(held.Body, &page) == nil {
			return page, nil
		}
	}
	page, err := c.Registry.List(ctx, q)
	if err == nil {
		page.Cache = nil
		if raw, marshalErr := json.Marshal(page); marshalErr == nil {
			c.cache.store(key, raw)
		}
		return page, nil
	}
	if !have || ctx.Err() != nil || !isBrowseFailure(err) {
		return Page{}, err
	}
	var stale Page
	if json.Unmarshal(held.Body, &stale) != nil {
		return Page{}, err
	}
	stale.Cache = noteFor(held, err)
	return stale, nil
}

func (c *cachedRegistry) Detail(ctx context.Context, slug string) (Detail, error) {
	key := c.cache.key("registry", "/packages/"+slug, nil)
	held, have := c.cache.load(key)
	if have && !refreshAsked(ctx) && c.cache.fresh(held) {
		var detail Detail
		if json.Unmarshal(held.Body, &detail) == nil {
			return detail, nil
		}
	}
	detail, err := c.Registry.Detail(ctx, slug)
	if err == nil {
		detail.Cache = nil
		if raw, marshalErr := json.Marshal(detail); marshalErr == nil {
			c.cache.store(key, raw)
		}
		return detail, nil
	}
	if !have || ctx.Err() != nil || !isBrowseFailure(err) {
		return Detail{}, err
	}
	var stale Detail
	if json.Unmarshal(held.Body, &stale) != nil {
		return Detail{}, err
	}
	stale.Cache = noteFor(held, err)
	return stale, nil
}

func isBrowseFailure(err error) bool {
	return errors.Is(err, ErrUnreachable) || errors.Is(err, ErrBadResponse)
}

func noteFor(held cacheEntry, err error) *CacheNote {
	cause := CacheCauseBadResponse
	if errors.Is(err, ErrUnreachable) {
		cause = CacheCauseUnreachable
	}
	return &CacheNote{CachedAt: held.FetchedAt.UTC().Format(time.RFC3339), Cause: cause}
}
