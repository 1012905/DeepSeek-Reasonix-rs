import { computed, ref, toValue, type MaybeRefOrGetter } from "vue";
import type { AgentPort, Checkpoint } from "../port/port";
import type { Item } from "../state/session";
import { pairCheckpoints } from "../state/checkpoints";
import { t } from "../i18n";
import type { Quote, ReplyActions } from "./cards/SayCard.vue";

interface Inputs {
  port: AgentPort;
  items: MaybeRefOrGetter<Item[]>;
  checkpoints: MaybeRefOrGetter<Checkpoint[]>;
  running: MaybeRefOrGetter<boolean>;
  model?: MaybeRefOrGetter<string | undefined>;
  submit: (text: string) => Promise<boolean>;
  reloadSession: () => Promise<void>;
  onSettings: (section?: string) => void;
  onRunDetail: () => void;
  onError: (e: unknown) => void;
  // The title the branch is given and what to do with the session it makes.
  branchName?: MaybeRefOrGetter<string | undefined>;
  onBranched?: (path: string) => void;
}

interface ReplyTurn {
  turn: number;
  text: string;
  hasLaterTurns: boolean;
}

function sameTurns(a: Map<string, ReplyTurn>, b: Map<string, ReplyTurn>) {
  if (a.size !== b.size) return false;
  for (const [id, x] of a) {
    const y = b.get(id);
    if (!y || x.turn !== y.turn || x.text !== y.text || x.hasLaterTurns !== y.hasLaterTurns) return false;
  }
  return true;
}

/** What a finished reply can be acted on with, and the draft signal a quote
 *  travels to the composer on. The transcript owns neither: the pane holds the
 *  session these read from, and the composer is where a quote has to land. */
export function useReplyActions({ port, items, checkpoints, running, model, submit, reloadSession, onSettings, onRunDetail, onError, branchName, onBranched }: Inputs) {
  const quote = ref<Quote>({ text: "", n: 0 });

  // Re-running a turn is a conversation rewind and then the same words again:
  // the transcript goes back, the files do not, and the reply that was there
  // stays in the history the rewind wrote. Scope is conversation for exactly
  // that reason — reverting the work too would be a far larger promise than
  // the word "regenerate" makes.
  const regenerate = async (turn: number, text: string) => {
    const plan = await port.prepareRewind(turn, "conversation");
    if (!plan.canConversation) throw new Error(plan.disabledReason || t("chat.this_turn_cannot_be_regenerated"));
    await port.commitRewind(plan.planId);
    await reloadSession();
    await submit(text);
  };

  // A reply belongs to the most recent user turn, including when that turn
  // produced several replies. A turn without a paired checkpoint must not
  // inherit the preceding turn's rewind target.
  const derived = computed(() => {
    const all = toValue(items);
    const paired = pairCheckpoints(all, toValue(checkpoints));
    const turns = new Map<string, ReplyTurn>();
    let ask: ReplyTurn | undefined;
    for (const item of all) {
      if (item.t === "user" && !item.pending && !item.steer) {
        if (ask) ask.hasLaterTurns = true;
        const cp = paired.get(item.id);
        ask = cp ? { turn: cp.turn, text: item.text, hasLaterTurns: false } : undefined;
      } else if (item.t === "say" && ask) {
        turns.set(item.id, ask);
      }
    }
    return { paired, turns };
  });

  // A streamed delta makes a new items array without moving this answer; the
  // previous map is kept while equal so settled rows holding `reply` do not
  // redraw once per delta.
  let kept: Map<string, ReplyTurn> | null = null;
  const replyTurns = computed(() => {
    const next = derived.value.turns;
    if (!kept || !sameTurns(kept, next)) kept = next;
    return kept;
  });

  // Which reply is being quoted is the kernel's to say, so the turn its
  // checkpoint named travels with the text. A transcript rebuilt without
  // checkpoints has no turn to give and sends none rather than a guess.
  const turnOf = (id: string) => {
    const all = toValue(items);
    const paired = derived.value.paired;
    const at = all.findIndex((i) => i.id === id);
    for (let i = at < 0 ? all.length - 1 : at; i >= 0; i--) {
      const item = all[i];
      if (item.t !== "user" || item.pending) continue;
      return paired.get(item.id)?.turn;
    }
    return undefined;
  };

  const reply = computed<ReplyActions>(() => ({
    onQuote: (text: string, id: string) => { quote.value = { text, turn: turnOf(id), n: quote.value.n + 1 }; },
    onQuoteFile: (text: string, source: string) => { quote.value = { text, source, n: quote.value.n + 1 }; },
    canRegenerate: (id: string) => !toValue(running) && replyTurns.value.has(id),
    hasLaterTurns: (id: string) => replyTurns.value.get(id)?.hasLaterTurns ?? false,
    onRegenerate: (id: string) => {
      if (toValue(running)) return;
      const ask = replyTurns.value.get(id);
      if (ask) void regenerate(ask.turn, ask.text).catch(onError);
    },
    canBranch: (id: string) => !!onBranched && replyTurns.value.has(id) && (!toValue(running) || !!replyTurns.value.get(id)?.hasLaterTurns),
    onBranch: async (id: string) => {
      const ask = replyTurns.value.get(id);
      if (ask && onBranched && (!toValue(running) || ask.hasLaterTurns)) {
        try { await onBranched(await port.branchAt(ask.turn, toValue(branchName) ?? "")); } catch (e) { onError(e); }
      }
    },
    model: toValue(model),
    onConfigureModel: () => onSettings("model"),
    onRunDetail,
  }));

  // Rewriting a message is the same act with different words: the turn goes
  // back and what the person now means goes out. The card asks for it by the
  // turn its own checkpoint named, so it cannot aim at a turn that moved.
  const onResend = (turn: number, text: string) => regenerate(turn, text);

  return { quote, reply, onResend };
}