package serve

import (
	"context"
	"encoding/json"
	"net/http"
	"strings"
	"sync/atomic"
	"testing"

	"reasonix/internal/contract/config"
	"reasonix/internal/ext/market"
)

type scriptedRegistry struct {
	page   market.Page
	detail market.Detail
	down   *atomic.Bool
}

func (s *scriptedRegistry) List(context.Context, market.Query) (market.Page, error) {
	if s.down.Load() {
		return market.Page{}, market.ErrBadResponse
	}
	return s.page, nil
}

func (s *scriptedRegistry) Detail(context.Context, string) (market.Detail, error) {
	if s.down.Load() {
		return market.Detail{}, market.ErrBadResponse
	}
	return s.detail, nil
}

func realRegistryAt(t *testing.T) *atomic.Bool {
	t.Helper()
	down := &atomic.Bool{}
	yes := true
	reg := &scriptedRegistry{
		page: market.Page{Packages: []market.Package{{Slug: "registry/kit", Kind: "skill", Name: "kit", Status: "active", Pinned: &yes, LatestVersion: "1.0.0"}}, Limit: 24},
		detail: market.Detail{
			Package:  market.Package{Slug: "registry/kit", Kind: "skill", Status: "active", LatestVersion: "1.0.0", Pinned: &yes},
			Approved: &market.Version{Version: "1.0.0", Source: "https://example.test/SKILL.md", ContentHash: "sha256:" + strings.Repeat("a", 64)},
		},
		down: down,
	}
	oldReg, oldBrowse := marketRegistry, marketBrowser
	marketRegistry = func(*http.Client) market.Registry { return reg }
	marketBrowser = func(*http.Client) market.Registry { return market.WithCacheRegistry(reg, config.CacheDir()) }
	t.Cleanup(func() { marketRegistry, marketBrowser = oldReg, oldBrowse })
	return down
}

func marketGetJSON(t *testing.T, u string) (int, map[string]any) {
	t.Helper()
	resp, err := http.Get(u)
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	var out map[string]any
	_ = json.NewDecoder(resp.Body).Decode(&out)
	return resp.StatusCode, out
}

func TestMarketBrowseAnswersFromTheLastGoodCopyAndLabelsIt(t *testing.T) {
	_, _, base := pluginHome(t)
	down := realRegistryAt(t)

	code, fresh := marketGetJSON(t, base+"/market/packages?q=kit")
	if code != 200 || fresh["cache"] != nil {
		t.Fatalf("fresh list = %d %v", code, fresh)
	}
	if code, _ = marketGetJSON(t, base+"/market/packages/registry/kit"); code != 200 {
		t.Fatalf("detail = %d", code)
	}

	down.Store(true)
	if code, again := marketGetJSON(t, base+"/market/packages?q=kit"); code != 200 || again["cache"] != nil {
		t.Fatalf("inside the fresh window the copy is the answer, unlabelled: %d %v", code, again)
	}
	code, list := marketGetJSON(t, base+"/market/packages?q=kit&refresh=1")
	note, _ := list["cache"].(map[string]any)
	if code != 200 || note == nil || note["cause"] != "bad_response" || note["cachedAt"] == "" {
		t.Fatalf("stale list = %d %v", code, list)
	}
	if rows, _ := list["packages"].([]any); len(rows) != 1 {
		t.Fatalf("rows = %v", list["packages"])
	}
	code, detail := marketGetJSON(t, base+"/market/packages/registry/kit?refresh=1")
	if dn, _ := detail["cache"].(map[string]any); code != 200 || dn == nil || detail["pinned"] != true {
		t.Fatalf("stale detail = %d %v", code, detail)
	}

	code, other := marketGetJSON(t, base+"/market/packages?q=never-asked")
	if code != http.StatusBadGateway || other["code"] != "market.bad_response" {
		t.Fatalf("an unseen query must still fail loudly: %d %v", code, other)
	}
}

func TestMarketInstallNeverReadsTheBrowseCache(t *testing.T) {
	_, _, base := pluginHome(t)
	down := realRegistryAt(t)
	if code, _ := marketGetJSON(t, base+"/market/packages/registry/kit"); code != 200 {
		t.Fatalf("detail = %d", code)
	}
	down.Store(true)
	resp, err := http.Post(base+"/market/plan", "application/json", strings.NewReader(`{"slug":"registry/kit"}`))
	if err != nil {
		t.Fatal(err)
	}
	if got := marketCode(t, resp); got != "market.bad_response" {
		t.Fatalf("plan code = %q, want the registry failure, not a cached detail", got)
	}
}
