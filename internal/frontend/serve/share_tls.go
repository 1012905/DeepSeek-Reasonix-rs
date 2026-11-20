package serve

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/sha256"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/hex"
	"encoding/pem"
	"fmt"
	"math/big"
	"net"
	"net/netip"
	"os"
	"path/filepath"
	"time"

	"reasonix/internal/contract/config"
)

// shareCertPath is where the public door's self-signed certificate lives. It is
// generated once per install and reused; the private half never leaves this
// directory.
var shareCertPath = func() (cert, key string) {
	home := filepath.Clean(config.ReasonixHomeDir())
	if home == "" {
		return "", ""
	}
	dir := filepath.Join(home, "state")
	return filepath.Join(dir, "share-tls.pem"), filepath.Join(dir, "share-tls-key.pem")
}

// shareCertificate returns a certificate for the address the public door binds,
// creating one on first use or when the stored pair no longer names that
// address. The fingerprint is the SHA-256 of the DER, so the window can show
// the same identity a browser shows.
func shareCertificate(ip netip.Addr) (tls.Certificate, string, error) {
	certPath, keyPath := shareCertPath()
	if certPath == "" || keyPath == "" {
		return tls.Certificate{}, "", fmt.Errorf("no state directory for the share certificate")
	}
	if cert, fp, ok := loadShareCertificate(certPath, keyPath, ip); ok {
		return cert, fp, nil
	}
	cert, fp, err := mintShareCertificate(certPath, keyPath, ip)
	if err != nil {
		return tls.Certificate{}, "", err
	}
	return cert, fp, nil
}

func loadShareCertificate(certPath, keyPath string, ip netip.Addr) (tls.Certificate, string, bool) {
	pair, err := tls.LoadX509KeyPair(certPath, keyPath)
	if err != nil || len(pair.Certificate) == 0 {
		return tls.Certificate{}, "", false
	}
	leaf, err := x509.ParseCertificate(pair.Certificate[0])
	if err != nil {
		return tls.Certificate{}, "", false
	}
	want := ip.Unmap()
	covered := false
	for _, candidate := range leaf.IPAddresses {
		if addr, ok := netip.AddrFromSlice(candidate); ok && addr.Unmap() == want {
			covered = true
			break
		}
	}
	if !covered || time.Now().After(leaf.NotAfter.Add(-24*time.Hour)) {
		return tls.Certificate{}, "", false
	}
	sum := sha256.Sum256(pair.Certificate[0])
	return pair, hex.EncodeToString(sum[:]), true
}

func mintShareCertificate(certPath, keyPath string, ip netip.Addr) (tls.Certificate, string, error) {
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return tls.Certificate{}, "", err
	}
	serial, err := rand.Int(rand.Reader, new(big.Int).Lsh(big.NewInt(1), 128))
	if err != nil {
		return tls.Certificate{}, "", err
	}
	now := time.Now()
	tmpl := x509.Certificate{
		SerialNumber:          serial,
		Subject:               pkix.Name{CommonName: "Reasonix Studio share"},
		NotBefore:             now.Add(-time.Hour),
		NotAfter:              now.Add(397 * 24 * time.Hour),
		KeyUsage:              x509.KeyUsageDigitalSignature | x509.KeyUsageKeyEncipherment,
		ExtKeyUsage:           []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth},
		BasicConstraintsValid: true,
		IPAddresses:           []net.IP{ip.Unmap().AsSlice(), net.IPv4(127, 0, 0, 1), net.IPv6loopback},
	}
	der, err := x509.CreateCertificate(rand.Reader, &tmpl, &tmpl, &key.PublicKey, key)
	if err != nil {
		return tls.Certificate{}, "", err
	}
	keyDER, err := x509.MarshalECPrivateKey(key)
	if err != nil {
		return tls.Certificate{}, "", err
	}
	if err := os.MkdirAll(filepath.Dir(certPath), 0o700); err != nil {
		return tls.Certificate{}, "", err
	}
	certPEM := pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der})
	keyPEM := pem.EncodeToMemory(&pem.Block{Type: "EC PRIVATE KEY", Bytes: keyDER})
	if err := os.WriteFile(keyPath, keyPEM, 0o600); err != nil {
		return tls.Certificate{}, "", err
	}
	if err := os.WriteFile(certPath, certPEM, 0o644); err != nil {
		return tls.Certificate{}, "", err
	}
	pair, err := tls.X509KeyPair(certPEM, keyPEM)
	if err != nil {
		return tls.Certificate{}, "", err
	}
	sum := sha256.Sum256(der)
	return pair, hex.EncodeToString(sum[:]), nil
}
