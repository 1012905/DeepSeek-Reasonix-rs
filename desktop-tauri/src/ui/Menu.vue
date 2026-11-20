<script lang="ts">
import { computed, defineComponent, h, nextTick, ref, Teleport, useAttrs, watch, type PropType, type VNodeChild } from "vue";
import { CircleAlert, CircleDot, LoaderCircle } from "lucide-vue-next";
import { t } from "../i18n";
import { placeInViewport, useFollow, zoom } from "./place";
import { folded } from "./viewport";
import { hasVendorMark, VendorMark } from "./vendormarks.vue";
import StudioIcon from "./StudioIcon.vue";

export interface MenuStatus {
  label: string;
  tone: "warning" | "positive" | "neutral" | "checking";
  detail?: string;
}

export interface MenuItem {
  value: string;
  label: string;
  desc?: string;
  right?: string;
  status?: MenuStatus;
  // Dense choice menus sometimes need one short machine-facing label beside
  // the human one (for example "Deep  High") without pushing that label to
  // the far edge of the row.
  meta?: string;
  badge?: string;
  strength?: number;
  recommended?: boolean;
  plain?: boolean;
  divide?: boolean;
  // Pinned under the scrolling rows instead of scrolling with them.
  foot?: boolean;
  // A group caption. It labels the rows under it and cannot be chosen, so it
  // is not a button — a menu whose headings take focus is a menu you arrow
  // through twice.
  header?: boolean;
  disabled?: boolean;
  // A letter chip leading the row or heading, standing in for a vendor mark.
  providerIcon?: { tone: number };
  // A vendor slug; when artwork exists for it, it replaces the letter chip.
  mark?: string;
  // A switch rather than a choice: picking it flips this state, and the row
  // says which state it is in instead of being marked as the current value.
  toggle?: boolean;
}

// Where a list stops fitting the menu's own height cap. Past it a scrollbar
// alone still means hunting — one gateway account can publish a hundred
// models — and below it the field would be one more thing to look past.
const FILTER_FROM = 10;

type MenuData = Record<string, string | ((value: string) => string)>;

// data-* rides through to the item that raises the pick, the way Switch and Seg
// carry theirs: the action's identity is written at the call site, and the
// answer this menu gives is the item's own value.
function resolveData(id: MenuData, value: string): Record<string, string> {
  const data: Record<string, string> = {};
  for (const [key, entry] of Object.entries(id)) {
    if (!key.startsWith("data-")) continue;
    data[key] = typeof entry === "function" ? entry(value) : entry;
  }
  return data;
}

const Lead = defineComponent({
  name: "MenuLead",
  props: { it: { type: Object as PropType<MenuItem>, required: true } },
  setup(props) {
    return () => {
      const mark = props.it.mark;
      if (hasVendorMark(mark)) return h(VendorMark, { slug: mark });
      const icon = props.it.providerIcon;
      return icon ? h(ProviderIcon, icon) : null;
    };
  },
});

const ProviderIcon = defineComponent({
  name: "MenuProviderIcon",
  props: { tone: { type: Number, required: true } },
  setup(props) {
    return () => h("i", { class: "mi-mono", "data-tone": props.tone, style: { "--tone": `var(--cat-${props.tone + 1})` }, "aria-hidden": "true" }, [
      h(StudioIcon, { name: "server" }),
    ]);
  },
});

export const MenuRow = defineComponent({
  name: "MenuRow",
  props: {
    it: { type: Object as PropType<MenuItem>, required: true },
    id: { type: Object as PropType<MenuData>, default: () => ({}) },
    current: { type: String },
    lead: { type: String },
    onTake: { type: Function as PropType<(value: string) => void>, required: true },
  },
  setup(props) {
    return () => {
      const it = props.it;
      return h("button", {
        ...resolveData(props.id, it.value),
        "data-value": it.value,
        class: it.plain ? "mi plain" : "mi",
        role: it.toggle === undefined ? "menuitem" : "menuitemcheckbox",
        "aria-checked": it.toggle,
        "aria-current": it.value === props.current ? true : undefined,
        "aria-description": it.status?.detail,
        title: it.status?.detail,
        disabled: it.disabled,
        "data-on": it.value === props.current ? "" : undefined,
        "data-lead": it.value === props.lead ? "" : undefined,
        "data-strength": it.strength,
        "data-recommended": it.recommended ? "" : undefined,
        onClick: () => props.onTake(it.value),
      }, [
        it.providerIcon || hasVendorMark(it.mark) ? h(Lead, { it }) : h("span", { class: "dot" }),
        h("span", { class: "tx" }, [
          h("span", { class: "studio-item-line" }, [
            h("span", { class: "lb" }, it.label),
            it.meta && h("span", { class: "studio-item-meta" }, it.meta),
            it.badge && h("span", { class: "studio-item-badge" }, it.badge),
          ]),
          it.desc && h("span", { class: "ds" }, it.desc),
        ]),
        it.strength != null && h("span", { class: "studio-effort-strength", "aria-hidden": "true" }, [
          it.recommended
            ? h(StudioIcon, { name: "spark" })
            : [1, 2, 3, 4].map((level) => h("i", { key: level, "data-on": level <= (it.strength ?? 0) ? "" : undefined })),
        ]),
        it.toggle !== undefined && h("span", { class: "studio-item-switch", "aria-hidden": "true" }),
        it.right && h("span", { class: "rt" }, it.right),
        it.status && h("span", { class: "menu-item-status", "data-tone": it.status.tone }, [
          it.status.tone === "warning"
            ? h(CircleAlert, { "aria-hidden": "true" })
            : it.status.tone === "checking"
              ? h(LoaderCircle, { "aria-hidden": "true" })
              : h(CircleDot, { "aria-hidden": "true" }),
          it.status.label,
        ]),
        it.value === props.current && h(StudioIcon, { name: "check", class: "menu-current" }),
      ]);
    };
  },
});

