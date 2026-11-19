package serve

import (
	"crypto/aes"
	"crypto/cipher"
	"crypto/ecdh"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/gorilla/websocket"
	"golang.org/x/crypto/hkdf"
)

const (
	relayTestDeviceID   = "device-1"
	relayTestCredential = "credential-1"
	relayTestConnection = "cccccccccccccccccccccccccccccccc"
	identityEnvKey      = "REASONIX_REMOTE_DEVICE_IDENTITY"
)

// fakeRelay is the Worker's REST surface and room in one handler: it registers,
// pairs, upgrades the device socket and forwards controller frames to it.
type fakeRelay struct {
	mu            sync.Mutex
	publicKey     string
	registrations int
	pairs         int
	revocations   int
	device        *websocket.Conn
	connected     chan struct{}
	frames        chan []byte
	closed        chan struct{}
	closeOnce     sync.Once
}

func newFakeRelay(t *testing.T) (*fakeRelay, *httptest.Server) {
	t.Helper()
	fake := &fakeRelay{
		connected: make(chan struct{}, 1),
		frames:    make(chan []byte, 8),
		closed:    make(chan struct{}),
	}
	mux := http.NewServeMux()
	mux.HandleFunc("POST /v1/devices", fake.register)
	mux.HandleFunc("POST /v1/devices/{id}/pair", fake.pair)
	mux.HandleFunc("GET /v1/devices/{id}/connect", fake.connect)
	mux.HandleFunc("DELETE /v1/devices/{id}", fake.revoke)
	server := httptest.NewServer(mux)
	t.Cleanup(server.Close)
	return fake, server
}

func (f *fakeRelay) register(w http.ResponseWriter, r *http.Request) {
	var body struct {
		PublicKey    string   `json:"publicKey"`
		Capabilities []string `json:"capabilities"`
	}
	if err := json.NewDecoder(r.Body).Decode(&body); err != nil || body.PublicKey == "" ||
		len(body.Capabilities) != 1 || body.Capabilities[0] != "desktop" {
		http.Error(w, "bad registration", http.StatusBadRequest)
		return
	}
	f.mu.Lock()
	f.publicKey = body.PublicKey
	f.registrations++
	f.mu.Unlock()
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(http.StatusCreated)
	_ = json.NewEncoder(w).Encode(map[string]string{
		"deviceId": relayTestDeviceID, "deviceCredential": relayTestCredential,
	})
}

func (f *fakeRelay) authorized(r *http.Request) bool {
	return r.PathValue("id") == relayTestDeviceID &&
		r.Header.Get("Authorization") == "Bearer "+relayTestCredential
}

