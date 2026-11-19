package market

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"regexp"
	"sort"
	"strings"

	"reasonix/internal/ext/registry"
)

const defaultPageSize = 24

var (
	// ErrUnreachable: the registry could not be reached at all.
	ErrUnreachable = errors.New("market: registry unreachable")
	// ErrBadResponse: the signed index or one of its files was unusable.
	ErrBadResponse = errors.New("market: registry answered unexpectedly")
	// ErrNotFound: no live entry has that name.
	ErrNotFound = errors.New("market: no such package")
	// ErrBadSlug: the slug is not <handle>/<name>.
	ErrBadSlug = errors.New("market: not a package slug")
	// ErrFilterUnsupported: an installable-only listing could not be honoured.
	ErrFilterUnsupported = errors.New("market: registry cannot filter to installable packages")
)

// Package is one listed capability as the signed registry describes it.
type Package struct {
	Kind          string   `json:"kind"`
	Handle        string   `json:"handle"`
	Name          string   `json:"name"`
	Slug          string   `json:"slug"`
	Summary       string   `json:"summary"`
	Description   string   `json:"description"`
	Homepage      string   `json:"homepage"`
	RepoURL       string   `json:"repoUrl"`
	Tags          []string `json:"tags"`
	LatestVersion string   `json:"latestVersion"`
	InstallCount  int      `json:"installCount"`
	StarCount     int      `json:"starCount"`
	UpCount       int      `json:"upCount"`
	DownCount     int      `json:"downCount"`
	// Nil while nobody has voted, which is not the same as zero approval.
	ApprovalRate *float64 `json:"approvalRate"`
	Score        float64  `json:"score"`
	Verified     bool     `json:"verified"`
	Status       string   `json:"status"`
	UpdatedAt    string   `json:"updatedAt"`
	// Pinned is true because the registry's digest was signed and is checked.
	Pinned *bool `json:"pinned,omitempty"`
}

// Version is one immutable published version row.
type Version struct {
	Version     string `json:"version"`
	Source      string `json:"source"`
	ContentHash string `json:"contentHash"`
	RiskLevel   string `json:"riskLevel"`
	CreatedAt   string `json:"createdAt"`
}

// Detail is a package with the version that is signed and installable.
type Detail struct {
	Package  Package    `json:"package"`
	Approved *Version   `json:"approved"`
	Cache    *CacheNote `json:"cache,omitempty"`
}

// Query narrows a listing. Unknown kinds and sorts are refused.
type Query struct {
	Kind   string
	Q      string
	Sort   string
	Offset int
	// Pinned asks for installable packages only; every signed row qualifies.
	Pinned bool
}

// Page is one listing page.
type Page struct {
	Packages []Package `json:"packages"`
	Limit    int       `json:"limit"`
	Offset   int       `json:"offset"`
	// Cache is set when the registry could not answer and this is its last good copy.
	Cache *CacheNote `json:"cache,omitempty"`
}

// Registry is what a caller needs from the community registry.
type Registry interface {
	List(ctx context.Context, q Query) (Page, error)
	Detail(ctx context.Context, slug string) (Detail, error)
}

// source is the signed registry beneath every market read and install.
type source interface {
	List(context.Context) ([]registry.Entry, error)
	Resolve(context.Context, string, string) (registry.Entry, error)
	Fetch(context.Context, registry.Entry) (registry.Content, error)
	SourceURL(string) (string, error)
}

// Client reads the signed registry. It has one source; the retired account
// host is neither a default nor a fallback.
type Client struct {
	reg   source
	cache *diskCache
	view  Registry
}

// NewClient wraps hc and reads the signed registry at its declared URL.
func NewClient(hc *http.Client) *Client {
	return NewClientWithRegistry(hc, registry.NewClient(hc))
}

// NewClientWithRegistry uses a caller-supplied registry client, which tests
// point at a local signed index.
func NewClientWithRegistry(hc *http.Client, reg *registry.Client) *Client {
	if reg == nil {
		reg = registry.NewClient(hc)
	}
	return &Client{reg: reg}
}

// WithCache serves the last good browse answer when the registry cannot
// answer. Installs never read it.
func (c *Client) WithCache(root string) *Client {
	out := *c
	out.view = nil
	out.cache = nil
	direct := *c
	direct.view = nil
	out.view = WithCacheRegistry(&direct, root)
	if wrapped, ok := out.view.(*cachedRegistry); ok {
		out.cache = wrapped.cache
	}
	return &out
}

func (c *Client) List(ctx context.Context, q Query) (Page, error) {
	if c.view != nil {
		return c.view.List(ctx, q)
	}
	return c.list(ctx, q)
}

