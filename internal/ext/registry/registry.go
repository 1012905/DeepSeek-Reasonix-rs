package registry

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"regexp"
	"runtime"
	"slices"
	"strconv"
	"strings"
	"sync"
	"time"

	"reasonix/internal/contract/provider"
	"reasonix/internal/platform/update"
	"reasonix/internal/safety/redirectguard"
)

// releaseHosts are the hosts a signed asset may be served from: the index and
// signature are release assets, and GitHub answers them with a redirect.
var releaseHosts = []string{"github.com", ".githubusercontent.com"}

const (
	// RegistryIndexURL is the signed catalogue the desktop and CLI read. The
	// publish workflow attaches the signed index to the latest release, so this
	// names that release asset and the signature sits beside it.
	RegistryIndexURL = "https://github.com/1012905/reasonix-registry/releases/latest/download/index.json"

	// RegistryContentURL is where an entry's relative files live: the signed
	// index travels with a release, the files it names stay on the branch.
	RegistryContentURL = "https://raw.githubusercontent.com/1012905/reasonix-registry/main/"

	signatureSuffix = ".minisig"
	requestTimeout  = 15 * time.Second
	maxIndexBytes   = 2 << 20
	maxSigBytes     = 8 << 10
	maxFileBytes    = 8 << 20
	maxFiles        = 256
	maxEntryBytes   = 32 << 20
)

var (
	// ErrUnreachable: no answer could be read from the registry origin.
	ErrUnreachable = errors.New("registry: unreachable")
	// ErrBadResponse: an answer was not a usable registry document.
	ErrBadResponse = errors.New("registry: unexpected response")
	// ErrBadSignature: the signature did not check out under the release key.
	ErrBadSignature = errors.New("registry: bad signature")
	// ErrNotFound: no live entry has that id and version.
	ErrNotFound = errors.New("registry: no such entry")
)

// File is one content file covered by a signed entry.
type File struct {
	Path   string `json:"path"`
	SHA256 string `json:"sha256"`
	Size   int64  `json:"size"`
}

// Compat declares the platforms and frontends an entry supports.
type Compat struct {
	Platforms []string `json:"platforms"`
	Frontends []string `json:"frontends"`
}

// Entry is one signed catalogue row. Fields are claims verified before use.
type Entry struct {
	ID         string `json:"id"`
	Kind       string `json:"kind"`
	Version    string `json:"version"`
	URL        string `json:"url"`
	SHA256     string `json:"sha256"`
	Files      []File `json:"files"`
	MinVersion string `json:"minVersion"`
	Compat     Compat `json:"compat"`
	License    string `json:"license"`
	Revoked    bool   `json:"revoked"`
}

// FetchedFile is a verified file and the signed origin it came from.
type FetchedFile struct {
	File
	URL  string
	Data []byte
}

// Content is every file of an entry, each checked against its signed digest.
type Content struct {
	Entry Entry
	URL   string
	Files []FetchedFile
}

// Client reads one signed registry. Its cache holds only index bytes that
// already passed signature verification.
type Client struct {
	http       *http.Client
	indexURL   string
	contentURL string
	version    string

	// Verify is the signature gate; tests replace it with a throwaway key pair.
	Verify func(data, sig []byte) error

	mu       sync.Mutex
	verified []byte
	etag     string
}

// NewClient reads the declared registry index for this running version.
func NewClient(hc *http.Client) *Client {
	return NewClientWithOrigin(hc, RegistryIndexURL, RegistryContentURL, provider.ClientVersion())
}

// NewClientWithOrigin binds a client to one signed index and the origin its
// relative entry files resolve against.
func NewClientWithOrigin(hc *http.Client, indexURL, contentURL, version string) *Client {
	if hc == nil {
		hc = &http.Client{}
	}
	transport := *hc
	transport.CheckRedirect = redirectguard.Follow(releaseHosts...)
	return &Client{
		http:       &transport,
		indexURL:   indexURL,
		contentURL: contentURL,
		version:    version,
		Verify:     update.Verify,
	}
}

// IndexURL reports the index this client reads.
func (c *Client) IndexURL() string { return c.indexURL }

