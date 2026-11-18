<script lang="ts">
import { computed, defineComponent, h, onScopeDispose, reactive, ref, shallowRef, Teleport, useId, watch, type VNodeChild } from "vue";
import { ArrowUp } from "lucide-vue-next";
import { useFolded } from "./usefold";
import StopButton from "./StopButton.vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import PasteKeep, { keepPasteAsFile, type PasteChip } from "./pastekeep.vue";
import type { AgentPort, ChipCall, SessionStatus, WorkspaceGit, WorkspaceInfo } from "../port/port";
import { Picker } from "./Menu.vue";
import BranchChip from "./BranchChip.vue";
import WorktreeChip from "./WorktreeChip.vue";
import { useWorktreeSetup, WorktreeSetupFailure } from "./useWorktreeSetup";
import WorktreeSetupPage from "./WorktreeSetupPage.vue";
import Policy from "./Policy.vue";
import { chosenGroups, groupHeadings, modelLabel, modelMenu, modelTooltip, narrowToGroups } from "./modelmenu";
import ModelGroupChips from "./ModelGroupChips.vue";
import { useModelHealth } from "./modelhealth";
import { effortMenu, effortReading, effortShort, effortsFor, forcesThinkingFor, routeEffortPick } from "./effort";
import EffortColumn from "./EffortColumn.vue";
import { hasVendorMark, VendorMark } from "./vendormarks.vue";
import { CompletionMenu, useCompletion } from "./Completion.vue";
import ChipMirror, { useSkillChips } from "./ChipMirror.vue";
import { useIme } from "./ime";
import { usePlaceholderFit } from "./placeholderfit";
import { useTypedPlaceholder } from "./firstRise";
import { useFitHeight } from "./fitHeight";
import { onComposerHintChange, showsComposerHint, useRunningMessageIntent, useSendShortcut } from "../state/prefs";
import { useExternal } from "../state/external";
import { runningSendIntent, sendAria, sendHint, sendsOnKey } from "./sendkey";
import { countLines, pasteIsLong, planTone, planVerb } from "./intake";
import { nameOf, previewURL } from "./chipfile";
import { fileAppearance } from "./document/FileTypeIcon.vue";
import { useIntake } from "./useIntake";
import type { Dropped } from "./filedrop";
import type { Quote } from "./cards/SayCard.vue";
import StudioIcon, { type StudioIconName } from "./StudioIcon.vue";
import { usePromptRefine } from "./PromptRefine";
import { useProviderOrder } from "../state/providerorder";
import { useModelFilter, writeModelFilter } from "../state/modelfilter";
import { useDraft } from "./useDraft";
import type { Submitted } from "./usesubmit";
import { touchKeyboard } from "./touchKeyboard";
import ImagePreview from "./ImagePreview.vue";
import SideAskButton from "./SideAskButton.vue";
import { chipId, chipName, isPicture, quoteBlock, releaseChip, type Chip } from "./composerChips";

interface Props {
  worktreeTarget?: HTMLElement | null;
  onWorktreePhase?: (phase: string | null) => void;
  fresh?: boolean;
  port: AgentPort;
  status: SessionStatus | null;
  running: boolean;
  queued?: boolean;
  quote?: Quote;
  restore?: { n: number; text: string };
  inject?: { n: number; text: string; session?: { id: string; title: string } };
  focus?: number;
  onSubmit: (text: string, chips?: ChipCall) => Promise<Submitted>;
  onChanged: () => void;
  onError: (e: unknown) => void;
  onSettings?: (section?: string) => void;
  side?: boolean;
  onSide?: () => void;
  sidePanel?: VNodeChild;
  queueStrip?: VNodeChild;
  changeCount?: number;
  countElsewhere?: boolean;
  git?: WorkspaceGit | null;
  workspace?: WorkspaceInfo | null;
  onTreeChanged?: (git: WorkspaceGit) => void;
  pulse?: number;
  draftKey?: string;
}

