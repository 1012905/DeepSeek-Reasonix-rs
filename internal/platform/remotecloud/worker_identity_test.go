package remotecloud

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"

	"reasonix/internal/base/testenv"
)

func TestRelayAddressAdmitsWSSAndOnlyLoopbackWS(t *testing.T) {
	for input, want := range map[string]string{
		"wss://relay.example.com":    "wss://relay.example.com",
		" wss://relay.example.com/ ": "wss://relay.example.com",
		"ws://127.0.0.1:8787":        "ws://127.0.0.1:8787",
	} {
		address, err := RelayAddress(input)
		if err != nil || address != want {
			t.Errorf("RelayAddress(%q) = %q, %v; want %q", input, address, err, want)
		}
	}
	for _, input := range []string{"", "https://relay.example.com", "ws://relay.example.com", "http://127.0.0.1:8787"} {
		if address, err := RelayAddress(input); err == nil {
			t.Errorf("RelayAddress(%q) = %q, want a refusal", input, address)
		}
	}
}

// recordingWorker is the Worker's REST surface without the room: enough to hold
// the registrar to its registration, pairing and revocation contract.
type recordingWorker struct {
	mu            sync.Mutex
	registrations int
	pairs         int
	revocations   int
	publicKey     string
	capabilities  []string
	pairAuth      string
}

func (w *recordingWorker) ServeHTTP(rw http.ResponseWriter, r *http.Request) {
	switch {
	case r.Method == http.MethodPost && r.URL.Path == "/v1/devices":
		var body struct {
			PublicKey    string   `json:"publicKey"`
			Capabilities []string `json:"capabilities"`
		}
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			http.Error(rw, "bad body", http.StatusBadRequest)
			return
		}
		w.mu.Lock()
		w.registrations++
		w.publicKey = body.PublicKey
		w.capabilities = body.Capabilities
		w.mu.Unlock()
		rw.Header().Set("Content-Type", "application/json")
		rw.WriteHeader(http.StatusCreated)
		_ = json.NewEncoder(rw).Encode(map[string]string{"deviceId": "dev-1", "deviceCredential": "secret-1"})
	case r.Method == http.MethodPost && r.URL.Path == "/v1/devices/dev-1/pair":
		if r.Header.Get("Authorization") != "Bearer secret-1" {
			http.Error(rw, "unauthorized", http.StatusUnauthorized)
			return
		}
		w.mu.Lock()
		w.pairs++
		w.pairAuth = r.Header.Get("Authorization")
		w.mu.Unlock()
		rw.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(rw).Encode(map[string]any{"code": "123456", "expiresIn": 300})
	case r.Method == http.MethodDelete && r.URL.Path == "/v1/devices/dev-1":
		if r.Header.Get("Authorization") != "Bearer secret-1" {
			http.Error(rw, "unauthorized", http.StatusUnauthorized)
			return
		}
		w.mu.Lock()
		w.revocations++
		w.mu.Unlock()
		rw.WriteHeader(http.StatusNoContent)
	default:
		http.NotFound(rw, r)
	}
}

func (w *recordingWorker) snapshot() (int, string, []string, int, string, int) {
	w.mu.Lock()
	defer w.mu.Unlock()
	return w.registrations, w.publicKey, w.capabilities, w.pairs, w.pairAuth, w.revocations
}

func TestWorkerRegistrarRegistersPairsAndRevokesTheSavedIdentity(t *testing.T) {
	t.Setenv("REASONIX_HOME", testenv.TempDir(t))
	t.Setenv("REASONIX_CREDENTIALS_STORE", "file")
	t.Setenv(identityCredentialKey, "")
	worker := &recordingWorker{}
	server := httptest.NewServer(worker)
	defer server.Close()
	relayURL := "ws" + strings.TrimPrefix(server.URL, "http")
	registrar, err := NewWorkerRegistrar(relayURL)
	if err != nil {
		t.Fatal(err)
	}
	host := NewRelay(registrar, nil, relayURL, "test")

	deviceID, err := host.EnsureDevice(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	saved, err := loadIdentity()
	if err != nil || saved == nil {
		t.Fatalf("loadIdentity err = %v, saved present = %v; want a saved identity", err, saved != nil)
	}
	registrations, publicKey, capabilities, _, _, _ := worker.snapshot()
	raw, err := base64.RawURLEncoding.DecodeString(publicKey)
	if err != nil || len(raw) != 32 || registrations != 1 {
		t.Fatalf("registration = %d keys of %d bytes, want one X25519 key", registrations, len(raw))
	}
	if len(capabilities) != 1 || capabilities[0] != "desktop" {
		t.Fatalf("capabilities = %v, want [desktop]", capabilities)
	}

	again, err := host.EnsureDevice(context.Background())
	if registrations, _, _, _, _, _ = worker.snapshot(); err != nil || again != deviceID || registrations != 1 {
		t.Fatalf("second EnsureDevice = %q, %v (registrations %d), want the saved identity reused",
			again, err, registrations)
	}

	pairing, err := registrar.Pair(context.Background())
	var pairs int
	var pairAuth string
	if _, _, _, pairs, pairAuth, _ = worker.snapshot(); err != nil || pairing.Code != "123456" || pairing.ExpiresIn != 300 || pairs != 1 {
		t.Fatalf("pairing = %+v, %v, calls %d", pairing, err, pairs)
	}
	if pairAuth != "Bearer secret-1" {
		t.Fatal("the pairing request did not carry the saved device credential")
	}

	if err := registrar.Revoke(context.Background()); err != nil {
		t.Fatalf("revoke = %v", err)
	}
	var revocations int
	if _, _, _, _, _, revocations = worker.snapshot(); revocations != 1 {
		t.Fatalf("revocation calls = %d, want one", revocations)
	}
	if _, err := registrar.Pair(context.Background()); !errors.Is(err, ErrNotRegistered) {
		t.Fatalf("pair after revoke = %v, want ErrNotRegistered", err)
	}
}

func TestEnsureDeviceNeedsARelayRegistrar(t *testing.T) {
	if _, err := New(nil, nil, "wss://relay.example.com", "test").EnsureDevice(context.Background()); err == nil {
		t.Fatal("an account-only host answered a relay registration")
	}
}
