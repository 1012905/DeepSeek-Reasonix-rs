package serve

import (
	"crypto/tls"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"reasonix/internal/base/testenv"
)

func withPublicAddresses(t *testing.T, list ...ShareAddress) {
	t.Helper()
	prev := publicAddresses
	publicAddresses = func() []ShareAddress { return list }
	t.Cleanup(func() { publicAddresses = prev })
}

func TestOpenPublicRefusesAPrivateAddress(t *testing.T) {
	withPublicAddresses(t, ShareAddress{IP: "192.168.1.20", Kind: AddressLAN})
	share := NewDeviceShare(nil)
	share.Attach(http.NotFoundHandler())
	if _, err := share.OpenPublic("192.168.1.20"); err == nil {
		t.Fatal("a LAN address opened the public door")
	}
}

func TestOpenPublicServesItsPageOverHTTPS(t *testing.T) {
	withPublicAddresses(t, ShareAddress{IP: "127.0.0.1", Kind: AddressPublic})
	dir := testenv.TempDir(t)
	prev := shareCertPath
	shareCertPath = func() (string, string) {
		return dir + "/cert.pem", dir + "/key.pem"
	}
	t.Cleanup(func() { shareCertPath = prev })

	share := NewDeviceShare(nil)
	share.Attach(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write([]byte("ok"))
	}))
	st, err := share.OpenPublic("127.0.0.1")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(share.Close)
	if !st.Open || !st.Public || !strings.HasPrefix(st.Origin, "https://") {
		t.Fatalf("status = %+v, want an open public door", st)
	}
	if len(st.CertFingerprint) != 64 {
		t.Fatalf("fingerprint = %q, want 64 hex chars", st.CertFingerprint)
	}

	// The certificate is minted for this test's loopback IP with a throwaway key;
	// trusting this one client is the point of the assertion.
	client := &http.Client{Transport: &http.Transport{TLSClientConfig: &tls.Config{InsecureSkipVerify: true}}, Timeout: 5 * time.Second}
	res, err := client.Get(st.Origin + "/")
	if err != nil {
		t.Fatal(err)
	}
	defer res.Body.Close()
	if res.StatusCode != http.StatusOK {
		t.Fatalf("GET %s = %d", st.Origin, res.StatusCode)
	}
}

func TestGateRefusesAPublicDoorOnHTTP(t *testing.T) {
	gate := NewDeviceGate(http.NotFoundHandler(), DeviceGateOptions{
		Registry: NewDeviceRegistry(),
		Origin:   "http://127.0.0.1:4321",
		Public:   true,
	})
	rec := httptest.NewRecorder()
	gate.ServeHTTP(rec, httptest.NewRequest(http.MethodGet, "http://127.0.0.1:4321/", nil))
	if rec.Code != http.StatusInternalServerError {
		t.Fatalf("status = %d, want a refused misconfiguration", rec.Code)
	}
	if !strings.Contains(rec.Body.String(), codeDeviceMisconfig) {
		t.Fatalf("body = %s, want %s", rec.Body.String(), codeDeviceMisconfig)
	}
}

func TestPairLimiterSpendsItsBudget(t *testing.T) {
	now := time.Unix(1_700_000_000, 0)
	limiter := newPairLimiter()
	limiter.now = func() time.Time { return now }

	for range pairAttemptsPerClient {
		if !limiter.allow("203.0.113.9") {
			t.Fatal("an attempt inside the budget was refused")
		}
	}
	if limiter.allow("203.0.113.9") {
		t.Fatal("an attempt past the per-client budget was allowed")
	}
	if !limiter.allow("203.0.113.10") {
		t.Fatal("another client was refused its own budget")
	}

	now = now.Add(pairWindow + time.Minute)
	if !limiter.allow("203.0.113.9") {
		t.Fatal("the budget did not recover after the window")
	}
}
