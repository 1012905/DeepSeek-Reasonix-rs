import { computed, h, onUnmounted, ref, type VNode } from "vue";
import type { AgentPort, CommitProposal, CommitResult } from "../port/port";
import type { CommitDraftFailure } from "../port/workspace";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";

type Phase =
  | { phase: "idle" }
  | { phase: "drafting" }
  | { phase: "ready"; proposal: CommitProposal; message: string; ack: boolean; committing: boolean; error?: string }
  | { phase: "done"; result: CommitResult }
  | { phase: "failed"; error: string };

// Why there is no drafted message. The staged set is still shown, so the person
// writes it themselves; each reason says what to expect from drafting again.
const DRAFT_NOTE: Record<CommitDraftFailure, string> = {
  no_model: "workbench.this_session_has_no_model",
  timeout: "workbench.drafting_timed_out_write_the",
  no_answer: "workbench.the_model_gave_no_commit",
  model_failed: "workbench.the_model_failed_to_draft",
};

// Only the directory gives way when the pane is narrow, from its left, so its tail
// and the whole file name stay visible. The directory is rtl for that ellipsis side.
function CommitPath(path: string): VNode {
  const cut = path.lastIndexOf("/") + 1;
  return h("span", { class: "commit-path" }, [
    cut > 0 ? h("span", { class: "commit-dir" }, path.slice(0, cut)) : null,
    h("span", { class: "commit-base" }, path.slice(cut)),
  ]);
}

