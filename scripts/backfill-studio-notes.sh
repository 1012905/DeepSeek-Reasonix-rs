#!/usr/bin/env bash
# Retired. Studio notes now publish as the notes.md release asset of each tag,
# uploaded by release-studio-tauri.yml before the catalog entry that names them,
# so no catalog entry is ever left without its notes object. The R2 catalog this
# backfill repaired no longer exists; there is nothing left to fill.
echo "backfill-studio-notes.sh is retired: release-studio-tauri.yml publishes notes as the notes.md release asset of each tag" >&2
exit 1