// List returns the live entries this build and platform may use.
func (c *Client) List(ctx context.Context) ([]Entry, error) {
	data, err := c.indexBytes(ctx)
	if err != nil {
		return nil, err
	}
	entries, err := parseIndex(data)
	if err != nil {
		return nil, err
	}
	out := make([]Entry, 0, len(entries))
	for _, e := range entries {
		if e.Revoked || !platformAllowed(e.Compat.Platforms) || tooNew(e.MinVersion, c.version) {
			continue
		}
		out = append(out, e)
	}
	return out, nil
}

// Resolve returns one live entry, the newest version when version is empty.
func (c *Client) Resolve(ctx context.Context, id, version string) (Entry, error) {
	list, err := c.List(ctx)
	if err != nil {
		return Entry{}, err
	}
	var found *Entry
	for i := range list {
		if list[i].ID != id || (version != "" && list[i].Version != version) {
			continue
		}
		if found == nil || compareVersion(list[i].Version, found.Version) > 0 {
			found = &list[i]
		}
	}
	if found == nil {
		return Entry{}, fmt.Errorf("%w: %s@%s", ErrNotFound, id, version)
	}
	return *found, nil
}

// Fetch downloads every file of entry and verifies each digest and the entry's
// listing digest before returning any of them.
func (c *Client) Fetch(ctx context.Context, entry Entry) (Content, error) {
	if len(entry.Files) == 0 || len(entry.Files) > maxFiles {
		return Content{}, fmt.Errorf("%w: entry %s has an invalid file count", ErrBadResponse, entry.ID)
	}
	source, err := c.SourceURL(entry.URL)
	if err != nil {
		return Content{}, err
	}
	out := Content{Entry: entry, URL: source}
	listing := sha256.New()
	var total int
	for _, file := range entry.Files {
		if file.Size < 0 || file.Size > maxFileBytes {
			return Content{}, fmt.Errorf("%w: %s declares an invalid size", ErrBadResponse, file.Path)
		}
		total += int(file.Size)
		if total > maxEntryBytes {
			return Content{}, fmt.Errorf("%w: entry %s exceeds the byte cap", ErrBadResponse, entry.ID)
		}
		address, err := c.SourceURL(file.Path)
		if err != nil {
			return Content{}, err
		}
		body, _, _, err := c.get(ctx, address, nil, maxFileBytes)
		if err != nil {
			return Content{}, err
		}
		if int64(len(body)) != file.Size {
			return Content{}, fmt.Errorf("%w: %s is %d bytes, want %d", ErrBadResponse, file.Path, len(body), file.Size)
		}
		if err := update.CheckSHA256(body, file.SHA256); err != nil {
			return Content{}, fmt.Errorf("%w: %s: %w", ErrBadResponse, file.Path, err)
		}
		_, _ = fmt.Fprintf(listing, "%s\t%s\n", file.SHA256, file.Path)
		out.Files = append(out.Files, FetchedFile{File: file, URL: address, Data: body})
	}
	if got := hex.EncodeToString(listing.Sum(nil)); got != entry.SHA256 {
		return Content{}, fmt.Errorf("%w: entry %s listing digest %s, want %s", ErrBadResponse, entry.ID, got, entry.SHA256)
	}
	return out, nil
}

// SourceURL resolves a signed relative path against the content origin.
func (c *Client) SourceURL(ref string) (string, error) {
	base, err := url.Parse(c.contentURL)
	if err != nil {
		return "", fmt.Errorf("%w: bad content URL", ErrBadResponse)
	}
	rel, err := url.Parse(ref)
	if err != nil {
		return "", fmt.Errorf("%w: bad entry path %q", ErrBadResponse, ref)
	}
	resolved := base.ResolveReference(rel)
	if resolved.Scheme != base.Scheme || resolved.Host != base.Host {
		return "", fmt.Errorf("%w: entry path leaves the content origin: %q", ErrBadResponse, ref)
	}
	return resolved.String(), nil
}