export const Picker = defineComponent({
  name: "Picker",
  inheritAttrs: false,
  props: {
    label: { type: null as unknown as PropType<VNodeChild>, required: true },
    items: { type: Array as PropType<MenuItem[]>, required: true },
    current: { type: String },
    onPick: { type: Function as PropType<(value: string) => void>, required: true },
    place: { type: String as PropType<"top" | "bottom">, required: true },
    // Which edge of the trigger the menu lines up with. It is stated here because
    // the menu no longer sits under a positioned ancestor a stylesheet can reach.
    align: { type: String as PropType<"start" | "end">, default: "start" },
    className: { type: String },
    title: { type: String },
    // The choice already made is being applied and has not landed. Marked on the
    // trigger alone: whatever else is on that shelf is still the reader's to
    // change while this one waits.
    pending: { type: Boolean },
    triggerAction: { type: String },
    ariaPressed: { type: Boolean, default: undefined },
    ariaLabel: { type: String },
    ariaDescribedBy: { type: String },
    wrapClassName: { type: String },
    menuClassName: { type: String },
    menuTitle: { type: null as unknown as PropType<VNodeChild> },
    // Opening is when the reader asks what the choices are, so a menu whose rows
    // come from elsewhere re-reads them here rather than showing its mount's copy.
    onOpen: { type: Function as PropType<() => void> },
    // The field is the menu's way in rather than its answer to length: a model
    // list is searched for by name however few entries it has.
    searchAlways: { type: Boolean },
    searchPlaceholder: { type: String },
    // Drawn under the field and above the rows: whatever the caller offers for
    // narrowing the list, owned by the caller so its inputs stay its own.
    head: { type: null as unknown as PropType<VNodeChild> },
    aside: { type: Function as PropType<(close: () => void) => VNodeChild> },
    asideClassName: { type: String },
    stayOpen: { type: Function as PropType<(value: string) => boolean> },
    sheet: { type: Boolean },
  },
  setup(props) {
    const attrs = useAttrs();
    const open = ref(false);
    const query = ref("");
    const wrap = ref<HTMLElement | null>(null);
    const menu = ref<HTMLElement | null>(null);
    const btn = ref<HTMLButtonElement | null>(null);
    const find = ref<HTMLInputElement | null>(null);

    const menuData = (): MenuData => {
      const out: MenuData = {};
      for (const [key, value] of Object.entries(attrs)) {
        if (key.startsWith("data-") && (typeof value === "string" || typeof value === "function")) out[key] = value as string | ((value: string) => string);
      }
      return out;
    };
    const choosable = computed(() => props.items.filter((it) => !it.header && !it.plain).length);
    const filtering = computed(() => !!props.searchAlways || choosable.value > FILTER_FROM);
    const shown = computed(() => (filtering.value ? matching(props.items, query.value) : props.items));
    // What Enter takes while the field still has focus. Only shown once the
    // query narrows something, or every menu would open pre-selected.
    const lead = computed(() => (query.value ? shown.value.find((it) => !it.header && !it.plain)?.value : undefined));
    const none = computed(() => filtering.value && !shown.value.some((it) => !it.header && !it.plain));
    const pinned = computed(() => shown.value.some((it) => it.foot));

    watch([open, filtering], ([isOpen], _was, onCleanup) => {
      if (!isOpen) {
        query.value = "";
        return;
      }
      void nextTick(() => {
        if (!open.value) return;
        if (filtering.value) find.value?.focus();
        else menu.value?.querySelector<HTMLElement>("button.mi:not(:disabled)")?.focus();
        menu.value?.querySelector<HTMLElement>("button.mi[data-on]")?.scrollIntoView({ block: "nearest" });
      });
      // The menu is not inside the wrapper any more, so without asking it too a
      // press on one of its own rows would read as a press outside.
      const onDown = (event: MouseEvent) => {
        const at = event.target as Node;
        if (!wrap.value?.contains(at) && !menu.value?.contains(at)) open.value = false;
      };
      // Esc unwinds one layer at a time: the menu first, the run only once no
      // menu is left, so the capture phase has to stop it reaching the app.
      const onKey = (event: KeyboardEvent) => {
        if (event.key !== "Escape") return;
        event.stopPropagation();
        open.value = false;
        btn.value?.focus();
      };
      addEventListener("mousedown", onDown);
      addEventListener("keydown", onKey, true);
      onCleanup(() => {
        removeEventListener("mousedown", onDown);
        removeEventListener("keydown", onKey, true);
      });
    }, { flush: "post", immediate: true });

    // Measured rather than anchored, because the menu is rendered into the body
    // and has no positioned ancestor left to hang on. It is rendered there so no
    // ancestor can take its z-index down with it: a stacking context anywhere
    // above — an entrance animation's fill transform, a card's paint containment
    // — traps the menu at that ancestor's level and the transcript draws over it.
    const put = () => {
      const anchor = btn.value?.getBoundingClientRect();
      const el = menu.value;
      if (!anchor || !el || el.hidden) return;
      if (props.sheet && folded("phone")) {
        el.style.left = "";
        el.style.top = "";
        return;
      }
      // The layout box, not the painted one: the entrance transition is running
      // while this measures, and its scaleY would place the menu by a size it is
      // on its way out of. offsetWidth is in CSS pixels where a client rect is on
      // screen, so it is scaled to meet the anchor's units.
      const s = zoom();
      const box = { width: el.offsetWidth * s, height: el.offsetHeight * s };
      const under = anchor.bottom + 7;
      const over = anchor.top - box.height - 9;
      const down = props.place === "top";
      const fits = down ? under + box.height <= innerHeight - 6 : over >= 6;
      const at = placeInViewport(
        { x: props.align === "end" ? anchor.right - box.width : anchor.left, y: fits === down ? under : over },
        box,
        { width: innerWidth, height: innerHeight },
        s,
      );
      el.style.left = `${at.left}px`;
      el.style.top = `${at.top}px`;
    };
    useFollow(open, put, () => shown.value.length, { anchor: btn, container: menu });

    // Headings are divs and never take focus, so walking the buttons is what
    // keeps one arrow press from landing on nothing.
    const arrows = (event: KeyboardEvent) => {
      const typed = event.key.length === 1 && event.key !== " " && !event.ctrlKey && !event.metaKey && !event.altKey;
      if (filtering.value && typed && event.target !== find.value) {
        find.value?.focus();
        return;
      }
      const rowsOf = (inAside: boolean) => Array.from(menu.value?.querySelectorAll<HTMLElement>("button.mi:not(:disabled)") ?? [])
        .filter((el) => !!el.closest(".menu-aside") === inAside);
      const here = document.activeElement as HTMLElement | null;
      const inAside = !!here?.closest(".menu-aside");
      if (props.aside && (event.key === "ArrowRight" || event.key === "ArrowLeft") && here !== find.value) {
        const toAside = event.key === "ArrowRight";
        if (toAside === inAside) return;
        const column = rowsOf(toAside);
        const target = column.find((el) => el.hasAttribute("data-on")) ?? column[0];
        if (!target) return;
        event.preventDefault();
        target.focus();
        return;
      }
      if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
      const all = rowsOf(inAside);
      if (!all.length) return;
      const i = all.indexOf(here as HTMLElement);
      if (i < 0) {
        if (event.key !== "ArrowDown") return;
        event.preventDefault();
        all[0].focus();
        return;
      }
      const to = event.key === "ArrowDown" ? i + 1 : i - 1;
      if (to < 0 && filtering.value && !inAside) {
        event.preventDefault();
        find.value?.focus();
        return;
      }
      if (to < 0 || to >= all.length) return;
      event.preventDefault();
      all[to].focus();
    };

    // Taking a value closes the menu the way Escape does, so the focus goes back
    // the same way too. Without it the row that had focus is unmounted under the
    // caret, which drops to the document — and the next Tab restarts at the top
    // of the page, so choosing costs a keyboard user their place and cancelling
    // does not.
    const take = (value: string) => {
      if (props.stayOpen?.(value)) {
        props.onPick(value);
        return;
      }
      open.value = false;
      btn.value?.focus();
      props.onPick(value);
    };
    const closeMenu = () => {
      open.value = false;
      btn.value?.focus();
    };

    const row = (it: MenuItem, i: number): VNodeChild[] => {
      const nodes: VNodeChild[] = [];
      if (it.divide && i > 0 && !it.foot) nodes.push(h("div", { key: `div:${it.value}`, class: "div" }));
      nodes.push(it.header
        ? h("div", { key: `head:${it.value}`, class: "mi head" }, [
            h(Lead, { it }),
            h("span", { class: "lb" }, it.label),
            it.right && h("span", { class: "rt" }, it.right),
          ])
        : h(MenuRow, { key: `row:${it.value}`, it, id: menuData(), current: props.current, lead: lead.value, onTake: take }));
      return nodes;
    };

    return () => {
      const menuClass = [
        props.place === "top" ? "menu portmenu pm-down" : "menu portmenu pm-up",
        props.aside ? "has-aside" : "",
        props.sheet ? "menu-sheet" : "",
        props.menuClassName,
      ].filter(Boolean).join(" ");
      const children: VNodeChild[] = [];
      if (props.menuTitle) children.push(h("div", { class: "studio-picker-title" }, [props.menuTitle]));
      if (filtering.value) {
        children.push(h("input", {
          ref: find,
          class: "mfind",
          value: query.value,
          placeholder: props.searchPlaceholder ?? t("app.filter_n_entries", { n: choosable.value }),
          "aria-label": t("app.filter"),
          spellcheck: false,
          onInput: (event: Event) => { query.value = (event.target as HTMLInputElement).value; },
          onKeydown: (event: KeyboardEvent) => {
            if (event.key !== "Enter" || !lead.value) return;
            event.preventDefault();
            take(lead.value);
          },
        }));
      }
      if (props.head) children.push(props.head);
      if (none.value) children.push(h("div", { class: "mnone" }, t("app.nothing_matches_2")));
      if (pinned.value) {
        children.push(h("div", { class: "menu-rows" }, shown.value.filter((it) => !it.foot).flatMap(row)));
        children.push(...shown.value.filter((it) => it.foot).flatMap(row));
      } else {
        children.push(...shown.value.flatMap(row));
      }
      if (props.aside) {
        children.push(h("div", { class: ["menu-aside", props.asideClassName].filter(Boolean).join(" "), role: "group" }, [props.aside(closeMenu)]));
      }
      return [
        h("div", { class: ["picker", props.wrapClassName].filter(Boolean).join(" "), ref: wrap }, [
          h("button", {
            ref: btn,
            class: props.className,
            "data-action": props.triggerAction,
            "aria-haspopup": "menu",
            "aria-expanded": open.value,
            "aria-pressed": props.ariaPressed,
            "aria-label": props.ariaLabel,
            "aria-describedby": props.ariaDescribedBy,
            "data-pending": props.pending ? "" : undefined,
            title: props.title,
            onClick: () => {
              if (!open.value) props.onOpen?.();
              open.value = !open.value;
            },
          }, [props.label]),
        ]),
        h(Teleport, { to: "body" }, [
          h("div", { ref: menu, class: menuClass, role: "menu", hidden: !open.value, onKeydown: arrows }, children),
        ]),
      ];
    };
  },
});

