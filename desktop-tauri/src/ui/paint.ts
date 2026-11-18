import { computed, onWatcherCleanup, ref, toValue, watch, type MaybeRefOrGetter, type Ref, type ShallowRef } from "vue";
import { effectsMode, onEffectsChange, type EffectsMode } from "../state/prefs";
import { useExternal } from "../state/external";
import { adopt as adoptLang } from "../i18n";
import type { Appearance as Look, ThemePack } from "../port/port";
import type { HubPort, RuntimeView } from "../port/hub";
import { apply as applyLook } from "./look";
import { apply as applyThemePack } from "./theme";
import { trailing } from "./trailing";
import { onThemeScheduleChange, themeSchedule } from "../state/themeSchedule";
import { nextScheduledBoundary, scheduledScheme } from "./scheduledTheme";
import { host } from "../port/host";

export interface Paint {
  theme: Ref<string>;
  setTheme: (v: string) => void;
  // The resolved colour mode, independent of the selected preference.
  scheme: Ref<"light" | "dark">;
  contrast: Ref<string>;
  setContrast: (v: string) => void;
  weight: Ref<string>;
  setWeight: (v: string) => void;
  effects: ShallowRef<EffectsMode>;
  look: Ref<Look>;
  onLook: (next: Look) => void;
  pack: Ref<ThemePack | null>;
  reloadThemes: () => void;
}

/** How this window is painted: the scheme it resolved to, the pack behind it,
 *  and the reader's own size and contrast. One concern, and the only one that
 *  writes `documentElement`'s dataset, so nothing else has to know the order
 *  a pack and a look have to be applied in. */
export function usePaint(hub: HubPort, runtimes: MaybeRefOrGetter<RuntimeView[]>, running: MaybeRefOrGetter<boolean>, onError: (e: unknown) => void): Paint {
  const theme = ref((() => {
    try {
      const saved = localStorage.getItem("rx-theme");
      return saved && ["auto", "light", "dark", "scheduled"].includes(saved) ? saved : "auto";
    } catch { return "auto"; }
  })());
  const setTheme = (v: string) => { theme.value = v; };
  const schedule = useExternal(onThemeScheduleChange, themeSchedule);
  const scheme = ref<"light" | "dark">("dark");
  // "" means never chosen, which is what lets the system's own contrast setting
  // decide. Any explicit pick wins over it from then on.
  const contrast = ref(localStorage.getItem("rx-contrast") ?? "");
  const setContrast = (v: string) => { contrast.value = v; };
  // "" means follow the language: a Chinese interface is one weight bolder than
  // a Western one by right, and the stylesheet defaults it by :lang.
  const weight = ref(localStorage.getItem("rx-weight") ?? "");
  const setWeight = (v: string) => { weight.value = v; };
  const effects = useExternal(onEffectsChange, effectsMode);
  const pack = ref<ThemePack | null>(null);
  const look = ref<Look>({});

  const lookPort = computed(() => hub.machinePort());

  const reloadThemes = () => {
    lookPort.value
      ?.themes()
      .then((list) => { pack.value = list.find((p) => p.active) ?? null; })
      .catch(onError);
  };
  watch(() => toValue(runtimes), (rts) => { if (rts.some((rt) => !rt.host)) reloadThemes(); }, { immediate: true });

  watch([lookPort, () => toValue(runtimes)], ([lp, rts]) => {
    if (!rts.some((rt) => !rt.host)) return;
    let alive = true;
    lp
      ?.appearance()
      .then((next) => {
        if (!alive) return;
        // The kernel's copy is the authority across machines; adopt reloads if
        // this window booted in the wrong language from a stale local cache.
        if (adoptLang(next.language)) return;
        look.value = next;
      })
      .catch(() => {});
    onWatcherCleanup(() => { alive = false; });
  }, { immediate: true });

  // The control moves now and the config catches up: a size or a colour that
  // waits on a round trip reads as a dead click. The kernel's answer is still
  // what is kept, since it clamps what the slider sent.
  const saveLook = computed(() => {
    const lp = lookPort.value;
    return lp ? trailing((next: Look) => lp.saveAppearance(next), (v) => { look.value = v; }, onError) : null;
  });
  const onLook = (next: Look) => {
    look.value = next;
    saveLook.value?.(next);
  };

  // A pack carries a light and a dark set, so it is repainted with the scheme
  // rather than once at load: switching the OS to dark has to move both. The
  // running flag rides along because a pack's picture recedes while a turn is
  // in flight — the transition is in CSS, this only moves the target.
  watch([theme, schedule, pack, () => toValue(running), look, contrast], () => {
    const mq = matchMedia("(prefers-color-scheme: dark)");
    let timer: number | undefined;
    const paint = () => {
      clearTimeout(timer);
      const clock = new Date();
      const th = theme.value;
      const now = th === "scheduled" ? scheduledScheme(schedule.value, clock) :
        th === "auto" ? (mq.matches ? "dark" : "light") : th;
      document.documentElement.dataset.theme = now;
      scheme.value = now as "light" | "dark";
      // The active pack is the same state shown by the appearance picker.
      // Applying `null` here made activation persist in the kernel while the
      // shell deliberately kept drawing the default palette, so a successful
      // click looked broken until forever. Keep the authored Studio layout,
      // but let the selected pack supply its documented colour/background
      // tokens immediately.
      applyThemePack(pack.value, now as "light" | "dark", toValue(running), contrast.value);
      // After the pack, never before: size and type are the reader's, and a
      // palette someone else authored does not get to overrule them.
      applyLook(look.value, toValue(running));
      try {
        localStorage.setItem("rx-theme", th);
        localStorage.setItem("rx-theme-resolved", now);
      } catch { /* this window keeps its choice */ }
      host().syncTheme(th, now as "light" | "dark");
      if (th === "scheduled") {
        const delay = nextScheduledBoundary(schedule.value, clock);
        if (delay !== null) timer = window.setTimeout(paint, delay);
      }
    };
    paint();
    mq.addEventListener("change", paint);
    if (theme.value === "scheduled") {
      window.addEventListener("focus", paint);
      window.addEventListener("pageshow", paint);
      document.addEventListener("visibilitychange", paint);
    }
    onWatcherCleanup(() => {
      clearTimeout(timer);
      mq.removeEventListener("change", paint);
      window.removeEventListener("focus", paint);
      window.removeEventListener("pageshow", paint);
      document.removeEventListener("visibilitychange", paint);
    });
  }, { immediate: true });

  watch([contrast, weight], ([c, w]) => {
    if (c) document.documentElement.dataset.contrast = c;
    else delete document.documentElement.dataset.contrast;
    localStorage.setItem("rx-contrast", c);
    if (w) document.documentElement.dataset.weight = w;
    else delete document.documentElement.dataset.weight;
    localStorage.setItem("rx-weight", w);
  }, { immediate: true });

  watch(effects, (fx) => {
    if (fx === "reduced") document.documentElement.dataset.effects = fx;
    else delete document.documentElement.dataset.effects;
  }, { immediate: true });

  return { theme, setTheme, scheme, contrast, setContrast, weight, setWeight, effects, look, onLook, pack, reloadThemes };
}