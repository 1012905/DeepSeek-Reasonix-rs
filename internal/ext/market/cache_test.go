package market

import (
	"context"
	"errors"
	"fmt"
	"net/url"
	"os"
	"path/filepath"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"reasonix/internal/base/testenv"
)

type cachedStub struct {
	page   Page
	detail Detail
	err    error
	calls  atomic.Int32
}

func (s *cachedStub) List(context.Context, Query) (Page, error) {
	s.calls.Add(1)
	if s.err != nil {
		return Page{}, s.err
	}
	page := s.page
	page.Cache = nil
	return page, nil
}

func (s *cachedStub) Detail(context.Context, string) (Detail, error) {
	s.calls.Add(1)
	if s.err != nil {
		return Detail{}, s.err
	}
	detail := s.detail
	detail.Cache = nil
	return detail, nil
}

func wrap(dir string, reg Registry) Registry { return WithCacheRegistry(reg, dir) }

func cachedPage(reg Registry, ctx context.Context, q Query) (Page, error) {
	return reg.List(ctx, q)
}

func freezeClock(t *testing.T) *time.Time {
	t.Helper()
	now := time.Date(2026, 10, 6, 12, 0, 0, 0, time.UTC)
	old := cacheNow
	cacheNow = func() time.Time { return now }
	t.Cleanup(func() { cacheNow = old })
	return &now
}

func cacheFiles(t *testing.T, dir string) []string {
	t.Helper()
	items, _ := filepath.Glob(filepath.Join(dir, "market", "*.json"))
	return items
}

func TestFirstReadWithNoCacheAndNoRegistryIsUnreachable(t *testing.T) {
	reg := wrap(testenv.TempDir(t), &cachedStub{err: ErrUnreachable})
	page, err := cachedPage(reg, context.Background(), Query{})
	if !errors.Is(err, ErrUnreachable) || page.Cache != nil {
		t.Fatalf("page = %+v err = %v", page, err)
	}
}

func TestListFallsBackToTheLastGoodCopyAndSaysSo(t *testing.T) {
	now := freezeClock(t)
	stub := &cachedStub{page: Page{Packages: []Package{{Slug: "a/kit"}}}}
	reg := wrap(testenv.TempDir(t), stub)
	if _, err := reg.List(context.Background(), Query{}); err != nil {
		t.Fatal(err)
	}
	*now = now.Add(time.Hour)
	stub.err = ErrUnreachable
	page, err := reg.List(context.Background(), Query{})
	if err != nil {
		t.Fatal(err)
	}
	if page.Cache == nil || page.Cache.Cause != CacheCauseUnreachable || page.Cache.CachedAt != "2026-10-06T12:00:00Z" {
		t.Fatalf("cache note = %+v", page.Cache)
	}
	if len(page.Packages) != 1 || page.Packages[0].Slug != "a/kit" {
		t.Fatalf("packages = %+v", page.Packages)
	}
}

func TestBadResponsesFallBackButClientErrorsDoNot(t *testing.T) {
	now := freezeClock(t)
	stub := &cachedStub{page: Page{Packages: []Package{{Slug: "a/kit"}}}}
	reg := wrap(testenv.TempDir(t), stub)
	if _, err := reg.List(context.Background(), Query{}); err != nil {
		t.Fatal(err)
	}
	*now = now.Add(time.Hour)
	stub.err = ErrBadResponse
	if page, err := reg.List(context.Background(), Query{}); err != nil || page.Cache == nil || page.Cache.Cause != CacheCauseBadResponse {
		t.Fatalf("bad response: page = %+v err = %v", page, err)
	}
	stub.err = ErrNotFound
	if _, err := reg.List(context.Background(), Query{}); !errors.Is(err, ErrNotFound) {
		t.Fatalf("client error was masked: %v", err)
	}
}

func TestAFreshCopyIsServedWithoutTheNetwork(t *testing.T) {
	now := freezeClock(t)
	stub := &cachedStub{page: Page{Packages: []Package{{Slug: "a/kit"}}}}
	reg := wrap(testenv.TempDir(t), stub)
	if _, err := reg.List(context.Background(), Query{}); err != nil {
		t.Fatal(err)
	}
	*now = now.Add(cacheFreshFor - time.Second)
	stub.err = ErrUnreachable
	page, err := reg.List(context.Background(), Query{})
	if err != nil || page.Cache != nil || stub.calls.Load() != 1 {
		t.Fatalf("page = %+v err = %v calls = %d", page, err, stub.calls.Load())
	}
}