func (c *Client) indexBytes(ctx context.Context) ([]byte, error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	var cond http.Header
	if c.etag != "" {
		cond = http.Header{"If-None-Match": []string{c.etag}}
	}
	body, header, status, err := c.get(ctx, c.indexURL, cond, maxIndexBytes)
	if err != nil {
		return nil, err
	}
	if status == http.StatusNotModified {
		if len(c.verified) == 0 {
			return nil, fmt.Errorf("%w: 304 before a verified index was cached", ErrBadResponse)
		}
		return c.verified, nil
	}
	sig, _, _, err := c.get(ctx, c.indexURL+signatureSuffix, nil, maxSigBytes)
	if err != nil {
		return nil, err
	}
	if err := c.Verify(body, sig); err != nil {
		return nil, fmt.Errorf("%w: %w", ErrBadSignature, err)
	}
	if _, err := parseIndex(body); err != nil {
		return nil, err
	}
	c.verified = body
	c.etag = header.Get("ETag")
	return body, nil
}

func (c *Client) get(ctx context.Context, address string, extra http.Header, limit int) ([]byte, http.Header, int, error) {
	ctx, cancel := context.WithTimeout(ctx, requestTimeout)
	defer cancel()
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, address, nil)
	if err != nil {
		return nil, nil, 0, fmt.Errorf("%w: %w", ErrBadResponse, err)
	}
	req.Header.Set("Accept", "application/json")
	req.Header.Set("User-Agent", "reasonix-registry/1.0")
	for name, values := range extra {
		for _, value := range values {
			req.Header.Add(name, value)
		}
	}
	resp, err := c.http.Do(req)
	if err != nil {
		return nil, nil, 0, fmt.Errorf("%w: %w", ErrUnreachable, err)
	}
	defer resp.Body.Close()
	body, err := io.ReadAll(io.LimitReader(resp.Body, int64(limit)+1))
	if err != nil {
		return nil, nil, 0, fmt.Errorf("%w: %w", ErrUnreachable, err)
	}
	if len(body) > limit {
		return nil, resp.Header, resp.StatusCode, fmt.Errorf("%w: body exceeds %d bytes", ErrBadResponse, limit)
	}
	if resp.StatusCode == http.StatusNotModified {
		return body, resp.Header, resp.StatusCode, nil
	}
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		if resp.StatusCode == http.StatusNotFound {
			return nil, resp.Header, resp.StatusCode, fmt.Errorf("%w: %s", ErrNotFound, address)
		}
		return nil, resp.Header, resp.StatusCode, fmt.Errorf("%w: HTTP %d for %s", ErrBadResponse, resp.StatusCode, address)
	}
	return body, resp.Header, resp.StatusCode, nil
}

var (
	idPattern      = regexp.MustCompile(`^[a-z0-9][a-z0-9._-]{1,63}$`)
	versionPattern = regexp.MustCompile(`^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$`)
	urlPattern     = regexp.MustCompile(`^(mcp/[a-z0-9._-]+\.json|(skills|themes|plugins)/[a-z0-9._-]+/)$`)
	shaPattern     = regexp.MustCompile(`^[a-f0-9]{64}$`)
	filePattern    = regexp.MustCompile(`^(mcp/[a-z0-9._-]+\.json|(skills|themes|plugins)/[a-z0-9._-]+/.+)$`)
	licensePattern = regexp.MustCompile(`^[A-Za-z0-9.+-]{2,64}$`)
)

func parseIndex(data []byte) ([]Entry, error) {
	var top map[string]json.RawMessage
	if err := json.Unmarshal(data, &top); err != nil {
		return nil, fmt.Errorf("%w: index is not an object: %w", ErrBadResponse, err)
	}
	if err := exactKeys(top, "schemaVersion", "entries"); err != nil {
		return nil, fmt.Errorf("%w: index: %w", ErrBadResponse, err)
	}
	var schema int
	if err := json.Unmarshal(top["schemaVersion"], &schema); err != nil || schema != 1 {
		return nil, fmt.Errorf("%w: unsupported schemaVersion", ErrBadResponse)
	}
	var rows []json.RawMessage
	if err := json.Unmarshal(top["entries"], &rows); err != nil {
		return nil, fmt.Errorf("%w: entries is not an array: %w", ErrBadResponse, err)
	}
	out := make([]Entry, 0, len(rows))
	for _, row := range rows {
		e, err := parseEntry(row)
		if err != nil {
			return nil, err
		}
		out = append(out, e)
	}
	return out, nil
}

