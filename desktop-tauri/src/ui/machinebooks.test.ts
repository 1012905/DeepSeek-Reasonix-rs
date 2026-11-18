// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { render, waitFor } from "@testing-library/vue";
import { defineComponent } from "vue";
import { useMachineBooks } from "./machinebooks";
import type { HubPort, TreeWorkspace } from "../port/hub";
import type { RemoteHost } from "../port/remote";

const tree: TreeWorkspace[] = [{ root: "/root", name: "root", sessions: [] } as unknown as TreeWorkspace];
const host = (name: string, status: string) => ({ name, status, target: "root@h" }) as unknown as RemoteHost;
const hubWith = (remoteTree: (h: string) => Promise<TreeWorkspace[] | null>) => ({ remoteTree }) as unknown as HubPort;

function mountBooks(hub: HubPort, remotes: RemoteHost[] | null) {
  let api!: ReturnType<typeof useMachineBooks>;
  const Harness = defineComponent({
    setup() {
      api = useMachineBooks(hub, remotes);
      return () => null;
    },
  });
  render(Harness);
  return api;
}

afterEach(() => localStorage.clear());

describe("a machine's book", () => {
  it("is read on demand while nothing is open on it, and remembered", async () => {
    const remoteTree = vi.fn(async () => tree);
    const books = mountBooks(hubWith(remoteTree), [host("my-server", "idle")]);
    expect(remoteTree).not.toHaveBeenCalled();
    await books.read("my-server");
    expect(books.trees.value["my-server"]).toEqual(tree);
    expect(JSON.parse(localStorage.getItem("rx-remote-books") ?? "[]")).toEqual(["my-server"]);
  });

  it("is read again on start for a machine read before, and not for one never read", async () => {
    localStorage.setItem("rx-remote-books", JSON.stringify(["my-server"]));
    const remoteTree = vi.fn(async () => tree);
    mountBooks(hubWith(remoteTree), [host("my-server", "idle"), host("other", "idle")]);
    await waitFor(() => expect(remoteTree).toHaveBeenCalledWith("my-server"));
    expect(remoteTree).not.toHaveBeenCalledWith("other");
  });

  it("hands a failed read to the caller", async () => {
    const remoteTree = vi.fn(async () => {
      throw new Error("remote.unreachable");
    });
    const books = mountBooks(hubWith(remoteTree), [host("my-server", "idle")]);
    await expect(books.read("my-server")).rejects.toThrow("remote.unreachable");
    expect(localStorage.getItem("rx-remote-books")).toBeNull();
  });
});