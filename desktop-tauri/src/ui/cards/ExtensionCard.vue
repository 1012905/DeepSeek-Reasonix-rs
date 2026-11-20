<script setup lang="ts">
import { computed, defineComponent, h, ref } from "vue";
import { t } from "../../i18n";
import type { ExtensionFormField, ExtensionSurface } from "../../port/wire";
import ExtensionView from "./ExtensionView.vue";
import LazyMarkdown from "../LazyMarkdown.vue";
import Sym from "../Sym.vue";

const props = defineProps<{
  ext: ExtensionSurface;
  onInvoke?: (name: string) => void;
  onSubmit?: (pluginId: string, surfaceId: string, values: Record<string, unknown>) => void | Promise<void>;
}>();

// An extension publishes data, not markup — so severity arrives as its own
// word and gets the same left colour bar every other card in this UI uses.
function lvl(severity?: string): string | undefined {
  switch (severity) {
    case "error":
    case "critical":
      return "err";
    case "warn":
    case "warning":
      return "warn";
    case "ok":
    case "success":
      return "ok";
    default:
      return undefined;
  }
}

const Bar = defineComponent({
  name: "ExtensionBar",
  props: { value: { type: Number, required: true } },
  setup(props) {
    const pct = computed(() => Math.max(0, Math.min(1, props.value)) * 100);
    return () => h("span", { class: "extbar", role: "progressbar", "aria-valuenow": Math.round(pct.value) }, [
      h("i", { style: { width: `${pct.value}%` } }),
    ]);
  },
});

// A declared default is the extension's answer until the user changes it, so
// the form opens pre-filled rather than empty.
function seed(fields?: ExtensionFormField[]): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const f of fields ?? []) {
    if (f.default !== undefined) out[f.key] = f.default;
    else if (f.kind === "multiselect") out[f.key] = [];
    else if (f.kind === "confirm") out[f.key] = false;
  }
  return out;
}

function asList(v: unknown): string[] {
  return Array.isArray(v) ? v.map(String) : [];
}

function empty(v: unknown): boolean {
  if (v === undefined || v === null) return true;
  if (Array.isArray(v)) return v.length === 0;
  if (typeof v === "string") return v.trim() === "";
  if (typeof v === "boolean") return v === false;
  return false;
}

const draft = ref<Record<string, unknown>>(seed(props.ext.form?.fields));
const submission = ref<"idle" | "pending" | "sent">("idle");
const sent = computed(() => submission.value === "sent");
const pending = computed(() => submission.value === "pending");
const sealed = computed(() => sent.value || pending.value);

const values = computed(() => {
  const out = seed(props.ext.form?.fields);
  for (const f of props.ext.form?.fields ?? []) {
    if (Object.hasOwn(draft.value, f.key)) out[f.key] = draft.value[f.key];
  }
  return out;
});

const set = (key: string, v: unknown) => {
  draft.value = { ...draft.value, [key]: v };
};

const missing = computed(() => (props.ext.form?.fields ?? []).filter((f) => f.required && empty(values.value[f.key])));

const submitLabel = computed(() =>
  pending.value ? t("common.submitting") : missing.value.length ? t("chat.submit_n_still_to_fill", { n: missing.value.length }) : t("chat.submit"),
);

const submit = async () => {
  if (submission.value !== "idle" || missing.value.length || !props.onSubmit) return;
  submission.value = "pending";
  try {
    await props.onSubmit(props.ext.pluginId, props.ext.surfaceId, values.value);
    submission.value = "sent";
  } catch {
    submission.value = "idle";
  }
};

const picked = (f: ExtensionFormField) =>
  f.kind === "multiselect" ? asList(values.value[f.key]) : values.value[f.key] === undefined ? [] : [String(values.value[f.key])];
const isPicked = (f: ExtensionFormField, opt: string) => picked(f).includes(opt);
const pick = (f: ExtensionFormField, opt: string) => {
  if (sealed.value) return;
  if (f.kind !== "multiselect") {
    set(f.key, opt);
    return;
  }
  const current = picked(f);
  set(f.key, current.includes(opt) ? current.filter((p) => p !== opt) : [...current, opt]);
};
const toggleConfirm = (f: ExtensionFormField) => {
  if (sealed.value) return;
  set(f.key, values.value[f.key] !== true);
};
const onInput = (f: ExtensionFormField, e: Event) => set(f.key, (e.target as HTMLInputElement).value);
const textValue = (f: ExtensionFormField) =>
  values.value[f.key] === undefined || values.value[f.key] === null ? "" : String(values.value[f.key]);
</script>

