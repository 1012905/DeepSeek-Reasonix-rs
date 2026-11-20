<script lang="ts">
import type { TreeSession, TreeWorkspace } from "../port/hub";

/** Choosing the project a conversation moves to. It says what travels (the
 *  conversation and the labels the person set) and what does not (what the old
 *  project allowed: the new one decides that for itself), and that the paths
 *  recorded inside the conversation are left as they were. */
export interface MoveSessionDialogProps {
  session: TreeSession;
  label: string;
  projects: TreeWorkspace[];
  busy: boolean;
  error: string;
  note: string;
  onMove: (path: string) => void;
  onClose: () => void;
  onAddProject?: () => void;
}
</script>

<script setup lang="ts">
import { nextTick, onMounted, ref } from "vue";
import { t } from "../i18n";
import StudioIcon from "./StudioIcon.vue";
import { useModalFocus } from "./useModalFocus";

const props = defineProps<MoveSessionDialogProps>();

const sheet = ref<HTMLElement | null>(null);
const cancel = ref<HTMLButtonElement | null>(null);
useModalFocus(sheet);

const target = ref(props.projects[0]?.root ?? "");

onMounted(() => {
  void nextTick(() => {
    const checked = sheet.value?.querySelector<HTMLInputElement>('input[type="radio"]:checked');
    (checked ?? cancel.value)?.focus();
  });
});

function submit(): void {
  if (target.value) props.onMove(target.value);
}

function addProject(): void {
  props.onClose();
  props.onAddProject?.();
}
</script>

<template>
  <Teleport to="body">
    <div ref="sheet" class="pickveil" role="dialog" aria-modal="true" aria-labelledby="movess-t">
      <form class="pickcard movess" data-action="session.move-to" :data-target="session.path" @submit.prevent="submit">
        <h2 id="movess-t">{{ t("session.move_name_to_another_project", { name: label }) }}</h2>
        <div v-if="projects.length === 0" class="movess-empty">
          <p class="picknote">{{ t("session.there_is_no_other_project") }}</p>
          <button v-if="onAddProject" type="button" class="act" data-action="session.move-to-add" @click="addProject">
            {{ t("session.add_a_project") }}
          </button>
        </div>
        <template v-else>
          <div class="movess-label">{{ t("session.target_project") }}</div>
          <div class="movess-list" role="radiogroup" :aria-label="t('session.target_project')">
            <label v-for="p in projects" :key="p.root" class="movess-row" :data-on="p.root === target ? '' : undefined">
              <input
                type="radio"
                name="movess-target"
                :value="p.root"
                :checked="p.root === target"
                :disabled="busy"
                data-action-change="session.move-to-target"
                @change="target = p.root"
              />
              <StudioIcon name="folder" />
              <span class="movess-name"><b>{{ p.name }}</b><small>{{ p.root }}</small></span>
              <StudioIcon v-if="p.root === target" name="check" class="movess-check" />
            </label>
          </div>
          <div class="movess-cols">
            <section data-part="travels">
              <h3>{{ t("session.travels") }}</h3>
              <ul>
                <li><StudioIcon name="check" />{{ t("session.the_conversation_itself") }}</li>
                <li><StudioIcon name="check" />{{ t("session.the_title_pin_and_archive") }}</li>
                <li><StudioIcon name="check" />{{ t("session.paths_recorded_in_the_conversation") }}</li>
              </ul>
            </section>
            <section data-part="stays">
              <h3>{{ t("session.stays_behind") }}</h3>
              <ul>
                <li><StudioIcon name="close" />{{ t("session.checkpoints_rewind_snapshots") }}</li>
                <li><StudioIcon name="close" />{{ t("session.background_task_logs") }}</li>
                <li><StudioIcon name="close" />{{ t("session.queued_messages") }}</li>
              </ul>
            </section>
          </div>
          <p class="picknote">{{ t("session.what_the_old_project_allowed") }}</p>
        </template>
        <p v-if="error" class="why" role="alert">{{ error }}</p>
        <p v-if="note" class="picknote" role="status">{{ note }}</p>
        <div class="movess-act">
          <button
            ref="cancel"
            type="button"
            class="kbtn"
            data-size="md"
            data-variant="ghost"
            data-action="session.move-to-cancel"
            :disabled="busy"
            @click="onClose"
          >
            {{ note ? t("common.close") : t("common.cancel") }}
          </button>
          <button
            v-if="!note"
            type="submit"
            class="kbtn"
            data-size="md"
            data-variant="primary"
            data-action="session.move-to"
            :data-target="session.path"
            :disabled="busy || !target"
          >
            {{ t("session.move") }}
          </button>
        </div>
      </form>
    </div>
  </Teleport>
</template>