func parseEntry(row json.RawMessage) (Entry, error) {
	var m map[string]json.RawMessage
	if err := json.Unmarshal(row, &m); err != nil {
		return Entry{}, fmt.Errorf("%w: entry is not an object: %w", ErrBadResponse, err)
	}
	if err := exactKeys(m, "id", "kind", "version", "url", "sha256", "files", "minVersion", "compat", "license", "revoked"); err != nil {
		return Entry{}, fmt.Errorf("%w: entry: %w", ErrBadResponse, err)
	}
	var e Entry
	var err error
	if e.ID, err = text(m, "id"); err != nil || !idPattern.MatchString(e.ID) {
		return Entry{}, fmt.Errorf("%w: invalid entry id", ErrBadResponse)
	}
	if e.Kind, err = text(m, "kind"); err != nil || !kindAllowed(e.Kind) {
		return Entry{}, fmt.Errorf("%w: invalid kind for %s", ErrBadResponse, e.ID)
	}
	if e.Version, err = text(m, "version"); err != nil || !versionPattern.MatchString(e.Version) {
		return Entry{}, fmt.Errorf("%w: invalid version for %s", ErrBadResponse, e.ID)
	}
	if e.URL, err = text(m, "url"); err != nil || !urlPattern.MatchString(e.URL) {
		return Entry{}, fmt.Errorf("%w: invalid url for %s", ErrBadResponse, e.ID)
	}
	if e.SHA256, err = text(m, "sha256"); err != nil || !shaPattern.MatchString(e.SHA256) {
		return Entry{}, fmt.Errorf("%w: invalid sha256 for %s", ErrBadResponse, e.ID)
	}
	if e.MinVersion, err = text(m, "minVersion"); err != nil || !versionPattern.MatchString(e.MinVersion) {
		return Entry{}, fmt.Errorf("%w: invalid minVersion for %s", ErrBadResponse, e.ID)
	}
	if e.License, err = text(m, "license"); err != nil || !licensePattern.MatchString(e.License) {
		return Entry{}, fmt.Errorf("%w: invalid license for %s", ErrBadResponse, e.ID)
	}
	if e.Revoked, err = boolean(m, "revoked"); err != nil {
		return Entry{}, err
	}
	if e.Files, err = parseFiles(m["files"], e.ID); err != nil {
		return Entry{}, err
	}
	var compat map[string]json.RawMessage
	if err := json.Unmarshal(m["compat"], &compat); err != nil {
		return Entry{}, fmt.Errorf("%w: invalid compat for %s", ErrBadResponse, e.ID)
	}
	if err := exactKeys(compat, "platforms", "frontends"); err != nil {
		return Entry{}, fmt.Errorf("%w: compat for %s: %w", ErrBadResponse, e.ID, err)
	}
	if e.Compat.Platforms, err = choices(compat["platforms"], "platforms", "windows", "macos", "linux", "any"); err != nil {
		return Entry{}, err
	}
	if e.Compat.Frontends, err = choices(compat["frontends"], "frontends", "studio", "cli"); err != nil {
		return Entry{}, err
	}
	return e, nil
}

