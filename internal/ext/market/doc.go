// Package market reads the signed Reasonix registry and installs what it
// lists through install_source. The registry is a source, never a trust root:
//
//   - The signed index is fetched over https at one fixed URL, verified with
//     the release key before it is parsed, and revalidated with ETag. Every
//     file named by an entry is checked against its signed sha256.
//   - Browse reads may fall back to a last-good copy, labelled with when it
//     was saved. Install and plan never read or write that copy.
//   - Entries revoked, too new for this build, or for another platform are
//     filtered before they reach the page.
//   - An install is the ordinary two-phase plan and apply; expectDigest is
//     the sha256 the signed index declared, so source drift is refused before
//     anything is written. What landed is kept in a ledger under the Reasonix
//     home and reported as installed only while its targets still exist.
package market