// A proposal, not an action: the model drafts for what is staged, the person
// edits, and only the confirm button records a local commit. Nothing here stages
// a file or pushes.
export function useCommitCard(port: AgentPort, onCommitted: () => void, repo?: string) {
  const state = ref<Phase>({ phase: "idle" });
  let pending: AbortController | null = null;
  onUnmounted(() => pending?.abort());

  const draft = () => {
    pending?.abort();
    const ac = new AbortController();
    pending = ac;
    state.value = { phase: "drafting" };
    (repo ? port.proposeCommit(ac.signal, repo) : port.proposeCommit(ac.signal)).then(
      (proposal) => { if (!ac.signal.aborted) state.value = { phase: "ready", proposal, message: proposal.message, ack: false, committing: false }; },
      (e) => { if (!ac.signal.aborted) state.value = { phase: "failed", error: reason(e) }; },
    );
  };
  const close = () => {
    pending?.abort();
    pending = null;
    state.value = { phase: "idle" };
  };

  const button = computed<VNode>(() => h("button", { type: "button", class: "btn sm commit-open", "data-action": "commit.draft", disabled: state.value.phase !== "idle", onClick: draft }, t("workbench.commit")));

  const card = computed<VNode | null>(() => {
    const s = state.value;
    if (s.phase === "idle") return null;

    if (s.phase === "drafting") {
      return h("section", { class: "commit-card", "data-phase": "drafting", "aria-label": t("workbench.commit_message") }, [
        h("p", { class: "refine-wait", role: "status" }, t("workbench.reading_the_staged_changes_and")),
        h("footer", null, [h("button", { type: "button", class: "btn sm", "data-weak": "", "data-action": "commit.close", onClick: close }, t("common.cancel"))]),
      ]);
    }

    if (s.phase === "failed") {
      return h("section", { class: "commit-card", "data-phase": "failed", "aria-label": t("workbench.commit_message") }, [
        h("p", { class: "refine-error", role: "alert" }, s.error),
        h("footer", null, [
          h("button", { type: "button", class: "btn sm", "data-action": "commit.draft", onClick: draft }, t("workbench.draft_commit_message")),
          h("button", { type: "button", class: "btn sm", "data-weak": "", "data-action": "commit.close", onClick: close }, t("common.cancel")),
        ]),
      ]);
    }

    if (s.phase === "done") {
      return h("section", { class: "commit-card", "data-phase": "done", "aria-label": t("workbench.commit_message") }, [
        h("p", { class: "commit-done", role: "status" }, t("workbench.committed_hash_subject", { hash: s.result.hash.slice(0, 7), subject: s.result.subject })),
        h("small", null, t("workbench.recorded_locally_only_nothing_is")),
        h("footer", null, [h("button", { type: "button", class: "btn sm", "data-weak": "", "data-action": "commit.close", onClick: close }, t("common.collapse"))]),
      ]);
    }

    const { proposal, message, ack, committing, error } = s;
    const sensitive = proposal.files.filter((f) => f.sensitive).map((f) => f.path);
    const warned = sensitive.length > 0 || proposal.contentSecrets;
    const set = (patch: Partial<typeof s>) => (state.value = { ...s, ...patch });
    const confirm = () => {
      set({ committing: true, error: undefined });
      port
        .commitStaged({ ...(repo ? { repo } : {}), message, fingerprint: proposal.fingerprint, acknowledgeSecrets: ack })
        .then(
          (result) => { state.value = { phase: "done", result }; onCommitted(); },
          (e) => (state.value = { ...s, committing: false, error: reason(e) }),
        );
    };

    return h("section", { class: "commit-card", "data-phase": "ready", "aria-label": t("workbench.commit_message") }, [
      h("header", null, [
        h("b", null, t("workbench.staged_files")),
        h("small", null, t("workbench.n_files_staged", { n: proposal.files.length })),
      ]),
      h("ul", { class: "commit-files" }, proposal.files.map((f) => h("li", { key: f.path, "data-sensitive": f.sensitive ? "" : undefined, title: f.path }, [
        h("em", { class: "st", "data-s": f.status }, f.status),
        CommitPath(f.path),
        f.insertions !== undefined ? h("i", { "data-io": "up" }, `+${f.insertions}`) : null,
        f.deletions !== undefined ? h("i", { "data-io": "down" }, `-${f.deletions}`) : null,
      ]))),
      proposal.truncated ? h("small", null, t("workbench.the_staged_diff_is_long")) : null,
      warned ? h("div", { class: "commit-warn", role: "alert" }, [
        sensitive.length > 0 ? h("p", null, t("workbench.may_contain_secrets_files", { files: sensitive.join(", ") })) : null,
        proposal.contentSecrets ? h("p", null, t("workbench.added_lines_contain_values_shaped")) : null,
        h("label", null, [
          h("input", { type: "checkbox", "data-action": "commit.acknowledge", checked: ack, disabled: committing, onChange: (e: Event) => set({ ack: (e.target as HTMLInputElement).checked }) }),
          t("workbench.i_confirm_these_can_be"),
        ]),
      ]) : null,
      proposal.draftFailure ? h("p", { class: "refine-error", role: "status", "data-draft": proposal.draftFailure }, t(DRAFT_NOTE[proposal.draftFailure])) : null,
      h("textarea", {
        class: "commit-message",
        autofocus: !!proposal.draftFailure,
        "data-action": "commit.edit",
        "aria-label": t("workbench.commit_message"),
        value: message,
        rows: 6,
        spellcheck: false,
        disabled: committing,
        onInput: (e: Event) => set({ message: (e.target as HTMLTextAreaElement).value }),
      }),
      error ? h("p", { class: "refine-error", role: "alert" }, error) : null,
      h("small", null, t("workbench.recorded_locally_only_nothing_is")),
      h("footer", null, [
        h("button", { type: "button", class: "btn sm", "data-primary": "true", "data-action": "commit.confirm", disabled: committing || !message.trim() || (warned && !ack), onClick: confirm }, committing ? t("common.submitting") : t("workbench.commit_locally")),
        h("button", { type: "button", class: "btn sm", "data-action": "commit.draft", disabled: committing, onClick: draft }, t("workbench.redraft")),
        h("button", { type: "button", class: "btn sm", "data-weak": "", "data-action": "commit.close", disabled: committing, onClick: close }, t("common.cancel")),
      ]),
    ]);
  });

  return { button, card };
}