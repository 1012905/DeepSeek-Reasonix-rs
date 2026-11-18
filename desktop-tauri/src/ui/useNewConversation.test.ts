// @vitest-environment jsdom
import { cleanup, render } from "@testing-library/vue";
import { defineComponent, h } from "vue";
import { afterEach, expect, it, vi } from "vitest";
import { useNewConversation } from "./useNewConversation";
import type { RuntimeView } from "../port/hub";

afterEach(cleanup);

it.each([undefined, "gpu"])("starts in the focused project's environment: %s", async (host) => {
  const open = vi.fn().mockResolvedValue(undefined);
  const remote = vi.fn().mockResolvedValue(undefined);
  const runtime = { id: "active", root: "/shared/project", host } as RuntimeView;
  let start!: () => Promise<void>;
  render(defineComponent({
    setup() {
      start = useNewConversation("active", [runtime], "/other", open, remote);
      return () => h("div");
    },
  }));
  await start();
  if (host) {
    expect(remote).toHaveBeenCalledWith("gpu", "/shared/project");
    expect(open).not.toHaveBeenCalled();
  } else {
    expect(open).toHaveBeenCalledWith({ root: "/shared/project" });
    expect(remote).not.toHaveBeenCalled();
  }
});