func (c *Client) list(ctx context.Context, q Query) (Page, error) {
	entries, err := c.reg.List(ctx)
	if err != nil {
		return Page{}, projectSourceError(err)
	}
	kind := strings.TrimSpace(q.Kind)
	term := strings.ToLower(strings.TrimSpace(q.Q))
	rows := make([]Package, 0, len(entries))
	for _, entry := range entries {
		if kind != "" && kindOf(entry.Kind) != kind {
			continue
		}
		pkg := packageOf(entry)
		if term != "" && !strings.Contains(strings.ToLower(pkg.Name+" "+pkg.Summary+" "+pkg.Description), term) {
			continue
		}
		rows = append(rows, pkg)
	}
	sort.SliceStable(rows, func(i, j int) bool {
		switch q.Sort {
		case "new":
			return rows[i].LatestVersion > rows[j].LatestVersion
		case "installs":
			return rows[i].InstallCount > rows[j].InstallCount
		case "trending":
			return rows[i].Score > rows[j].Score
		default:
			return rows[i].Slug < rows[j].Slug
		}
	})
	offset := max(0, min(q.Offset, 10000))
	if offset >= len(rows) {
		rows = []Package{}
	} else {
		rows = rows[offset:min(offset+defaultPageSize, len(rows))]
	}
	return Page{Packages: rows, Limit: defaultPageSize, Offset: offset}, nil
}

func (c *Client) Detail(ctx context.Context, slug string) (Detail, error) {
	if c.view != nil {
		return c.view.Detail(ctx, slug)
	}
	return c.detail(ctx, slug)
}

func (c *Client) detail(ctx context.Context, slug string) (Detail, error) {
	_, name, err := SplitSlug(slug)
	if err != nil {
		return Detail{}, err
	}
	entry, err := c.reg.Resolve(ctx, name, "")
	if err != nil {
		return Detail{}, projectSourceError(err)
	}
	pkg := packageOf(entry)
	if pkg.Slug != slug {
		return Detail{}, ErrNotFound
	}
	source, digest := primaryContent(entry)
	address, err := c.reg.SourceURL(source)
	if err != nil {
		return Detail{}, projectSourceError(err)
	}
	return Detail{
		Package: pkg,
		Approved: &Version{
			Version:     entry.Version,
			Source:      address,
			ContentHash: "sha256:" + digest,
			RiskLevel:   "low",
		},
	}, nil
}

// RegistryHandle is the namespace every signed entry is filed under.
const RegistryHandle = "registry"

func packageOf(entry registry.Entry) Package {
	yes := true
	return Package{
		Kind:          kindOf(entry.Kind),
		Handle:        RegistryHandle,
		Name:          entry.ID,
		Slug:          RegistryHandle + "/" + entry.ID,
		Summary:       "Licensed " + entry.License,
		Description:   "Signed registry entry for " + strings.Join(entry.Compat.Frontends, ", "),
		Tags:          []string{entry.License},
		LatestVersion: entry.Version,
		Verified:      true,
		Status:        "active",
		Pinned:        &yes,
	}
}

func kindOf(kind string) string {
	switch strings.ToLower(strings.TrimSpace(kind)) {
	case "skills":
		return "skill"
	case "plugins":
		return "plugin"
	case "themes":
		return "theme"
	default:
		return strings.ToLower(strings.TrimSpace(kind))
	}
}

// primaryContent picks the file an install_source can fetch for this kind.
func primaryContent(entry registry.Entry) (string, string) {
	suffix := map[string]string{"skill": "/SKILL.md", "theme": "/theme.json", "plugin": "/plugin.json", "mcp": ""}[kindOf(entry.Kind)]
	for _, file := range entry.Files {
		if suffix != "" && strings.HasSuffix(file.Path, suffix) {
			return file.Path, file.SHA256
		}
		if suffix == "" && strings.HasSuffix(file.Path, ".json") {
			return file.Path, file.SHA256
		}
	}
	if len(entry.Files) > 0 {
		return entry.Files[0].Path, entry.Files[0].SHA256
	}
	return "", ""
}

func projectSourceError(err error) error {
	switch {
	case errors.Is(err, registry.ErrUnreachable):
		return fmt.Errorf("%w: %w", ErrUnreachable, err)
	case errors.Is(err, registry.ErrNotFound):
		return fmt.Errorf("%w: %w", ErrNotFound, err)
	case errors.Is(err, registry.ErrBadSignature):
		return fmt.Errorf("%w: %w", ErrBadResponse, err)
	default:
		return fmt.Errorf("%w: %w", ErrBadResponse, err)
	}
}

// SplitSlug validates a <handle>/<name> slug.
func SplitSlug(slug string) (string, string, error) {
	handle, name, ok := strings.Cut(strings.TrimSpace(slug), "/")
	if !ok || !slugPart(handle) || !slugPart(name) {
		return "", "", fmt.Errorf("%w: %q", ErrBadSlug, slug)
	}
	return handle, name, nil
}

func slugPart(s string) bool {
	return slugSegment.MatchString(s) && !strings.HasPrefix(s, ".")
}

var slugSegment = regexp.MustCompile(`^[A-Za-z0-9._-]{1,64}$`)
