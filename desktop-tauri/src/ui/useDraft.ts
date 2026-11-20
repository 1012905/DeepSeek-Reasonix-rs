import { onMounted, onScopeDispose, ref, toValue, watch, type MaybeRefOrGetter } from "vue";
import { readDraft, writeDraft } from "./drafts";

type Setter<T> = (next: T | ((prev: T) => T)) => void;

export function useDraft(draftKey: MaybeRefOrGetter<string>, submitting: MaybeRefOrGetter<boolean>) {
  const text = ref(readDraft(toValue(draftKey)));
  const setText: Setter<string> = (next) => { text.value = typeof next === "function" ? next(text.value) : next; };
  let keyRef = toValue(draftKey);
  let textRef = text.value;
  let pendingRef: { text: string; key: string } | null = null;
  let skipWrite = false;
  // The submitted line is cleared from the mirror without clearing the field:
  // the composer owns what it shows, and a flush must not write it back.
  watch(text, (v) => { textRef = v; }, { immediate: true, flush: "sync" });

  watch(() => toValue(draftKey), (key) => {
    if (keyRef === key) return;
    const previous = keyRef;
    writeDraft(previous, pendingRef?.text ?? textRef);
    keyRef = key;
    if (!key) return;
    // A newly known session key must not replay the draft being submitted.
    if (pendingRef) {
      writeDraft(key, pendingRef.text);
      skipWrite = true;
      return;
    }
    const saved = readDraft(key);
    if (!previous && !saved && textRef) writeDraft(key, textRef);
    else text.value = saved;
    skipWrite = true;
  }, { immediate: true });

  watch([() => toValue(draftKey), text, () => toValue(submitting)], (_v, _was, onCleanup) => {
    const key = toValue(draftKey);
    if (!key || skipWrite || toValue(submitting)) {
      skipWrite = false;
      return;
    }
    const timer = window.setTimeout(() => writeDraft(key, text.value), 250);
    onCleanup(() => window.clearTimeout(timer));
  }, { immediate: true });

  const flush = () => writeDraft(keyRef, pendingRef?.text ?? textRef);
  onMounted(() => window.addEventListener("pagehide", flush));
  onScopeDispose(() => {
    window.removeEventListener("pagehide", flush);
    flush();
  });

  return {
    text,
    setText,
    beginSubmit: (draft: string) => { pendingRef = { text: draft, key: keyRef }; },
    finishSubmit: (sent: boolean) => {
      const pending = pendingRef;
      pendingRef = null;
      if (sent) {
        if (pending?.key && pending.key !== keyRef) writeDraft(pending.key, "");
        textRef = "";
        writeDraft(keyRef, "");
      }
    },
  };
}