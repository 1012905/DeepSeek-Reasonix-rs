package market

import (
	"context"
	"errors"
	"strings"
	"testing"

	"reasonix/internal/ext/registry"
)

type fakeSource struct {
	entries []registry.Entry
	err     error
}

func (f *fakeSource) List(context.Context) ([]registry.Entry, error) { return f.entries, f.err }
func (f *fakeSource) Resolve(_ context.Context, id, version string) (registry.Entry, error) {
	if f.err != nil {
		return registry.Entry{}, f.err
	}
	for _, e := range f.entries {
		if e.ID == id && (version == "" || e.Version == version) {
			return e, nil
		}
	}
	return registry.Entry{}, registry.ErrNotFound
}
func (f *fakeSource) Fetch(context.Context, registry.Entry) (registry.Content, error) {
	return registry.Content{}, nil
}
func (f *fakeSource) SourceURL(ref string) (string, error) {
	return "https://example.test/" + ref, nil
}

func sampleEntry(id, kind string) registry.Entry {
	return registry.Entry{
		ID: id, Kind: kind, Version: "1.0.0", URL: "skills/" + id + "/",
		SHA256:     strings.Repeat("a", 64),
		Files:      []registry.File{{Path: "skills/" + id + "/SKILL.md", SHA256: strings.Repeat("b", 64), Size: 4}},
		MinVersion: "0.1.0", License: "MIT",
		Compat: registry.Compat{Platforms: []string{"any"}, Frontends: []string{"studio"}},
	}
}

func TestNewClientReadsTheSignedRegistry(t *testing.T) {
	c := NewClient(nil)
	reg, ok := c.reg.(*registry.Client)
	if !ok || reg.IndexURL() != registry.RegistryIndexURL {
		t.Fatalf("source = %#v", c.reg)
	}
}

func TestNewClientWithRegistryUsesTheGivenClient(t *testing.T) {
	reg := registry.NewClient(nil)
	if got := NewClientWithRegistry(nil, reg).reg; got != reg {
		t.Fatal("supplied registry client was not used")
	}
}

func TestListMapsLiveEntriesAndNarrowsThem(t *testing.T) {
	src := &fakeSource{entries: []registry.Entry{
		sampleEntry("kit", "skills"), sampleEntry("desk", "themes"),
	}}
	c := &Client{reg: src}
	page, err := c.List(context.Background(), Query{})
	if err != nil {
		t.Fatal(err)
	}
	if len(page.Packages) != 2 || page.Limit != defaultPageSize || page.Offset != 0 {
		t.Fatalf("page = %+v", page)
	}
	found := page.Packages[0]
	if found.Slug != "registry/desk" || found.Kind != "theme" || found.LatestVersion != "1.0.0" || found.Pinned == nil || !*found.Pinned || !found.Verified {
		t.Fatalf("package = %+v", found)
	}
	page, err = c.List(context.Background(), Query{Kind: "skill", Q: "kit"})
	if err != nil || len(page.Packages) != 1 || page.Packages[0].Name != "kit" {
		t.Fatalf("narrowed = %+v err = %v", page, err)
	}
	if page, err = c.List(context.Background(), Query{Kind: "mcp"}); err != nil || len(page.Packages) != 0 {
		t.Fatalf("unknown kind = %+v err = %v", page, err)
	}
}

func TestDetailPinsTheSignedDigestOfThePrimaryFile(t *testing.T) {
	c := &Client{reg: &fakeSource{entries: []registry.Entry{sampleEntry("kit", "skills")}}}
	d, err := c.Detail(context.Background(), "registry/kit")
	if err != nil {
		t.Fatal(err)
	}
	if d.Approved == nil || d.Approved.Version != "1.0.0" || d.Approved.ContentHash != "sha256:"+strings.Repeat("b", 64) {
		t.Fatalf("approved = %+v", d.Approved)
	}
	if d.Approved.Source != "https://example.test/skills/kit/SKILL.md" {
		t.Fatalf("source = %q", d.Approved.Source)
	}
	if _, err := c.Detail(context.Background(), "registry/other"); !errors.Is(err, ErrNotFound) {
		t.Fatalf("err = %v", err)
	}
}

func TestSourceFailuresCarryTheMarketIdentity(t *testing.T) {
	c := &Client{reg: &fakeSource{err: registry.ErrUnreachable}}
	if _, err := c.List(context.Background(), Query{}); !errors.Is(err, ErrUnreachable) {
		t.Fatalf("unreachable: %v", err)
	}
	c.reg = &fakeSource{err: registry.ErrBadSignature}
	if _, err := c.Detail(context.Background(), "registry/kit"); !errors.Is(err, ErrBadResponse) {
		t.Fatalf("bad signature: %v", err)
	}
}

func TestSplitSlugRefusesPathTricks(t *testing.T) {
	for _, s := range []string{"", "a", "a/b/c", "../b", "a/..", ".b/c", "a/.b", "a/..b", "a/b?x=1", "a/%2e%2e", `a\b/c`} {
		if _, _, err := SplitSlug(s); !errors.Is(err, ErrBadSlug) {
			t.Errorf("SplitSlug(%q) accepted", s)
		}
	}
	if h, n, err := SplitSlug("registry/review-kit"); err != nil || h != RegistryHandle || n != "review-kit" {
		t.Errorf("real slug refused: %v", err)
	}
}
