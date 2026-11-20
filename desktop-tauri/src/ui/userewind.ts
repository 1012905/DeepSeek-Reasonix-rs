import { toValue, type MaybeRefOrGetter } from "vue";
import { t } from "../i18n";
import type { AgentPort, RewindPlan, RewindResult, RewindScope } from "../port/port";

export interface Rewinding {
  onPrepareRewind: (turn: number, scope: RewindScope) => Promise<RewindPlan>;
  onCommitRewind: (planId: string) => Promise<RewindResult>;
  onUndoRewind: (transactionId: string) => Promise<void>;
  onResend: (turn: number, text: string) => Promise<void>;
  onPrepareFileRevert: (path: string) => Promise<RewindPlan>;
  onCommitFileRevert: (planId: string, resolution?: string) => Promise<RewindResult>;
}

/** Editing a message is a rewind composed with a send, not a capability of its
 *  own: the kernel already owns both halves and holds no state between them. */
export function useRewind(
  port: MaybeRefOrGetter<AgentPort>,
  reloadSession: () => void,
  send: MaybeRefOrGetter<(text: string) => Promise<unknown>>,
): Rewinding {
  const onPrepareRewind = (turn: number, scope: RewindScope) => toValue(port).prepareRewind(turn, scope);

  const onCommitRewind = async (planId: string) => {
    const result = await toValue(port).commitRewind(planId);
    reloadSession();
    return result;
  };

  const onUndoRewind = (transactionId: string) => toValue(port).undoRewind(transactionId).then(reloadSession);

  const onResend = async (turn: number, text: string) => {
    const agent = toValue(port);
    const plan = await agent.prepareRewind(turn, "conversation");
    if (!plan.canConversation) throw new Error(plan.disabledReason || t("app.this_turn_cannot_be_rewound"));
    const result = await agent.commitRewind(plan.planId);
    reloadSession();
    // The commit can touch files and leave the conversation standing, which
    // would send the message a second time rather than replace it.
    if (!result.conversationOk) throw new Error(result.error || t("app.the_conversation_was_not_rewound"));
    await toValue(send)(text);
  };

  const onPrepareFileRevert = (path: string) => toValue(port).prepareFileRevert(path);

  const onCommitFileRevert = (planId: string, resolution?: string) => toValue(port).commitFileRevert(planId, resolution);

  return { onPrepareRewind, onCommitRewind, onUndoRewind, onResend, onPrepareFileRevert, onCommitFileRevert };
}