package serve

import (
	"context"
	"errors"
	"net/http"
	"os"
	"strings"
	"sync"
	"time"

	"reasonix/internal/contract/provider"
	"reasonix/internal/platform/remotecloud"
)

const relayURLEnv = "REASONIX_RELAY_URL"

// relayManager holds this pane's relay presence. Nothing dials until the window
// enables it, and disabling stops the host without forgetting the identity.
type relayManager struct {
	mu       sync.Mutex
	host     *remotecloud.Host
	stop     context.CancelFunc
	done     chan struct{}
	deviceID string
}

type relayState struct {
	Enabled  bool   `json:"enabled"`
	Online   bool   `json:"online"`
	DeviceID string `json:"deviceId"`
}

// registerRelayRoutes adds where this machine stands on the relay and the three
// acts the window takes on it: enable, disable, and pair.
func (s *Server) registerRelayRoutes(mux *http.ServeMux) {
	mux.HandleFunc("GET /remote/relay", s.relayStatus)
	mux.HandleFunc("POST /remote/relay/enable", s.relayEnable)
	mux.HandleFunc("POST /remote/relay/disable", s.relayDisable)
	mux.HandleFunc("POST /remote/relay/pair", s.relayPair)
}

// relayHostOnly refuses a paired device: the relay is this window's own door,
// and a device already reached the pane through it.
func (s *Server) relayHostOnly(w http.ResponseWriter, r *http.Request) bool {
	if hostReach(r) {
		return true
	}
	refuse(w, http.StatusNotFound, codeDeviceHostOnly, "a paired device cannot use this", nil)
	return false
}

func (s *Server) relayStatus(w http.ResponseWriter, r *http.Request) {
	if !s.relayHostOnly(w, r) {
		return
	}
	writeJSON(w, s.relay.status())
}

func (s *Server) relayEnable(w http.ResponseWriter, r *http.Request) {
	if !s.relayHostOnly(w, r) {
		return
	}
	address, ok := relayEndpoint(w)
	if !ok {
		return
	}
	deviceID, err := s.relay.enable(r.Context(), address, provider.ClientVersion())
	if err != nil {
		refuse(w, http.StatusBadGateway, "internal.failed", "the relay could not register this machine",
			map[string]any{"detail": err.Error()})
		return
	}
	writeJSON(w, map[string]string{"deviceId": deviceID})
}

func (s *Server) relayDisable(w http.ResponseWriter, r *http.Request) {
	if !s.relayHostOnly(w, r) {
		return
	}
	s.relay.disable()
	w.WriteHeader(http.StatusNoContent)
}

func (s *Server) relayPair(w http.ResponseWriter, r *http.Request) {
	if !s.relayHostOnly(w, r) {
		return
	}
	address, ok := relayEndpoint(w)
	if !ok {
		return
	}
	registrar, err := remotecloud.NewWorkerRegistrar(address)
	if err != nil {
		refuse(w, http.StatusBadRequest, codeBadValue, err.Error(), map[string]any{"field": relayURLEnv})
		return
	}
	pairing, err := registrar.Pair(r.Context())
	switch {
	case errors.Is(err, remotecloud.ErrNotRegistered):
		refuse(w, http.StatusConflict, "remote.disabled", "this machine has not been enabled on the relay", nil)
	case err != nil:
		refuse(w, http.StatusBadGateway, "internal.failed", "the relay could not issue a pairing code",
			map[string]any{"detail": err.Error()})
	default:
		writeJSON(w, pairing)
	}
}

// relayEndpoint reads the relay this build was pointed at. Unset and unsafe are
// two answers, so a person is told which one to fix.
func relayEndpoint(w http.ResponseWriter) (string, bool) {
	raw := strings.TrimSpace(os.Getenv(relayURLEnv))
	if raw == "" {
		missingField(w, relayURLEnv)
		return "", false
	}
	address, err := remotecloud.RelayAddress(raw)
	if err != nil {
		refuse(w, http.StatusBadRequest, codeBadValue, err.Error(), map[string]any{"field": relayURLEnv})
		return "", false
	}
	return address, true
}

func (m *relayManager) status() relayState {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.host == nil {
		return relayState{}
	}
	live := m.host.Status()
	deviceID := live.DeviceID
	if deviceID == "" {
		deviceID = m.deviceID
	}
	return relayState{Enabled: true, Online: live.Online, DeviceID: deviceID}
}

func (m *relayManager) enable(ctx context.Context, relayURL, version string) (string, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.host != nil {
		return m.deviceID, nil
	}
	registrar, err := remotecloud.NewWorkerRegistrar(relayURL)
	if err != nil {
		return "", err
	}
	host := remotecloud.NewRelay(registrar, nil, relayURL, version)
	deviceID, err := host.EnsureDevice(ctx)
	if err != nil {
		return "", err
	}
	runCtx, stop := context.WithCancel(context.Background())
	done := make(chan struct{})
	m.host, m.stop, m.done, m.deviceID = host, stop, done, deviceID
	go func() {
		defer close(done)
		host.Run(runCtx)
	}()
	return deviceID, nil
}

// disable stops the host and waits for its connection to close, so the answer
// means the relay saw the device leave rather than a cancel that outlived it.
func (m *relayManager) disable() {
	m.mu.Lock()
	stop, done := m.stop, m.done
	m.host, m.stop, m.done, m.deviceID = nil, nil, nil, ""
	m.mu.Unlock()
	if stop == nil {
		return
	}
	stop()
	timer := time.NewTimer(5 * time.Second)
	defer timer.Stop()
	select {
	case <-done:
	case <-timer.C:
	}
}