export default defineComponent((props: Props) => {
  const touch = touchKeyboard();
  const providerOrder = useProviderOrder();
  const modelGroups = useModelFilter();
  const submitting = ref(false);
  const worktree = reactive(useWorktreeSetup(() => props.port, () => !!props.fresh && !!props.git?.repo,
    () => props.status?.workspaceRoot ?? "", () => props.onChanged(), (git) => props.onTreeChanged?.(git), () => props.status?.sessionPath ?? ""));
  const setupDraft = shallowRef<{ text: string; files: string[] }>({ text: "", files: [] });
  const setupPhase = computed(() => worktree.stage?.phase ?? null);
  watch([() => props.onWorktreePhase, setupPhase], ([onPhase, phase]) => onPhase?.(phase), { immediate: true });
  const { text, setText, beginSubmit, finishSubmit } = useDraft(() => props.draftKey ?? "", submitting);
  const caret = ref(0);
  const shots = shallowRef<Chip[]>([]);
  const setShots = (next: Chip[] | ((prev: Chip[]) => Chip[])) => { shots.value = typeof next === "function" ? next(shots.value) : next; };
  const uploads = new Map<string, AbortController>();
  onScopeDispose(() => { uploads.forEach((upload) => upload.abort()); uploads.clear(); });
  const stopping = ref(false);
  const phone = useFolded("phone");
  const toolsOpen = ref(false);
  const picker = ref<HTMLInputElement | null>(null);
  const box = ref<HTMLTextAreaElement | null>(null);
  const modelHealth = reactive(useModelHealth(() => props.port, () => props.pulse ?? 0));
  const menuItems = computed(() => modelMenu(modelHealth.models, providerOrder.value, props.status?.modelRef, modelHealth.readings));
  const chosenModelGroups = computed(() => chosenGroups(menuItems.value, modelGroups.value));
  const busy = ref<Record<string, boolean>>({});
  const busyKeys = new Set<string>();
  let hadSetup = false;
  watch(setupPhase, (phase) => {
    if (hadSetup && !phase) box.value?.focus();
    hadSetup = !!phase;
  }, { immediate: true, flush: "post" });
  const refine = reactive(usePromptRefine(() => props.port, text, (next: string) => {
    setText(next);
    caret.value = next.length;
    box.value?.focus();
  }));
  const guide = useId();
  const completionId = useId();
  const attachTipId = useId();
  let pending: number | [number, number] | null = null;
  let caretAt = 0;
  const type = (next: string, at: number) => { setText(next); caret.value = at; caretAt = at; };
  const moveTo = (next: string, at: number) => { pending = at; type(next, at); };
  const chips = useSkillChips(box, text, moveTo);
  const addSession = (session: { id: string; title: string }) => setShots((prev) => prev.some((c) => c.k === "session" && c.session === session.id)
    ? prev : [...prev, { k: "session", id: chipId(), session: session.id, title: session.title }]);
  const menu = reactive(useCompletion(() => props.port, text, caret, (next: string, at: number) => { moveTo(next, at); box.value?.focus(); }, chips, addSession));
  const ime = useIme();
  const sendShortcut = useSendShortcut();
  const messageIntent = useRunningMessageIntent();
  const runningIntent = computed(() => runningSendIntent(messageIntent.value, chips.list.value.length > 0));

  watch(() => props.quote?.n, () => {
    const quote = props.quote;
    if (!quote?.n) return;
    setShots((prev) => [...prev, { k: "quote", id: chipId(), body: quote.text, turn: quote.turn, source: quote.source, lines: countLines(quote.text) }]);
    queueMicrotask(() => box.value?.focus());
  }, { immediate: true });
  let restoredAt = props.restore?.n ?? 0;
  watch(() => props.restore?.n, () => {
    const restore = props.restore;
    if (!restore?.n || restore.n === restoredAt) return;
    restoredAt = restore.n;
    setText((prev) => {
      const next = !prev.trim() || prev.trim() === restore.text.trim() ? restore.text : `${prev.replace(/\s+$/, "")}\n${restore.text}`;
      pending = next.length;
      return next;
    });
    queueMicrotask(() => box.value?.focus());
  });
  let injectedAt = props.inject?.n ?? 0;
  watch(() => props.inject?.n, () => {
    const inject = props.inject;
    if (!inject?.n || inject.n === injectedAt) return;
    injectedAt = inject.n;
    if (inject.session) addSession(inject.session);
    else setText((prev) => {
      const at = Math.min(caret.value, prev.length);
      const lead = at > 0 && !/\s$/.test(prev.slice(0, at)) ? " " : "";
      const piece = lead + inject.text + " ";
      pending = at + piece.length;
      return prev.slice(0, at) + piece + prev.slice(at).replace(/^ /, "");
    });
    queueMicrotask(() => box.value?.focus());
  });
  watch(() => props.focus, (focus) => { if (focus) box.value?.focus(); }, { immediate: true, flush: "post" });
  watch([() => props.running, () => props.status?.running === false], ([running, idle]) => {
    if (!running || idle) stopping.value = false;
  }, { immediate: true });
  const placeCaret = (el: HTMLTextAreaElement) => {
    if (pending === null) return;
    const [from, to] = typeof pending === "number" ? [pending, pending] : pending;
    el.setSelectionRange(from, to);
    pending = null;
  };
  useFitHeight(box, text, placeCaret);
  const showHint = useExternal(onComposerHintChange, showsComposerHint);
  const fittedPlaceholder = usePlaceholderFit(box, () => props.queued
    ? [t("composer.keep_typing_to_queue_further"), t("composer.keep_typing"), "…"]
    : props.running ? [t("composer.you_can_keep_typing_while"), t("composer.keep_typing"), "…"]
    : [t("composer.describe_a_task_question_or"), t("composer.describe_a_task"), "…"]);
  const typedPlaceholder = useTypedPlaceholder(fittedPlaceholder, () => text.value.length > 0);
  const keepPaste = (chip: PasteChip) => keepPasteAsFile(props.port, chip, setShots);

  const send = () => {
    if (submitting.value || shots.value.some((c) => c.k === "attachment" && c.state !== "ready")) return;
    const value = text.value.trim();
    if (!value && shots.value.length === 0) return;
    const refs = shots.value.flatMap((c) => c.k === "attachment" && c.a ? [c.a.ref] : c.k === "session" ? ["@session:" + c.session] : []);
    const pastes = shots.value.flatMap((c) => c.k === "paste" ? [c.saved ? c.saved.pointer.trim() : c.body] : []);
    const quotes = shots.value.flatMap((c) => c.k === "quote" ? [quoteBlock(c)] : []);
    const compose = (value: string) => [...quotes, [...refs, value.trim()].filter(Boolean).join(" "), ...pastes].filter(Boolean).join("\n\n");
    const line = compose(value);
    const call = chips.call(compose);
    const draft = { text: text.value, shots: shots.value, caret: caretAt, chips: chips.held() };
    if (worktree.selection || worktree.stage) setupDraft.value = { text: value, files: shots.value.map(chipName) };
    const restoreDraft = (select = false) => {
      finishSubmit(false);
      pending = select ? [0, draft.text.length] : draft.caret;
      type(draft.text, draft.caret);
      chips.restore(draft.chips);
      setShots((current) => [...draft.shots, ...current]);
      if (!worktree.selection && !worktree.stage) queueMicrotask(() => box.value?.focus());
    };
    beginSubmit(text.value);
    submitting.value = true;
    type("", 0);
    setShots([]);
    void (async () => {
      try {
        if (worktree.selection || worktree.stage) {
          const prepared = await worktree.prepare(draft.shots.flatMap((chip) => chip.k === "attachment" && chip.a ? [chip.a.path] : chip.k === "paste" && chip.saved ? [chip.saved.path] : []));
          if (!prepared) { restoreDraft(); return; }
        }
        const sent = await (call ? props.onSubmit(line, call) : props.onSubmit(line));
        if (sent === true) {
          finishSubmit(true);
          worktree.dismiss();
          draft.shots.forEach(releaseChip);
        } else {
          restoreDraft(sent === "shell_unavailable");
          worktree.dismiss();
        }
      } catch (error) {
        restoreDraft();
        if (!(error instanceof WorktreeSetupFailure)) props.onError(error);
      } finally { submitting.value = false; }
    })();
  };
  const insert = (snippet: string) => {
    const at = box.value ? box.value.selectionStart : caretAt;
    setText((prev) => {
      const cut = Math.min(at, prev.length);
      pending = cut + snippet.length;
      return prev.slice(0, cut) + snippet + prev.slice(cut);
    });
  };
  const discard = (chip: Chip) => {
    uploads.get(chip.id)?.abort();
    uploads.delete(chip.id);
    releaseChip(chip);
    setShots((prev) => prev.filter((c) => c !== chip));
  };
  const upload = (id: string, blob: File) => {
    const controller = new AbortController();
    uploads.get(id)?.abort();
    uploads.set(id, controller);
    void props.port.attach(blob, blob.name, controller.signal)
      .then((a) => setShots((prev) => prev.map((c) => c.k === "attachment" && c.id === id ? { ...c, a, state: "ready", error: "" } : c)))
      .catch((error: unknown) => setShots((prev) => prev.map((c) => c.k === "attachment" && c.id === id ? { ...c, state: "failed", error: reason(error) } : c)))
      .finally(() => { if (uploads.get(id) === controller) uploads.delete(id); });
  };
  const attach = (blobs: File[]) => {
    if (!blobs.length || submitting.value) return;
    const added: Chip[] = blobs.map((blob) => ({ k: "attachment", id: chipId(), name: blob.name, state: "adding", blob,
      url: blob.type.startsWith("image/") ? previewURL(blob) : undefined }));
    setShots((prev) => [...prev, ...added]);
    for (const chip of added) if (chip.k === "attachment" && chip.blob) upload(chip.id, chip.blob);
  };
  const refIn = (paths: string[]) => {
    props.port.dropRefs(paths).then((refs) => {
      const took = refs.flatMap((r) => r.ref ? [{ k: "attachment" as const, id: chipId(), name: nameOf(r.path ?? r.ref), state: "ready" as const,
        a: { path: r.path ?? "", ref: r.ref, image: r.image } }] : []);
      if (took.length) setShots((prev) => [...prev, ...took]);
      const refused = refs.flatMap((r) => r.error ? [r.error] : []);
      if (refused.length) props.onError(new Error(refused.join("\n")));
    }).catch(props.onError);
  };
  const receive = (drop: Dropped) => {
    if (drop.paths.length) refIn(drop.paths);
    else if (drop.files.length) attach(drop.files);
    if (drop.text) insert(drop.text + " ");
    box.value?.focus();
  };
  const intake = useIntake({ get root() { return props.status?.workspaceRoot ?? props.status?.cwd; }, onReceive: receive });
  const drag = computed(() => reactive(intake).plan);
  const dropzone = ref<HTMLElement | null>(null);
  watch(dropzone, (node, _was, onCleanup) => {
    const off = intake.ref(node);
    if (typeof off === "function") onCleanup(off);
  }, { flush: "post" });
  const efforts = computed(() => effortsFor(modelHealth.models, props.status?.modelRef));
  const declared = computed(() => efforts.value.length > 0);
  const modelLb = computed(() => modelLabel(modelHealth.models, props.status));
  const change = (key: string, call: () => Promise<void>) => {
    if (busyKeys.has(key)) return;
    busyKeys.add(key);
    busy.value = { ...busy.value, [key]: true };
    void Promise.resolve().then(call).then(props.onChanged).catch(props.onError).finally(() => {
      busyKeys.delete(key);
      busy.value = { ...busy.value, [key]: false };
    });
  };
  const stop = () => {
    if (stopping.value) return;
    stopping.value = true;
    void props.port.cancel().then(props.onChanged, (error: unknown) => { stopping.value = false; props.onError(error); });
  };
  const onPaste = (event: ClipboardEvent) => {
    if (submitting.value) { event.preventDefault(); return; }
    if (!event.clipboardData) return;
    const files = [...event.clipboardData.files];
    if (files.length) {
      event.preventDefault();
      attach(files.map((file) => new File([file], file.name, { type: file.type })));
      return;
    }
    const body = event.clipboardData.getData("text/plain");
    if (!pasteIsLong(body)) return;
    event.preventDefault();
    setShots((prev) => [...prev, { k: "paste", id: chipId(), body, lines: countLines(body) }]);
  };
  const onKeyDown = (event: KeyboardEvent) => {
    if (!ime.isIme(event) && refine.onKey(event)) return;
    if (ime.isIme(event)) {
      if (event.key === "Escape") event.stopPropagation();
      if (event.key === "Enter") event.preventDefault();
      return;
    }
    const sendKey = sendsOnKey(event, sendShortcut.value, touch);
    if (menu.open && (event.key === "ArrowDown" || event.key === "ArrowUp")) {
      event.preventDefault(); menu.move(event.key === "ArrowDown" ? 1 : -1); return;
    }
    if (menu.open && (event.key === "Tab" || (event.key === "Enter" && !menu.typedChosen && !(sendShortcut.value === "modifier_enter" && sendKey))) && !event.shiftKey) {
      event.preventDefault(); menu.accept(undefined, event.key === "Tab" ? "descend" : "confirm"); return;
    }
    if ((menu.open || menu.loading) && event.key === "Escape") {
      event.preventDefault(); event.stopPropagation(); menu.dismiss(); return;
    }
    if (sendKey) { event.preventDefault(); send(); }
  };
  const onSettings = (section?: string) => props.onSettings?.(section);
  const icon = (name: StudioIconName) => h(StudioIcon, { name });
  const renderShot = (chip: Chip, index: number) => h("li", { class: "shot", key: chip.id,
    "data-state": chip.k === "attachment" ? chip.state : undefined, style: { "--i": index } }, [
    ...(chip.k === "attachment" ? [
      isPicture(chip) ? h(ImagePreview, { src: chip.url, name: chip.name, className: "thumb" }, () => h("span", {
        "aria-hidden": "true", style: chip.url ? { backgroundImage: `url(${chip.url})` } : undefined,
      })) : h("span", { class: "glyph", "aria-hidden": "true" }, [h(fileAppearance(chip.a?.path ?? chip.name).icon, { size: 20, strokeWidth: 1.8 })]),
      h("span", { class: "meta" }, [
        h("span", { class: "nm", title: chip.a?.path ?? chip.name }, chip.name),
        chip.state === "adding" && h("span", { class: "sz live" }, t("composer.adding")),
        chip.state === "ready" && h("span", { class: "sz" }, isPicture(chip) ? t("composer.image") : t("common.files")),
        chip.state === "failed" && chip.error && h("span", { class: "why", title: chip.error }, chip.error),
        chip.state === "failed" && h("button", { class: "retry", "data-action": "session.attach", onClick: () => {
          if (!chip.blob) return;
          setShots((prev) => prev.map((c) => c === chip ? { ...chip, state: "adding", error: "" } : c));
          upload(chip.id, chip.blob);
        } }, t("composer.could_not_add_retry")),
      ]),
    ] : chip.k === "session" ? [
      h("span", { class: "glyph", "aria-hidden": "true" }, [icon("history")]),
      h("span", { class: "meta" }, [h("span", { class: "nm", title: chip.title }, chip.title), h("span", { class: "sz" }, t("composer.past_conversation_read_only_reference"))]),
    ] : [
      h("span", { class: "glyph", "aria-hidden": "true" }, [icon(chip.k === "quote" ? "quote" : "file")]),
      h("span", { class: "meta" }, [h("span", { class: "nm", title: chip.body }, chipName(chip)),
        h("button", { class: "undo", onClick: () => { discard(chip); insert(chip.k === "quote" ? quoteBlock(chip) : chip.body); } }, t("composer.n_lines_put_it_back", { n: chip.lines })),
        chip.k === "paste" && h(PasteKeep, { chip, onKeep: keepPaste }),
      ]),
    ]),
    h("button", { class: "x", "aria-label": t("common.remove_name", { name: chipName(chip) }), onClick: () => discard(chip) }, [icon("close")]),
  ]);

  return () => {
    const { port, status, running } = props;
    const adding = shots.value.some((c) => c.k === "attachment" && c.state === "adding");
    const failed = shots.value.some((c) => c.k === "attachment" && c.state === "failed");
    const hasDraft = text.value.trim().length > 0 || shots.value.some((c) => c.k !== "attachment" || c.state === "ready");
    const lines = countLines(text.value);
    const currentMark = modelHealth.models.find((model) => model.ref === status?.modelRef)?.mark;
    const sendLabel = t(submitting.value ? "common.sending" : running ? runningIntent.value === "followup" ? "composer.queue" : "common.steer" : "common.send");
    const setupPage = worktree.stage && h(WorktreeSetupPage, { stage: worktree.stage, task: setupDraft.value.text, files: setupDraft.value.files,
      busy: submitting.value, onRetry: send, onCancel: () => void worktree.cancel(), onEdit: worktree.dismiss });
    return [
      setupPage && (props.worktreeTarget ? h(Teleport, { to: props.worktreeTarget }, [setupPage]) : setupPage),
      h("div", { class: "dropzone", ref: dropzone, "data-worktree-setup": worktree.stage ? "" : undefined, inert: !!worktree.stage }, [
        menu.open && h(CompletionMenu, { id: completionId, items: menu.completion.items, active: menu.active, kind: menu.completion.kind,
          query: menu.completion.query ?? "", kb: menu.kb, onPick: menu.accept, onHover: menu.hover }),
        h("div", { role: "status", "aria-live": "polite", "aria-label": t("common.completions") }, [menu.loading && !menu.open && h("div", { class: "menu slashmenu" }, [h("div", { class: "mnone" }, t("composer.loading_commands_and_skills"))])]),
        drag.value && h("p", { class: "intake", "data-tone": planTone(drag.value), role: "status", "aria-live": "polite" }, planVerb(drag.value)),
        refine.card,
        shots.value.length > 0 && h("ul", { class: "shots" }, shots.value.map(renderShot)),
        status?.vision === false && shots.value.some((c) => c.k === "attachment" && c.state === "ready" && isPicture(c)) && h("p", { class: "shotwarn", role: "status" }, t(status?.visionDeclared === false ? "composer.nobody_has_said_whether_this" : "composer.this_model_does_not_read")),
        props.queueStrip,
        h("div", { class: "fmain" }, [
          h("span", { class: "prompt", "aria-hidden": "true" }, [icon("chevron")]),
          h("textarea", {
            "data-action-keydown": "session.send", "data-window-keys": "", ref: box, rows: 1, value: text.value,
            "data-chips": chips.list.value.length > 0 ? "" : undefined, placeholder: showHint.value ? typedPlaceholder.value : "", role: "combobox",
            "aria-label": t("composer.task_input"), "aria-describedby": guide, "aria-keyshortcuts": sendAria(sendShortcut.value, touch),
            "aria-busy": submitting.value, readOnly: submitting.value, "aria-expanded": menu.open, "aria-controls": menu.open ? completionId : undefined,
            "aria-autocomplete": "list", "aria-activedescendant": menu.open ? `${completionId}-${menu.active}` : undefined,
            onInput: (event: Event) => { const el = event.target as HTMLTextAreaElement; type(el.value, el.selectionStart); },
            onBlur: () => menu.dismiss(), onKeyup: (event: Event) => { caret.value = chips.snap(event.currentTarget as HTMLTextAreaElement, caret.value); },
            onClick: (event: Event) => { caret.value = chips.snap(event.currentTarget as HTMLTextAreaElement, caret.value); },
            "data-action-paste": "session.attach", onPaste, ...ime.handlers, onKeydown: onKeyDown,
          }),
          h(ChipMirror, { box, text: text.value, chips: chips.list.value }),
        ]),
        h("div", { class: "composeguide", id: guide }, [
          h("span", { class: "composecaps", "aria-hidden": "true" }, [h("span", [h("kbd", "/"), t("composer.commands_and_skills")]), h("span", [h("kbd", "@"), t("composer.reference_files")])]),
          h("span", { class: "composehint", "aria-live": "polite" }, stopping.value ? t("composer.stopping") : submitting.value ? t("common.sending") : adding ? t("composer.adding_attachments") : failed ? t("composer.an_attachment_failed_retry_or") : sendHint(sendShortcut.value, running, touch, runningIntent.value)),
          (text.value.length >= 240 || lines > 3) && h("span", { class: "fcount" }, t("composer.n_characters_lines_lines", { n: text.value.length, lines })),
        ]),
        h("div", { class: "row" }, [
          h("input", { ref: picker, "data-action-change": "session.attach", type: "file", multiple: true, hidden: true, onChange: (event: Event) => {
            const el = event.target as HTMLInputElement; attach([...(el.files ?? [])]); el.value = "";
          } }),
          h("div", { class: "turntools", "data-more": phone.value && toolsOpen.value ? "" : undefined }, [
            h("button", { class: "mode plain attach studio-attach", "aria-label": t("composer.add_an_attachment"), "aria-describedby": attachTipId, onClick: () => picker.value?.click() }, [
              icon("plus"), h("span", { class: "studio-control-tip", id: attachTipId, role: "tooltip" }, [h("b", t("composer.add_an_attachment")), h("span", t("composer.you_can_also_drop_or"))]),
            ]),
            props.onSide && h(SideAskButton, { open: props.side ?? false, onToggle: props.onSide, panel: props.sidePanel }),
            refine.button,
            h("div", { class: "studio-mode-control" }, [h(Picker, {
              className: "mode studio-mode-picker", triggerAction: "plan.mode", "data-action": "plan.mode", ariaPressed: status?.plan ?? false,
              place: "bottom", title: t("composer.work_mode"), menuClassName: "composer-menu mode-menu", current: status?.plan ? "plan" : "agent", pending: busy.value.plan,
              items: [{ value: "__mode", label: t("composer.work_mode"), right: t("common.current_task"), header: true },
                { value: "agent", label: "Agent", desc: t("composer.run_tasks_with_enabled_tools") }, { value: "plan", label: "Plan", desc: t("composer.plan_steps_before_changing_files") }],
              onPick: (value: string) => change("plan", () => port.setPlanMode(value === "plan")),
              label: [icon(status?.plan ? "tasks" : "agent"), h("span", { class: "studio-sr-label" }, t("common.plan")), h("span", status?.plan ? "Plan" : "Agent"), icon("down")],
            })]),
            props.git?.repo && (worktree.sources || status?.workspaceRoot) ? h(WorktreeChip, { port, workspace: props.workspace ?? null, onChanged: props.onChanged,
              onSwitched: props.onTreeChanged, onError: props.onError, changeCount: props.countElsewhere ? 0 : props.changeCount ?? 0,
              sources: worktree.sources ?? { available: false, sourceRoot: status?.workspaceRoot ?? "", sources: [] }, canCreate: !!props.fresh && !!worktree.sources?.available,
              selection: worktree.selection, branch: props.git.branch, busy: submitting.value || running, onSelect: worktree.setSelection, refresh: worktree.refresh,
            }) : h(BranchChip, { port, git: props.git ?? null, changeCount: props.changeCount ?? 0, countElsewhere: props.countElsewhere ?? false, onChanged: props.onChanged, onSwitched: props.onTreeChanged, onError: props.onError }),
            status?.planPhase === "executing" && h("span", { class: "mode plain", "data-plan-phase": "executing", title: t("composer.executing_an_approved_plan") }, [h("span", { class: "lb" }, t("composer.executing_plan"))]),
            h(Policy, { port, status, onChanged: props.onChanged, onBoundary: () => onSettings("tools:sandbox") }),
            h("div", { class: "studio-model-group" }, [h(Picker, {
              wrapClassName: "studio-model-control", className: "mode model-picker", "data-action": "model.select", sheet: true, place: "bottom", align: "end",
              title: modelTooltip(modelHealth.models, status?.modelRef, modelLb.value, status && declared.value ? effortReading(status.effort, status.modes) : undefined), current: status?.modelRef,
              items: narrowToGroups(menuItems.value, chosenModelGroups.value), head: h(ModelGroupChips, { headings: groupHeadings(menuItems.value), chosen: chosenModelGroups.value, onChange: writeModelFilter }),
              menuClassName: "studio-model-menu composer-menu", searchAlways: true, searchPlaceholder: t("composer.search_models_or_providers"),
              menuTitle: [h("b", t("composer.choose_model")), h("small", t("composer.for_future_tasks"))], onOpen: () => void modelHealth.check(modelHealth.loadModels()),
              pending: busy.value.model || busy.value.effort || busy.value.mode, asideClassName: "studio-effort-menu", stayOpen: (ref: string) => ref !== "__manage-models",
              onPick: (ref: string) => ref === "__manage-models" ? onSettings("providers") : change("model", () => port.setModel(ref)),
              aside: status ? (close: () => void) => h(EffortColumn, {
                items: effortMenu(efforts.value, modelLb.value, "__effort-declare", status.modes, forcesThinkingFor(modelHealth.models, status.modelRef)), current: declared.value ? status.effort || "auto" : "",
                onPick: (value: string) => { close(); routeEffortPick(value, status.modes, {
                  declare: () => onSettings("providers:effort-declare"), effort: (level) => change("effort", () => port.setEffort(level)), mode: (mode) => change("mode", () => port.setModelMode(mode)),
                }); },
              }) : undefined,
              label: [hasVendorMark(currentMark) && h(VendorMark, { slug: currentMark, size: 14 }), h("span", { class: "nm" }, modelLb.value), status && [h("i", { class: "studio-model-sep", "aria-hidden": "true" }),
                h("span", { class: "studio-effort-reading" }, [icon("brain"), declared.value ? effortShort(status.effort, status.modes) : t("common.not_declared")])], icon("down")],
            })]),
            phone.value && h("button", { type: "button", class: "mode plain turn-more", "data-action": "composer.more-tools", "aria-expanded": toolsOpen.value,
              "aria-label": t("composer.more_tools"), onClick: () => { toolsOpen.value = !toolsOpen.value; } }, [icon("more")]),
          ]),
          h("span", { class: "go" }, [running && h(StopButton, { stopping: stopping.value, onStop: stop }),
            h("button", { class: "btn send", "aria-label": sendLabel, "data-primary": running ? undefined : "", "data-action": "session.send", "data-running": running ? "" : undefined,
              disabled: submitting.value || stopping.value || adding || failed || !hasDraft || !!worktree.stage, "aria-busy": submitting.value, onClick: send }, [h("span", { class: "ic", "aria-hidden": "true" }, [h(ArrowUp)]), h("span", sendLabel)]),
          ]),
        ]),
      ]),
    ];
  };
}, { name: "Composer", props: ["worktreeTarget", "onWorktreePhase", "fresh", "port", "status", "running", "queued", "quote", "restore", "inject", "focus", "onSubmit", "onChanged", "onError", "onSettings", "side", "onSide", "sidePanel", "queueStrip", "changeCount", "countElsewhere", "git", "workspace", "onTreeChanged", "pulse", "draftKey"] });
</script>
