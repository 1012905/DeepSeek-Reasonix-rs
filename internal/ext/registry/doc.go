// Package registry reads the signed Reasonix extension registry. The index is
// verified with the release key before it is parsed, and every downloaded file
// is checked against the digest the signed index declares.
package registry