func TestARepeatedReadSkipsTheFreshWindow(t *testing.T) {
	freezeClock(t)
	stub := &cachedStub{page: Page{Packages: []Package{{Slug: "a/kit"}}}}
	reg := wrap(testenv.TempDir(t), stub)
	if _, err := reg.List(context.Background(), Query{}); err != nil {
		t.Fatal(err)
	}
	stub.err = ErrBadResponse
	page, err := reg.List(WithRefresh(context.Background()), Query{})
	if err != nil || page.Cache == nil || stub.calls.Load() != 2 {
		t.Fatalf("repeated read: page = %+v err = %v calls = %d", page, err, stub.calls.Load())
	}
}

func TestACancelledRequestIsNotAnsweredFromTheCache(t *testing.T) {
	now := freezeClock(t)
	stub := &cachedStub{page: Page{Packages: []Package{{Slug: "a/kit"}}}}
	reg := wrap(testenv.TempDir(t), stub)
	if _, err := reg.List(context.Background(), Query{}); err != nil {
		t.Fatal(err)
	}
	*now = now.Add(time.Hour)
	stub.err = ErrUnreachable
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if _, err := reg.List(ctx, Query{}); !errors.Is(err, ErrUnreachable) {
		t.Fatalf("cancelled read used the cache: %v", err)
	}
}

func TestQueriesAndDetailsKeepSeparateCopies(t *testing.T) {
	freezeClock(t)
	dir := testenv.TempDir(t)
	stub := &cachedStub{page: Page{Packages: []Package{{Slug: "a/kit"}}}, detail: Detail{Package: Package{Slug: "a/kit"}}}
	reg := wrap(dir, stub)
	for _, q := range []Query{{Q: "kit"}, {Q: "other"}} {
		if _, err := reg.List(context.Background(), q); err != nil {
			t.Fatal(err)
		}
	}
	if _, err := reg.Detail(context.Background(), "a/kit"); err != nil {
		t.Fatal(err)
	}
	if files := cacheFiles(t, dir); len(files) != 3 {
		t.Fatalf("%d cache files, want 3", len(files))
	}
	again := wrap(dir, &cachedStub{err: ErrUnreachable})
	if page, err := again.List(context.Background(), Query{Q: "kit"}); err != nil || len(page.Packages) != 1 {
		t.Fatalf("restart list = %+v err = %v", page, err)
	}
	if d, err := again.Detail(context.Background(), "a/kit"); err != nil || d.Package.Slug != "a/kit" {
		t.Fatalf("restart detail = %+v err = %v", d, err)
	}
}

func TestCorruptFilesAreIgnoredAndNoRootMeansNoCache(t *testing.T) {
	dir := testenv.TempDir(t)
	if err := os.MkdirAll(filepath.Join(dir, "market"), 0o700); err != nil {
		t.Fatal(err)
	}
	stub := &cachedStub{page: Page{Packages: []Package{{Slug: "a/kit"}}}}
	if err := os.WriteFile(filepath.Join(dir, "market", "deadbeef.json"), []byte("{"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := wrap(dir, stub).List(context.Background(), Query{}); err != nil {
		t.Fatal(err)
	}
	if wrap("", stub) != stub {
		t.Fatal("an empty root must leave the source unwrapped")
	}
}

func TestTheKeyIsCanonicalPerHostPathAndQuery(t *testing.T) {
	d := newDiskCache(testenv.TempDir(t))
	a := url.Values{"limit": {"24"}, "q": {"x"}}
	b := url.Values{}
	b.Set("q", "x")
	b.Set("limit", "24")
	if d.key("h", "/p", a) != d.key("h", "/p", b) || d.key("h", "/p", a) == d.key("g", "/p", a) {
		t.Fatal("key must be canonical per host, path and parameters")
	}
}

func TestConcurrentReadersAndWritersAgree(t *testing.T) {
	freezeClock(t)
	dir := testenv.TempDir(t)
	stub := &cachedStub{page: Page{Packages: []Package{{Slug: "a/kit"}}}}
	reg := wrap(dir, stub)
	var wg sync.WaitGroup
	for range 16 {
		wg.Go(func() {
			if _, err := reg.List(context.Background(), Query{}); err != nil {
				t.Errorf("list: %v", err)
			}
		})
	}
	wg.Wait()
	if stub.calls.Load() == 0 {
		t.Fatal("no registry read happened")
	}
}

func TestTheCacheStaysWithinItsLimits(t *testing.T) {
	freezeClock(t)
	dir := testenv.TempDir(t)
	d := newDiskCache(dir)
	for i := range cacheMaxEntries + 8 {
		d.store(fmt.Sprintf("%064x", i), []byte(`{"ok":true}`))
	}
	if files := cacheFiles(t, dir); len(files) > cacheMaxEntries {
		t.Fatalf("%d files over the cap of %d", len(files), cacheMaxEntries)
	}
}
