package remotecloud

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
	"time"

	"reasonix/internal/contract/provider"
)

// Registration is the device identity a relay Worker answers a registration with.
type Registration struct {
	DeviceID   string
	Credential string
}

// Registrar is how a relay host obtains and gives up its device identity.
type Registrar interface {
	Register(ctx context.Context, publicKey string, capabilities []string) (Registration, error)
	Revoke(ctx context.Context) error
}

// Pairing is the one-time code a controller redeems to reach this machine.
type Pairing struct {
	Code      string `json:"code"`
	ExpiresIn int    `json:"expiresIn"`
}

// WorkerRegistrar talks to the relay Worker's REST surface. The origin is the
// relay's own, mapped to https, so one address configures both.
type WorkerRegistrar struct {
	base   string
	client *http.Client
}

// RelayAddress normalises a relay endpoint and refuses one a device credential
// must not cross in the clear: wss:// anywhere, ws:// only to this machine.
func RelayAddress(raw string) (string, error) {
	address := strings.TrimRight(strings.TrimSpace(raw), "/")
	if address == "" || !relaySecure(address) {
		return "", errRelayInsecure
	}
	return address, nil
}

// NewWorkerRegistrar derives the REST origin from a relay address.
func NewWorkerRegistrar(relayURL string) (*WorkerRegistrar, error) {
	address, err := RelayAddress(relayURL)
	if err != nil {
		return nil, err
	}
	parsed, err := url.Parse(address)
	if err != nil || parsed.Host == "" {
		return nil, errRelayInsecure
	}
	scheme := "https"
	if strings.EqualFold(parsed.Scheme, "ws") {
		scheme = "http"
	}
	return &WorkerRegistrar{base: scheme + "://" + parsed.Host, client: &http.Client{Timeout: 30 * time.Second}}, nil
}

func (r *WorkerRegistrar) Register(ctx context.Context, publicKey string, capabilities []string) (Registration, error) {
	var response struct {
		DeviceID   string `json:"deviceId"`
		Credential string `json:"deviceCredential"`
	}
	body := map[string]any{"publicKey": publicKey, "capabilities": capabilities}
	if err := r.call(ctx, http.MethodPost, "/v1/devices", "", body, &response); err != nil {
		return Registration{}, err
	}
	if response.DeviceID == "" || response.Credential == "" {
		return Registration{}, errors.New("remote cloud: the relay answered an incomplete registration")
	}
	return Registration{DeviceID: response.DeviceID, Credential: response.Credential}, nil
}

// Pair asks the Worker for a one-time code bound to the saved identity.
func (r *WorkerRegistrar) Pair(ctx context.Context) (Pairing, error) {
	saved, err := relayIdentity()
	if err != nil {
		return Pairing{}, err
	}
	var pairing Pairing
	path := "/v1/devices/" + url.PathEscape(saved.DeviceID) + "/pair"
	if err := r.call(ctx, http.MethodPost, path, saved.DeviceCredential, nil, &pairing); err != nil {
		return Pairing{}, err
	}
	if pairing.Code == "" {
		return Pairing{}, errors.New("remote cloud: the relay answered no pairing code")
	}
	return pairing, nil
}

// Revoke deletes the saved identity at the Worker and on this machine.
func (r *WorkerRegistrar) Revoke(ctx context.Context) error {
	saved, err := relayIdentity()
	if err != nil {
		return err
	}
	path := "/v1/devices/" + url.PathEscape(saved.DeviceID)
	if err := r.call(ctx, http.MethodDelete, path, saved.DeviceCredential, nil, nil); err != nil {
		return err
	}
	return clearIdentity()
}

func relayIdentity() (*identity, error) {
	saved, err := loadIdentity()
	if err != nil {
		return nil, err
	}
	if saved == nil || saved.OwnerID != 0 {
		return nil, ErrNotRegistered
	}
	return saved, nil
}

func (r *WorkerRegistrar) call(ctx context.Context, method, path, credential string, body, out any) error {
	var reader io.Reader
	if body != nil {
		encoded, err := json.Marshal(body)
		if err != nil {
			return err
		}
		reader = bytes.NewReader(encoded)
	}
	request, err := http.NewRequestWithContext(ctx, method, r.base+path, reader)
	if err != nil {
		return err
	}
	if body != nil {
		request.Header.Set("Content-Type", "application/json")
	}
	if credential != "" {
		request.Header.Set("Authorization", "Bearer "+credential)
	}
	request.Header.Set("User-Agent", provider.ClientUserAgent())
	response, err := r.client.Do(request)
	if err != nil {
		return err
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return workerRefusal(path, response)
	}
	defer response.Body.Close()
	if out == nil {
		_, _ = io.Copy(io.Discard, io.LimitReader(response.Body, 64<<10))
		return nil
	}
	if err := json.NewDecoder(io.LimitReader(response.Body, 64<<10)).Decode(out); err != nil {
		return fmt.Errorf("remote cloud: the relay answered unreadable JSON: %w", err)
	}
	return nil
}

// workerRefusal reports the Worker's own error code and status; the body is
// parsed, never echoed, so a credential has no path into a log.
func workerRefusal(path string, response *http.Response) error {
	defer response.Body.Close()
	var payload struct {
		Error struct {
			Code string `json:"code"`
		} `json:"error"`
	}
	_ = json.NewDecoder(io.LimitReader(response.Body, 8<<10)).Decode(&payload)
	detail := payload.Error.Code
	if detail == "" {
		detail = http.StatusText(response.StatusCode)
	}
	return fmt.Errorf("remote cloud: %s answered %d %s", path, response.StatusCode, detail)
}