func (f *fakeRelay) pair(w http.ResponseWriter, r *http.Request) {
	if !f.authorized(r) {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	f.mu.Lock()
	f.pairs++
	f.mu.Unlock()
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(map[string]any{"code": "123456", "expiresIn": 300})
}

func (f *fakeRelay) revoke(w http.ResponseWriter, r *http.Request) {
	if !f.authorized(r) {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	f.mu.Lock()
	f.revocations++
	f.mu.Unlock()
	w.WriteHeader(http.StatusNoContent)
}

func (f *fakeRelay) connect(w http.ResponseWriter, r *http.Request) {
	if !f.authorized(r) {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	conn, err := (&websocket.Upgrader{}).Upgrade(w, r, nil)
	if err != nil {
		return
	}
	f.mu.Lock()
	f.device = conn
	f.mu.Unlock()
	_ = conn.WriteMessage(websocket.TextMessage, []byte(
		`{"type":"relay_hello","controllers":[],"heartbeat":"","heartbeatMs":25000}`))
	select {
	case f.connected <- struct{}{}:
	default:
	}
	for {
		_, raw, err := conn.ReadMessage()
		if err != nil {
			f.closeOnce.Do(func() { close(f.closed) })
			return
		}
		select {
		case f.frames <- raw:
		default:
		}
	}
}

func (f *fakeRelay) devicePublicKey() string {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.publicKey
}

func (f *fakeRelay) registrationCount() int {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.registrations
}

func (f *fakeRelay) pairCount() int {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.pairs
}

func (f *fakeRelay) sendController(t *testing.T, payload []byte) {
	t.Helper()
	f.mu.Lock()
	conn := f.device
	f.mu.Unlock()
	if conn == nil {
		t.Fatal("the device socket is not connected")
	}
	raw, err := json.Marshal(map[string]any{
		"type": "controller_message", "connectionId": relayTestConnection,
		"scopes": []string{"desktop"}, "payload": string(payload),
	})
	if err != nil {
		t.Fatal(err)
	}
	if err := conn.WriteMessage(websocket.TextMessage, raw); err != nil {
		t.Fatal(err)
	}
}

func relayTestFront(t *testing.T) (*fakeRelay, *Server, *httptest.Server) {
	t.Helper()
	fake, worker := newFakeRelay(t)
	t.Setenv(identityEnvKey, "")
	t.Setenv(relayURLEnv, "ws"+strings.TrimPrefix(worker.URL, "http"))
	s := noProviderServer(t)
	front := httptest.NewServer(operatorHandler(s))
	t.Cleanup(front.Close)
	t.Cleanup(s.relay.disable)
	return fake, s, front
}

func relayCall(t *testing.T, method, url string) (int, []byte) {
	t.Helper()
	request, err := http.NewRequest(method, url, nil)
	if err != nil {
		t.Fatal(err)
	}
	if method == http.MethodPost {
		request.Header.Set("Content-Type", "application/json")
	}
	response, err := http.DefaultClient.Do(request)
	if err != nil {
		t.Fatal(err)
	}
	defer response.Body.Close()
	raw, err := io.ReadAll(response.Body)
	if err != nil {
		t.Fatal(err)
	}
	return response.StatusCode, raw
}

func TestRelayEnablePersistsDeviceAndPairPassesWorkerCode(t *testing.T) {
	fake, _, front := relayTestFront(t)

	var enabled struct {
		DeviceID string `json:"deviceId"`
	}
	if status, raw := relayCall(t, http.MethodPost, front.URL+"/remote/relay/enable"); status != http.StatusOK {
		t.Fatalf("enable = %d %s", status, raw)
	} else if err := json.Unmarshal(raw, &enabled); err != nil || enabled.DeviceID != relayTestDeviceID {
		t.Fatalf("enable body = %s, want device %q", raw, relayTestDeviceID)
	}
	if fake.registrationCount() != 1 || fake.devicePublicKey() == "" {
		t.Fatalf("registrations = %d, public key present = %v; want one registration with a key",
			fake.registrationCount(), fake.devicePublicKey() != "")
	}

	var state relayState
	status, raw := relayCall(t, http.MethodGet, front.URL+"/remote/relay")
	if status != http.StatusOK || json.Unmarshal(raw, &state) != nil || !state.Enabled || state.DeviceID != relayTestDeviceID {
		t.Fatalf("status = %d %s, want enabled device %q", status, raw, relayTestDeviceID)
	}

	var pairing struct {
		Code      string `json:"code"`
		ExpiresIn int    `json:"expiresIn"`
	}
	status, raw = relayCall(t, http.MethodPost, front.URL+"/remote/relay/pair")
	if status != http.StatusOK || json.Unmarshal(raw, &pairing) != nil ||
		pairing.Code != "123456" || pairing.ExpiresIn != 300 || fake.pairCount() != 1 {
		t.Fatalf("pair = %d %s, calls = %d", status, raw, fake.pairCount())
	}

	if status, raw := relayCall(t, http.MethodPost, front.URL+"/remote/relay/disable"); status != http.StatusNoContent {
		t.Fatalf("disable = %d %s", status, raw)
	}
	if status, raw := relayCall(t, http.MethodPost, front.URL+"/remote/relay/enable"); status != http.StatusOK {
		t.Fatalf("re-enable = %d %s", status, raw)
	} else if err := json.Unmarshal(raw, &enabled); err != nil || enabled.DeviceID != relayTestDeviceID {
		t.Fatalf("re-enable body = %s, want the persisted device", raw)
	}
	if fake.registrationCount() != 1 {
		t.Fatalf("re-enable registered %d devices, want the persisted identity reused", fake.registrationCount())
	}
}

func TestRelayHostConnectsThroughWorkerAndDisableCloses(t *testing.T) {
	fake, _, front := relayTestFront(t)

	if status, raw := relayCall(t, http.MethodPost, front.URL+"/remote/relay/enable"); status != http.StatusOK {
		t.Fatalf("enable = %d %s", status, raw)
	}
	select {
	case <-fake.connected:
	case <-time.After(5 * time.Second):
		t.Fatal("the host never dialed the relay")
	}

	controller, err := ecdh.X25519().GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	salt := make([]byte, 16)
	if _, err := rand.Read(salt); err != nil {
		t.Fatal(err)
	}
	greeting, err := json.Marshal(map[string]any{
		"v": 1, "type": "hello",
		"publicKey": base64.RawURLEncoding.EncodeToString(controller.PublicKey().Bytes()),
		"salt":      base64.RawURLEncoding.EncodeToString(salt),
	})
	if err != nil {
		t.Fatal(err)
	}
	fake.sendController(t, greeting)

	var raw []byte
	select {
	case raw = <-fake.frames:
	case <-time.After(5 * time.Second):
		t.Fatal("the host answered no frame")
	}
	var directed struct {
		To      string `json:"to"`
		Payload string `json:"payload"`
	}
	if err := json.Unmarshal(raw, &directed); err != nil || directed.To != relayTestConnection {
		t.Fatalf("directed frame = %s, want to %q", raw, relayTestConnection)
	}
	seal := newPeerSeal(t, controller, fake.devicePublicKey(), salt)
	ready := seal.open(t, directed.Payload)
	if ready["type"] != "ready" || ready["deviceId"] != relayTestDeviceID {
		t.Fatalf("ready = %+v, want the sealed ready frame", ready)
	}

	if status, raw := relayCall(t, http.MethodPost, front.URL+"/remote/relay/disable"); status != http.StatusNoContent {
		t.Fatalf("disable = %d %s", status, raw)
	}
	select {
	case <-fake.closed:
	case <-time.After(5 * time.Second):
		t.Fatal("disable left the relay socket open")
	}
}

// peerSeal is the controller half of the relay protocol, held separately from
// the host's implementation so the test proves the cross-machine contract.
type peerSeal struct {
	aead cipher.AEAD
	aad  []byte
}

func newPeerSeal(t *testing.T, private *ecdh.PrivateKey, publicKey string, salt []byte) peerSeal {
	t.Helper()
	raw, err := base64.RawURLEncoding.DecodeString(publicKey)
	if err != nil {
		t.Fatal(err)
	}
	peer, err := ecdh.X25519().NewPublicKey(raw)
	if err != nil {
		t.Fatal(err)
	}
	secret, err := private.ECDH(peer)
	if err != nil {
		t.Fatal(err)
	}
	info := "reasonix-remote-v1|" + relayTestDeviceID
	key := make([]byte, 32)
	if _, err := io.ReadFull(hkdf.New(sha256.New, secret, salt, []byte(info)), key); err != nil {
		t.Fatal(err)
	}
	block, err := aes.NewCipher(key)
	if err != nil {
		t.Fatal(err)
	}
	aead, err := cipher.NewGCM(block)
	if err != nil {
		t.Fatal(err)
	}
	return peerSeal{aead: aead, aad: []byte(info)}
}

func (p peerSeal) open(t *testing.T, payload string) map[string]any {
	t.Helper()
	var wire struct {
		Version    int    `json:"v"`
		Nonce      string `json:"nonce"`
		Ciphertext string `json:"ciphertext"`
	}
	if err := json.Unmarshal([]byte(payload), &wire); err != nil {
		t.Fatal(err)
	}
	nonce, err := base64.RawURLEncoding.DecodeString(wire.Nonce)
	if err != nil {
		t.Fatal(err)
	}
	ciphertext, err := base64.RawURLEncoding.DecodeString(wire.Ciphertext)
	if err != nil {
		t.Fatal(err)
	}
	plain, err := p.aead.Open(nil, nonce, ciphertext, p.aad)
	if err != nil {
		t.Fatal(err)
	}
	var body map[string]any
	if err := json.Unmarshal(plain, &body); err != nil {
		t.Fatal(err)
	}
	return body
}