// Group order is kept rather than ranked by relevance: a list that reorders
// itself as you type moves the row out from under the cursor.
function matching(items: MenuItem[], query: string): MenuItem[] {
  const words = query.split(/\s+/).map(normalize).filter(Boolean);
  if (!words.length) return items;
  // A row is searchable by the heading above it too, so the account's name
  // finds models whose own names never mention it.
  let head = "";
  const hit = items.map((it) => {
    if (it.header) {
      head = normalize(`${it.label} ${it.right ?? ""}`);
      return false;
    }
    // Actions are not data — filtering them out would take "Open another folder…"
    // away exactly when the query found nothing and the user needs it.
    if (it.plain) return false;
    const hay = `${head}|${normalize(`${it.label} ${it.right ?? ""} ${it.meta ?? ""} ${it.desc ?? ""}`)}`;
    return words.every((w) => hay.includes(w));
  });
  return items.filter((it, i) => {
    if (it.plain) return true;
    if (!it.header) return hit[i];
    for (let j = i + 1; j < items.length && !items[j].header; j++) {
      if (hit[j]) return true;
    }
    return false;
  });
}

// Providers punctuate names inconsistently, so "gpt4" has to find "gpt-4o":
// the separators carry nothing worth matching on.
function normalize(s: string): string {
  return s.toLowerCase().replace(/[\s\-_/.]+/g, "");
}

export default Picker;
</script>
