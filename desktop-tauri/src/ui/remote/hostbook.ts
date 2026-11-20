import type { RemoteHost, RemoteHostEdit } from "../../port/remote";

// The host book's vocabulary, in one place because two screens read it: the
// settings page's list (settings/parts/data-remote-hosts.vue) and the main
// window's machines panel (./Remotes.vue, ./RemoteHosts.vue). The two differ in
// what they draw — one is a form inside the settings sheet, the other a panel
// with open/move — but never in what a stored row means, so a second copy of
// these would be a second answer to the same question.

const STATUS_LABEL: Record<string, string> = {
  idle: "common.not_connected",
  connecting: "remote.connecting_2",
  connected: "remote.connected",
  reconnecting: "remote.reconnecting",
  degraded: "remote.a_forward_did_not_attach",
  stopped: "remote.disconnected",
};

/** The word beside the name. A machine turned off reads as suspended rather
 *  than as whatever its last link was doing. */
export function statusKey(host: RemoteHost): string {
  return host.disabled ? "common.suspended" : STATUS_LABEL[host.status] ?? host.status;
}

/** A probe route's own name where the kernel's spelling is already the word
 *  (npm), and a message key where it is not. */
export const ROUTE_LABEL: Record<string, string> = { npm: "npm", upload: "remote.upload", download: "remote.download" };

/** One machine holds several projects. The kernel folds the two stored fields
 *  into one default-first list, so a row that only ever set the single
 *  workspace arrives here as a list of one rather than as a case to handle. */
export function workspacesOf(host: RemoteHost | null): string[] {
  return host?.workspaces?.length ? host.workspaces : host?.workspace ? [host.workspace] : [];
}

/** A saved row goes back in full: the endpoint replaces the entry, so a form
 *  that sent only what it displays would blank the rest. */
export function draftOf(host: RemoteHost | null): RemoteHostEdit {
  return {
    name: host?.name ?? "",
    kind: host?.kind ?? "",
    distro: host?.distro ?? "",
    host: host?.host ?? "",
    port: host?.port ?? 0,
    user: host?.user ?? "",
    identityFile: host?.identityFile ?? "",
    proxyJump: host?.proxyJump ?? "",
    workspaces: workspacesOf(host),
    serveInstall: host?.serveInstall ?? "",
    provider: host?.provider ?? "",
    useSSHConfig: host?.useSSHConfig ?? false,
    passphraseEnv: host?.passphraseEnv ?? "",
    passwordEnv: host?.passwordEnv ?? "",
    disabled: host?.disabled ?? false,
  };
}
