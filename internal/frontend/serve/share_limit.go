package serve

import (
	"net"
	"net/http"
	"sync"
	"time"
)

// Pairing codes are six digits, so a door reachable from the internet needs a
// budget: without one, every code is a thousand tries from guessed. The limits
// are per client and global, both per rolling window.
const (
	pairAttemptsPerClient = 8
	pairAttemptsGlobal    = 60
	pairWindow            = 10 * time.Minute
)

type pairLimiter struct {
	mu      sync.Mutex
	now     func() time.Time
	clients map[string][]time.Time
	all     []time.Time
}

func newPairLimiter() *pairLimiter {
	return &pairLimiter{now: time.Now, clients: map[string][]time.Time{}}
}

// allow records one attempt for ip and reports whether it fits the budget.
func (l *pairLimiter) allow(ip string) bool {
	t := l.now()
	cutoff := t.Add(-pairWindow)
	l.mu.Lock()
	defer l.mu.Unlock()
	l.all = pruneBefore(l.all, cutoff)
	if len(l.all) >= pairAttemptsGlobal {
		return false
	}
	seen := pruneBefore(l.clients[ip], cutoff)
	if len(seen) >= pairAttemptsPerClient {
		l.clients[ip] = seen
		return false
	}
	seen = append(seen, t)
	l.clients[ip] = seen
	l.all = append(l.all, t)
	return true
}

func pruneBefore(times []time.Time, cutoff time.Time) []time.Time {
	kept := times[:0]
	for _, t := range times {
		if t.After(cutoff) {
			kept = append(kept, t)
		}
	}
	return kept
}

func remoteIP(r *http.Request) string {
	host, _, err := net.SplitHostPort(r.RemoteAddr)
	if err != nil {
		return r.RemoteAddr
	}
	return host
}
