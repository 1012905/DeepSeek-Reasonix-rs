package registry

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"net/url"
	"runtime"
	"strings"
	"sync"
	"testing"

	"aead.dev/minisign"
)

type servedFile struct {
	path string
	body string
}

type indexFixture struct {
	srv      *httptest.Server
	index    []byte
	sig      []byte
	files    map[string]string
	pubText  []byte
	mu       sync.Mutex
	etag     string
	indexHit int
	sigHit   int
}

func newFixture(t *testing.T, entries []any, files []servedFile) *indexFixture {
	t.Helper()
	doc, err := json.MarshalIndent(map[string]any{"schemaVersion": 1, "entries": entries}, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	pub, priv, err := minisign.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	pubText, err := pub.MarshalText()
	if err != nil {
		t.Fatal(err)
	}
	f := &indexFixture{index: doc, sig: minisign.Sign(priv, doc), pubText: pubText, etag: `"registry-v1"`, files: map[string]string{}}
	for _, file := range files {
		f.files["/prefix/"+file.path] = file.body
	}
	f.srv = httptest.NewServer(http.HandlerFunc(f.serve))
	t.Cleanup(f.srv.Close)
	return f
}

func (f *indexFixture) serve(w http.ResponseWriter, r *http.Request) {
	f.mu.Lock()
	defer f.mu.Unlock()
	switch r.URL.Path {
	case "/prefix/index.json":
		f.indexHit++
		if r.Header.Get("If-None-Match") == f.etag {
			w.WriteHeader(http.StatusNotModified)
			return
		}
		w.Header().Set("ETag", f.etag)
		_, _ = w.Write(f.index)
	case "/prefix/index.json.minisig":
		f.sigHit++
		_, _ = w.Write(f.sig)
	default:
		if body, ok := f.files[r.URL.Path]; ok {
			_, _ = w.Write([]byte(body))
			return
		}
		http.NotFound(w, r)
	}
}

func (f *indexFixture) client() *Client {
	u := f.srv.URL + "/prefix/index.json"
	c := NewClientWithOrigin(f.srv.Client(), u, u, "2.0.0")
	c.Verify = func(data, sig []byte) error {
		var key minisign.PublicKey
		if err := key.UnmarshalText(f.pubText); err != nil {
			return err
		}
		if !minisign.Verify(key, data, sig) {
			return errors.New("signature mismatch")
		}
		return nil
	}
	return c
}

func (f *indexFixture) counts() (int, int) {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.indexHit, f.sigHit
}

func digest(s string) string {
	sum := sha256.Sum256([]byte(s))
	return hex.EncodeToString(sum[:])
}

func skillFile(id, body string) servedFile {
	return servedFile{path: "skills/" + id + "/SKILL.md", body: body}
}

func entry(id, kind, version, minimum string, platforms []string, revoked bool, files ...servedFile) map[string]any {
	var listing strings.Builder
	rows := make([]any, 0, len(files))
	for _, file := range files {
		sum := digest(file.body)
		rows = append(rows, map[string]any{"path": file.path, "sha256": sum, "size": len(file.body)})
		listing.WriteString(sum + "\t" + file.path + "\n")
	}
	return map[string]any{
		"id": id, "kind": kind, "version": version,
		"url": "skills/" + id + "/", "sha256": digest(listing.String()),
		"files": rows, "minVersion": minimum,
		"compat":  map[string]any{"platforms": platforms, "frontends": []string{"studio"}},
		"license": "MIT", "revoked": revoked,
	}
}

func TestBadSignatureIsRejectedBeforeParsing(t *testing.T) {
	f := newFixture(t, []any{entry("kit", "skills", "1.0.0", "0.1.0", []string{"any"}, false)}, nil)
	f.mu.Lock()
	f.index = []byte(`{ this is not json`)
	f.mu.Unlock()
	if _, err := f.client().List(context.Background()); !errors.Is(err, ErrBadSignature) {
		t.Fatalf("err = %v, want ErrBadSignature (parsing must come after verification)", err)
	}
}

func TestETagRevalidationReusesTheVerifiedIndex(t *testing.T) {
	files := []servedFile{skillFile("kit", "---\nname: kit\ndescription: d\n---\nbody")}
	f := newFixture(t, []any{entry("kit", "skills", "1.0.0", "0.1.0", []string{"any"}, false, files...)}, files)
	c := f.client()
	first, err := c.List(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	second, err := c.List(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(first) != 1 || len(second) != 1 || first[0].ID != second[0].ID {
		t.Fatalf("first = %+v second = %+v", first, second)
	}
	if hits, sigs := f.counts(); hits != 2 || sigs != 1 {
		t.Fatalf("index hits = %d sig hits = %d, want 2 and 1", hits, sigs)
	}
}

func TestFailedRevalidationDoesNotServeStaleBytes(t *testing.T) {
	files := []servedFile{skillFile("kit", "body")}
	f := newFixture(t, []any{entry("kit", "skills", "1.0.0", "0.1.0", []string{"any"}, false, files...)}, files)
	c := f.client()
	if _, err := c.List(context.Background()); err != nil {
		t.Fatal(err)
	}
	f.srv.Close()
	if _, err := c.List(context.Background()); !errors.Is(err, ErrUnreachable) {
		t.Fatalf("err = %v, want the revalidation failure", err)
	}
}

func TestListFiltersRevokedTooNewAndWrongPlatform(t *testing.T) {
	other := "windows"
	if runtime.GOOS == "windows" {
		other = "linux"
	}
	files := []servedFile{skillFile("kit", "body")}
	entries := []any{
		entry("good", "skills", "1.0.0", "1.0.0", []string{"any"}, false, files...),
		entry("gone", "skills", "1.0.0", "0.1.0", []string{"any"}, true, files...),
		entry("future", "skills", "2.0.0", "3.0.0", []string{"any"}, false, files...),
		entry("elsewhere", "skills", "1.0.0", "0.1.0", []string{other}, false, files...),
	}
	f := newFixture(t, entries, files)
	list, err := f.client().List(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(list) != 1 || list[0].ID != "good" {
		t.Fatalf("list = %+v, want only good", list)
	}
}

func TestFetchVerifiesEveryDeclaredDigest(t *testing.T) {
	good := []servedFile{skillFile("kit", "verified body")}
	f := newFixture(t, []any{entry("kit", "skills", "1.0.0", "0.1.0", []string{"any"}, false, good...)}, good)
	c := f.client()
	resolved, err := c.Resolve(context.Background(), "kit", "")
	if err != nil {
		t.Fatal(err)
	}
	content, err := c.Fetch(context.Background(), resolved)
	if err != nil {
		t.Fatal(err)
	}
	if len(content.Files) != 1 || content.Files[0].URL != f.srv.URL+"/prefix/skills/kit/SKILL.md" {
		t.Fatalf("content = %+v", content)
	}
	f.mu.Lock()
	f.files["/prefix/skills/kit/SKILL.md"] = "tampered body"
	f.mu.Unlock()
	if _, err := c.Fetch(context.Background(), resolved); !errors.Is(err, ErrBadResponse) || !strings.Contains(err.Error(), "sha256 mismatch") {
		t.Fatalf("tampered content: err = %v", err)
	}
}

func TestResolveRejectsAnUnknownEntry(t *testing.T) {
	f := newFixture(t, []any{}, nil)
	if _, err := f.client().Resolve(context.Background(), "missing", "1.0.0"); !errors.Is(err, ErrNotFound) {
		t.Fatalf("err = %v, want ErrNotFound", err)
	}
}

func TestDefaultClientUsesTheDeclaredIndexAndContentOrigins(t *testing.T) {
	c := NewClient(nil)
	if c.IndexURL() != RegistryIndexURL {
		t.Fatalf("index = %q", c.IndexURL())
	}
	got, err := c.SourceURL("skills/kit/SKILL.md")
	want := RegistryContentURL + "skills/kit/SKILL.md"
	if err != nil || got != want {
		t.Fatalf("source = %q err = %v, want %q", got, err, want)
	}
	if _, err := c.SourceURL("https://other.test/kit"); !errors.Is(err, ErrBadResponse) {
		t.Fatalf("foreign origin: err = %v", err)
	}
}

func TestSourceURLStaysOnTheIndexOrigin(t *testing.T) {
	c := NewClientWithOrigin(nil, "https://example.test/prefix/index.json", "https://example.test/prefix/index.json", "1.0.0")
	if got, err := c.SourceURL("skills/kit/SKILL.md"); err != nil || got != "https://example.test/prefix/skills/kit/SKILL.md" {
		t.Fatalf("source = %q err = %v", got, err)
	}
	if _, err := c.SourceURL("https://other.test/kit"); !errors.Is(err, ErrBadResponse) {
		t.Fatalf("foreign origin: err = %v", err)
	}
}

func TestFixtureParsesItsOwnIndex(t *testing.T) {
	files := []servedFile{skillFile("kit", "body")}
	f := newFixture(t, []any{entry("kit", "skills", "1.0.0", "0.1.0", []string{"any"}, false, files...)}, files)
	u, _ := url.Parse(f.srv.URL + "/prefix/index.json")
	if u.Path != "/prefix/index.json" {
		t.Fatalf("fixture URL = %s", u)
	}
	if _, err := parseIndex(f.index); err != nil {
		t.Fatalf("fixture index is invalid: %v", err)
	}
}
