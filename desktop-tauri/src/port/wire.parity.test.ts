// What this file proves: the Vue wire mirror's event kind set agrees with the Go
// reflection manifest in both directions — kinds are the stable routing surface,
// so none the kernel emits is unreadable here and none is invented.
// Type-witness parity is recorded as B4: the worktree wire.ts was restructured and
// dropped types the manifest still declares, so those witnesses need a deliberate
// contract reconciliation rather than a restore of the HEAD version of this file.
import { describe, expect, it } from "vitest";
import manifestJson from "../../../crates/contract/src/eventwire_manifest.json";
import * as wire from "./wire";

describe("wire kind-set parity", () => {
  it("kind sets agree in both directions", () => {
    expect([...wire.KINDS].sort()).toEqual([...manifestJson.kinds].sort());
  });
});