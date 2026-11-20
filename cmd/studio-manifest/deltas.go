package main

import (
	"fmt"
	"os"
	"path/filepath"

	"reasonix/internal/platform/update"
)

// deltas lists each platform's chunked update under deltaDir, laid out by
// studio-delta as <platform>/index.json.zst beside its signature. The index and
// the chunks it names ride as release assets of the same tag.
func deltas(deltaDir, tag string) (map[string]update.Delta, error) {
	entries, err := os.ReadDir(deltaDir)
	if err != nil {
		return nil, err
	}
	out := map[string]update.Delta{}
	for _, e := range entries {
		if !e.IsDir() || e.Name() == "chunks" {
			continue
		}
		platform := e.Name()
		index := filepath.Join(deltaDir, platform, update.DeltaIndexName)
		if _, err := os.Stat(index + ".minisig"); err != nil {
			return nil, fmt.Errorf("studio-manifest: %s delta index is not signed: %w", platform, err)
		}
		size, sum, err := hashFile(index)
		if err != nil {
			return nil, err
		}
		url := fmt.Sprintf("%s/delta-%s-%s", update.StudioChunks(tag), platform, update.DeltaIndexName)
		out[platform] = update.Delta{
			Index:  update.Asset{URL: url, Sig: url + ".minisig", Size: size, SHA256: sum},
			Chunks: update.StudioChunks(tag),
		}
		fmt.Printf("delta: %s (%d bytes index)\n", platform, size)
	}
	return out, nil
}