func parseFiles(raw json.RawMessage, id string) ([]File, error) {
	var rows []json.RawMessage
	if err := json.Unmarshal(raw, &rows); err != nil || len(rows) == 0 || len(rows) > maxFiles {
		return nil, fmt.Errorf("%w: invalid files for %s", ErrBadResponse, id)
	}
	out := make([]File, 0, len(rows))
	for _, row := range rows {
		var m map[string]json.RawMessage
		if err := json.Unmarshal(row, &m); err != nil {
			return nil, fmt.Errorf("%w: file of %s is not an object", ErrBadResponse, id)
		}
		if err := exactKeys(m, "path", "sha256", "size"); err != nil {
			return nil, fmt.Errorf("%w: file of %s: %w", ErrBadResponse, id, err)
		}
		var f File
		var err error
		if f.Path, err = text(m, "path"); err != nil || len(f.Path) > 1024 || !filePattern.MatchString(f.Path) {
			return nil, fmt.Errorf("%w: invalid file path for %s", ErrBadResponse, id)
		}
		if f.SHA256, err = text(m, "sha256"); err != nil || !shaPattern.MatchString(f.SHA256) {
			return nil, fmt.Errorf("%w: invalid file digest for %s", ErrBadResponse, id)
		}
		if f.Size, err = integer(m, "size"); err != nil || f.Size < 0 {
			return nil, fmt.Errorf("%w: invalid file size for %s", ErrBadResponse, id)
		}
		out = append(out, f)
	}
	return out, nil
}

func exactKeys(m map[string]json.RawMessage, want ...string) error {
	if len(m) != len(want) {
		return errors.New("unexpected fields")
	}
	for _, key := range want {
		if _, ok := m[key]; !ok {
			return fmt.Errorf("missing field %q", key)
		}
	}
	return nil
}

func text(m map[string]json.RawMessage, key string) (string, error) {
	var s string
	if err := json.Unmarshal(m[key], &s); err != nil {
		return "", err
	}
	return s, nil
}

func boolean(m map[string]json.RawMessage, key string) (bool, error) {
	var b bool
	if err := json.Unmarshal(m[key], &b); err != nil {
		return false, fmt.Errorf("%w: invalid %s", ErrBadResponse, key)
	}
	return b, nil
}

func integer(m map[string]json.RawMessage, key string) (int64, error) {
	var n int64
	if err := json.Unmarshal(m[key], &n); err != nil {
		return 0, err
	}
	return n, nil
}

func choices(raw json.RawMessage, key string, allowed ...string) ([]string, error) {
	var list []string
	if err := json.Unmarshal(raw, &list); err != nil || len(list) == 0 {
		return nil, fmt.Errorf("%w: invalid %s", ErrBadResponse, key)
	}
	for _, value := range list {
		if !allowedValue(value, allowed...) {
			return nil, fmt.Errorf("%w: unknown %s %q", ErrBadResponse, key, value)
		}
	}
	return list, nil
}

func allowedValue(value string, allowed ...string) bool {
	return slices.Contains(allowed, value)
}

func kindAllowed(kind string) bool {
	switch kind {
	case "mcp", "skill", "skills", "plugin", "plugins", "theme", "themes":
		return true
	}
	return false
}

func platformAllowed(platforms []string) bool {
	for _, p := range platforms {
		if p == "any" || p == runtime.GOOS {
			return true
		}
	}
	return false
}

func tooNew(minimum, running string) bool {
	want, ok := parseVersion(minimum)
	if !ok {
		return true
	}
	have, ok := parseVersion(running)
	if !ok {
		return false
	}
	return compareParts(want, have) > 0
}

type semver struct {
	core [3]int
	pre  string
}

func parseVersion(s string) (semver, bool) {
	s = strings.TrimPrefix(strings.TrimSpace(s), "v")
	core, pre, _ := strings.Cut(s, "-")
	parts := strings.Split(core, ".")
	if len(parts) != 3 || (pre != "" && !versionPattern.MatchString(s)) {
		return semver{}, false
	}
	var out semver
	for i, part := range parts {
		n, err := strconv.Atoi(part)
		if err != nil {
			return semver{}, false
		}
		out.core[i] = n
	}
	out.pre = pre
	return out, true
}

func compareVersion(a, b string) int {
	av, okA := parseVersion(a)
	bv, okB := parseVersion(b)
	if !okA || !okB {
		return strings.Compare(a, b)
	}
	return compareParts(av, bv)
}

func compareParts(a, b semver) int {
	for i := range a.core {
		if a.core[i] < b.core[i] {
			return -1
		}
		if a.core[i] > b.core[i] {
			return 1
		}
	}
	if a.pre == b.pre {
		return 0
	}
	if a.pre == "" {
		return 1
	}
	if b.pre == "" {
		return -1
	}
	return strings.Compare(a.pre, b.pre)
}
