import { computed, ref, shallowRef, toValue, watch, type ComputedRef, type MaybeRefOrGetter, type Ref, type ShallowRef } from "vue";
import { host } from "../port/host";
import type { AgentPort } from "../port/port";
import { GRANTED, conversationFile, isLocalPathShape } from "./filelinks";
import { GRANTED_FILE, grantedFileKey, grantedFilePath } from "./grantedfiles";
import type { LocalRefs } from "./Markdown.vue";
import { absoluteWorkspacePath, type FileMenu } from "./workbench/WorkbenchFileMenu.vue";

/** What a conversation's links and pictures resolve against, plus the one file
 *  the workbench was just asked to show and the menu a link opened. */
export interface ConversationFiles {
  refs: ComputedRef<LocalRefs>;
  request: ShallowRef<{ path: string; consumed: () => void } | undefined>;
  menu: ShallowRef<(FileMenu & { id: number }) | undefined>;
  closeMenu(): void;
  platform: Ref<string>;
}

/** The file navigation of one conversation: its workspace, the folders granted
 *  to it, and the workbench request a link leaves behind. */
export function useConversationFiles(
  port: MaybeRefOrGetter<AgentPort>,
  root: MaybeRefOrGetter<string>,
  remote: MaybeRefOrGetter<boolean>,
  shown: MaybeRefOrGetter<boolean>,
  reveal: () => void,
  fail: (e: unknown) => void,
): ConversationFiles {
  // Opening and revealing belong to the pane that draws the menu, so these two
  // only keep the call shape the pane already has.
  void remote;
  void fail;
  const request = shallowRef<{ path: string; consumed: () => void }>();
  const menu = shallowRef<FileMenu & { id: number }>();
  const platform = ref("");
  // The folders the user granted this workspace: a picture in one is served by
  // the controlled image channel, named by the folder it lives in.
  const granted = shallowRef<string[]>([]);
  let menuID = 0;
  let reads = 0;

  // A link opens its file once: the workbench reports it consumed, so a later
  // press on Files, or any redraw of the request, finds nothing to replay.
  const ask = (path: string) => {
    const req = { path, consumed: () => { if (request.value === req) request.value = undefined; } };
    request.value = req;
    reveal();
  };

  const closeMenu = () => { menu.value = undefined; };

  watch([() => toValue(port), () => toValue(root)], () => {
    request.value = undefined;
    menu.value = undefined;
    const ticket = ++reads;
    granted.value = [];
    const agent = toValue(port);
    void agent.workspaceRoots?.().then((r) => { if (ticket === reads) granted.value = r.extra ?? []; }, () => {});
  }, { immediate: true });

  // A menu points at a row of the conversation, so it goes away with it.
  watch(() => toValue(shown), closeMenu);

  void host().describe().then((info) => { platform.value = info.platform; }, () => {});

  const refs = computed<LocalRefs>(() => {
    const agent = toValue(port);
    const base = toValue(root);
    // Read as a dependency, not a live value: a message rendered before the
    // grants arrived is redrawn once they do, so its paths become links.
    const folders = granted.value;
    const path = (href: string) => conversationFile(base, href);
    const grantedMark = (src: string) => {
      if (!isLocalPathShape(src)) return null;
      for (const [n, folder] of folders.entries()) {
        const rel = conversationFile(folder, src);
        if (rel) return `${GRANTED}/${n}/${rel}`;
      }
      return null;
    };
    const image = (src: string): string | null => {
      const marked = src.startsWith(`${GRANTED}/`) ? /^[^/]+\/(\d+)\/(.+)$/.exec(src) : null;
      if (marked) {
        const folder = folders[Number(marked[1])];
        return folder ? agent.workspaceImageURL(decodeURIComponent(marked[2]), folder) : null;
      }
      const file = path(src);
      if (file) return agent.workspaceImageURL(file);
      const held = grantedMark(src);
      return held ? image(held) : null;
    };
    const offer = (e: MouseEvent | KeyboardEvent, href: string) => {
      const file = path(href);
      if (!file || !toValue(shown) || document.getSelection()?.isCollapsed === false) return;
      e.preventDefault();
      e.stopPropagation();
      const anchor = e.currentTarget as HTMLElement;
      const rect = anchor.getBoundingClientRect();
      const [x, y] = "clientX" in e && (e.clientX !== 0 || e.clientY !== 0) ? [e.clientX, e.clientY] : [rect.left, rect.bottom];
      menu.value = { id: ++menuID, path: file, folder: false, anchor, x, y };
    };
    return {
      file: (ref) => () => ask(ref.root ? grantedFilePath(ref.root, ref.path) : ref.path),
      path,
      // A path in text may become a link when it is in the workspace or in a granted folder.
      linkable: (href) => path(href) ?? (isLocalPathShape(href) ? grantedFileKey(folders, href) : null),
      title: (href) => { const file = path(href); return file ? absoluteWorkspacePath(base, file) : ""; },
      link: (href) => {
        const file = path(href);
        if (file) return () => ask(file);
        const held = href.startsWith(`${GRANTED_FILE}/`) || href.includes(`/${GRANTED_FILE}/`)
          ? decodeURIComponent(href.slice(href.indexOf(GRANTED_FILE)))
          : isLocalPathShape(href) ? grantedFileKey(folders, href) : null;
        return held ? () => ask(held) : null;
      },
      image,
      granted: grantedMark,
      menu: offer,
    };
  });

  return { refs, request, menu, closeMenu, platform };
}
