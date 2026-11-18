// A Chinese or Japanese IME confirms its candidate with Enter, and several of
// them end the composition just before that keydown arrives — so isComposing
// alone is already false and the confirm reads as "send". This window covers
// the reordering; it stays short so a deliberate second Enter still lands.
const GRACE_MS = 100;

// useIme reports whether a key event belongs to an input method rather than to
// the composer. Every key the composer acts on has to ask, or picking a word
// sends a half-written message.
export function useIme() {
  let composing = false;
  let endedAt = 0;

  const isIme = (e: { isComposing?: boolean; keyCode?: number }) => {
    if (composing || e.isComposing === true || e.keyCode === 229) return true;
    return endedAt > 0 && Date.now() - endedAt < GRACE_MS;
  };

  return {
    isIme,
    handlers: {
      onCompositionStart: () => {
        composing = true;
      },
      onCompositionEnd: () => {
        composing = false;
        endedAt = Date.now();
      },
    },
  };
}