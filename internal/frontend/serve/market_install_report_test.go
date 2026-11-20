package serve

import (
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"reasonix/internal/base/testenv"
	"reasonix/internal/contract/config"
)

func reportServer(t *testing.T) *Server {
	t.Helper()
	t.Setenv("REASONIX_HOME", testenv.TempDir(t))
	s := New(&pluginCtl{root: testenv.TempDir(t)}, NewBroadcaster(), config.ServeConfig{})
	srv := httptest.NewServer(operatorHandler(s))
	t.Cleanup(srv.Close)
	return s
}

func clearOptOuts(t *testing.T) {
	t.Helper()
	for _, k := range []string{"DO_NOT_TRACK", "REASONIX_TELEMETRY", "CI", "CONTINUOUS_INTEGRATION", "GITHUB_ACTIONS", "GITLAB_CI",
		"BUILDKITE", "CIRCLECI", "JENKINS_URL", "TEAMCITY_VERSION", "TF_BUILD"} {
		t.Setenv(k, "")
	}
}

// An install report goes out only when the host allows it, both desktop
// statistics switches are on, and nothing in the environment opts out.
func TestMarketInstallKeyFollowsTheStatisticsConsent(t *testing.T) {
	clearOptOuts(t)
	s := reportServer(t)
	req := httptest.NewRequest(http.MethodPost, "/market/install", nil)
	if s.marketInstallKey(req) != "" {
		t.Fatal("reported without the host grant")
	}
	s.AllowMarketInstallReport()
	key := s.marketInstallKey(req)
	if len(key) != 32 {
		t.Fatalf("key = %q with every switch on", key)
	}
	raw, err := os.ReadFile(filepath.Join(config.ReasonixHomeDir(), "market", "install-id"))
	if err != nil || key != strings.TrimSpace(string(raw)) {
		t.Fatalf("key is not the market's own stored id (%v)", err)
	}
	if _, err := os.Stat(filepath.Join(config.ReasonixHomeDir(), "cli-telemetry-install-id")); !os.IsNotExist(err) {
		t.Fatal("reporting created the usage-statistics id")
	}
	for _, off := range []string{"telemetry = false", "metrics = false"} {
		if err := os.WriteFile(config.UserConfigPath(), []byte("[desktop]\n"+off+"\n"), 0o600); err != nil {
			t.Fatal(err)
		}
		if s.marketInstallKey(req) != "" {
			t.Errorf("reported with %s", off)
		}
	}
	if err := os.Remove(config.UserConfigPath()); err != nil {
		t.Fatal(err)
	}
	t.Setenv("DO_NOT_TRACK", "1")
	if s.marketInstallKey(req) != "" {
		t.Fatal("reported under DO_NOT_TRACK")
	}
}