<template>
  <div v-if="ext.code" class="restore-notice" role="status" :data-code="ext.code">
    <span>{{ t("chat.extension_surface_expired") }}</span>
    <span class="why">{{ ext.pluginId }}/{{ ext.surfaceId }}</span>
  </div>
  <div v-else class="call" data-k="extension">
    <div class="g">
      <Sym icon="plug" />
    </div>
    <div class="c">
      <div class="hl">
        <span class="nm">{{ ext.card?.title || ext.form?.title || ext.pluginId }}</span>
        <span class="tag">{{ t("chat.extension_id", { id: ext.pluginId }) }}</span>
      </div>
      <div class="out">
        <!-- A composed surface draws itself from primitives. It comes first
             because an extension that sent one meant it as the surface, not
             as a decoration on the fixed fields. -->
        <ExtensionView v-if="ext.view" :body="ext.view.body" :on-action="onInvoke" />

        <div v-if="ext.status" class="finds">
          <div class="find" :data-lvl="lvl(ext.status.severity)">
            <span class="t">{{ ext.status.label }}</span>
            <span v-if="ext.status.detail" class="why">{{ ext.status.detail }}</span>
            <Bar v-if="ext.status.progress !== undefined" :value="ext.status.progress" />
          </div>
        </div>

        <div v-if="ext.notification" class="finds">
          <div class="find" :data-lvl="lvl(ext.notification.severity)">
            <span class="t">{{ ext.notification.title }}</span>
            <span v-if="ext.notification.body" class="why">{{ ext.notification.body }}</span>
          </div>
        </div>

        <div v-if="ext.card" class="extcard">
          <LazyMarkdown v-if="ext.card.markdown" :text="ext.card.markdown" />
          <div v-else-if="ext.card.text" class="exttext">{{ ext.card.text }}</div>
          <Bar v-if="ext.card.progress !== undefined" :value="ext.card.progress" />
          <dl v-if="ext.card.fields?.length" class="extkv">
            <div v-for="f in ext.card.fields" :key="f.key">
              <dt>{{ f.key }}</dt>
              <dd>{{ f.value }}</dd>
            </div>
          </dl>
          <div v-if="ext.card.actions?.length" class="extacts">
            <button v-for="a in ext.card.actions" :key="a.actionId" data-action="extensions.invoke" class="btn"
              @click="onInvoke?.(`/${ext.pluginId}:${a.actionId}`)">
              {{ a.label || a.actionId }}
            </button>
          </div>
        </div>

        <div v-if="ext.form" class="extform" :data-sealed="sent ? '' : undefined" :aria-busy="pending">
          <div class="why">{{ t("chat.this_is_a_question_from", { id: ext.pluginId }) }}</div>
          <div v-if="ext.form.message" class="exttext">{{ ext.form.message }}</div>
          <template v-for="f in ext.form.fields" :key="f.key">
            <div v-if="f.kind === 'select' || f.kind === 'multiselect'" class="extfield">
              <div class="extlabel">
                {{ f.label || f.key }}
                <i v-if="f.required" class="req">*</i>
              </div>
              <div class="opts">
                <button v-for="opt in f.options ?? []" :key="opt" class="opt"
                  :data-multi="f.kind === 'multiselect' ? '' : undefined"
                  :data-on="isPicked(f, opt) ? '' : undefined" :disabled="sealed" @click="pick(f, opt)">
                  <span class="mark" />
                  <span class="txt">
                    <span class="lb">{{ opt }}</span>
                  </span>
                </button>
              </div>
            </div>
            <div v-else-if="f.kind === 'confirm'" class="extfield">
              <div class="opts">
                <button class="opt" data-multi="" :data-on="values[f.key] === true ? '' : undefined" :disabled="sealed" @click="toggleConfirm(f)">
                  <span class="mark" />
                  <span class="txt">
                    <span class="lb">
                      {{ f.label || f.key }}
                      <i v-if="f.required" class="req">*</i>
                    </span>
                  </span>
                </button>
              </div>
            </div>
            <div v-else class="extfield">
              <div class="extlabel">
                {{ f.label || f.key }}
                <i v-if="f.required" class="req">*</i>
              </div>
              <input :aria-label="f.label || f.key" :aria-required="f.required || undefined"
                :value="textValue(f)" :readonly="sealed" @input="onInput(f, $event)" />
            </div>
          </template>
          <div v-if="!sent" class="extfoot">
            <button data-action="extensions.submit" class="btn" data-primary=""
              :disabled="missing.length > 0 || pending || !onSubmit" @click="submit">
              {{ submitLabel }}
            </button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
