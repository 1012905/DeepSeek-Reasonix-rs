<script lang="ts">
import type { RemoteHost, RemoteProbe } from "../../port/remote";

// One machine in the book, as a row. Everything it can do is a callback: the
// book belongs to the panel, so a row that saved or dropped an entry would be a
// second writer of the same list.
export interface RemoteHostsProps {
  host: RemoteHost;
  /** Every folder this machine is worked in, default first. Folded by the panel
   *  from the two stored fields, so one definition answers for both screens. */
  projects: string[];
  /** Where it sits in the book and how long the book is: the two facts the move
   *  entries are decided from. */
  position: number;
  total: number;
  /** What it answered when it was asked, if it has been. */
  probe?: RemoteProbe;
  probing?: boolean;
  /** A write of this row is in flight; every control on it is off meanwhile. */
  busy?: boolean;
  /** Newest activity this window knows on that machine, Unix ms. Absent until
   *  the caller has read the far book, which is the only place a time lives. */
  lastUsed?: number;
  /** Kept out of the rail. A display choice: the row is still here. */
  hidden?: boolean;
  onProbe: () => void;
  onOpen: () => void;
  onEdit: () => void;
  onRemove: () => void;
  /** Kept in the book but not dialed. */
  onToggleOff: () => void;
  onHide: () => void;
  /** Absent for a direction with no neighbour, rather than a disabled button. */
  onMove?: (direction: -1 | 1) => void;
}
</script>

<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import { t } from "../../i18n";
import { say } from "../../i18n/kernel";
import { REMOTE_STEP_LABEL } from "../../port/remote";
import { statusKey } from "./hostbook";
import StudioIcon from "../StudioIcon.vue";

const props = defineProps<RemoteHostsProps>();

// The menu and the removal question. Both belong to the row they are about: a
// removal confirmed one row up would have to carry which row it was.
const menu = ref(false);
const more = ref<HTMLSpanElement | null>(null);
const confirm = ref(false);
const removeButton = ref<HTMLButtonElement | null>(null);

const working = computed(() => props.host.status === "connecting" || props.host.status === "reconnecting");
const clock = (at: number): string => new Date(at).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });

/** The sentence under the row: a bootstrap step while a first connect walks
 *  them, a reason while the link is degraded or gone, and nothing when there is
 *  nothing to add. */
const detail = computed(() => {
  const host = props.host;
  if (working.value) return host.step ? t(REMOTE_STEP_LABEL[host.step] ?? host.step) : "";
  if (host.status === "reconnecting") {
    return host.attempt ? t("remote.dropped_reconnecting_attempt_n", { n: host.attempt }) : t("remote.dropped_reconnecting");
  }
  if (host.status === "degraded") return host.error || t("remote.connected_but_a_forward_did");
  if (host.status === "stopped") return host.error || t("remote.disconnected");
  return "";
});

const closed = computed(() => (props.probe?.routes ?? []).filter((route) => !route.ok));

watch(confirm, async (shown) => {
  if (!shown) return;
  await nextTick();
  removeButton.value?.focus();
});

/** The menu's own life: a press outside it or Escape closes it. Registered only
 *  while it is open, so a closed row costs nothing. */
watch(menu, (open, _was, onCleanup) => {
  if (!open) return;
  const away = (event: MouseEvent) => {
    if (!more.value?.contains(event.target as Node)) menu.value = false;
  };
  const esc = (event: KeyboardEvent) => {
    if (event.key !== "Escape") return;
    event.stopPropagation();
    menu.value = false;
  };
  addEventListener("mousedown", away);
  addEventListener("keydown", esc, true);
  onCleanup(() => {
    removeEventListener("mousedown", away);
    removeEventListener("keydown", esc, true);
  });
}, { flush: "post" });

function pick(run: () => void): void {
  menu.value = false;
  run();
}
</script>

<template>
  <div class="rmtrow" :data-state="host.status" :data-off="host.disabled ? '' : undefined" :data-busy="busy ? '' : undefined">
    <span class="rmticon" aria-hidden="true"><StudioIcon name="server" /></span>

    <div class="rmtmain">
      <div class="hd">
        <b>{{ host.name }}</b>
        <span class="tg" dir="ltr">{{ host.target }}</span>
        <span class="st" :data-state="host.disabled ? 'off' : host.status">{{ t(statusKey(host)) }}</span>
      </div>
      <div class="sub">
        <span v-if="lastUsed">{{ t("remote.last_used_at", { at: clock(lastUsed) }) }}</span>
        <span v-else-if="host.workspace" dir="ltr">{{ host.workspace }}</span>
        <span v-else class="dim">{{ t("remote.no_default_workspace") }}</span>
        <span v-if="projects.length > 1" class="tag">{{ t("remote.n_more_projects", { n: projects.length - 1 }) }}</span>
        <span v-if="host.kind === 'wsl'" class="tag">{{ t("remote.wsl_distribution") }}</span>
        <span v-if="host.useSSHConfig" class="tag">{{ t("remote.follows_ssh_config") }}</span>
        <span v-if="host.forwards" class="tag">{{ t("remote.n_forwards", { n: host.forwards }) }}</span>
        <span v-if="host.panes" class="tag">{{ t("remote.n_panes", { n: host.panes }) }}</span>
      </div>
    </div>

    <div class="rmtside">
      <!-- An idle machine is where connecting starts, so the button that asks it
           is the row's own; a connected one tests from the menu. -->
      <button
        v-if="host.status !== 'connected' && !host.disabled"
        type="button"
        class="rmtnext"
        data-action="remote.probe"
        :data-target="host.name"
        :disabled="probing || busy"
        @click="onProbe"
      >
        {{ t(probing ? "remote.asking" : "common.test_it") }}
      </button>

      <span ref="more" class="rmthost-more">
        <button
          type="button"
          class="rmthost-btn"
          data-action="remote.more"
          :data-target="host.name"
          :aria-label="t('remote.more_for_name', { name: host.name })"
          :title="t('common.more')"
          aria-haspopup="menu"
          :aria-expanded="menu"
          @click="menu = !menu"
        >
          <StudioIcon name="more" />
        </button>
        <span v-if="menu" class="rmthost-menu" role="menu" :aria-label="host.name">
          <button role="menuitem" data-action="remotes.open" :data-target="host.name" :disabled="host.disabled || !!busy" @click="pick(onOpen)">
            {{ t("common.open") }}
          </button>
          <button role="menuitem" data-action="remote.enable" :data-target="host.name" :disabled="!!busy" @click="pick(onToggleOff)">
            {{ t(host.disabled ? "remote.turn_this_machine_on" : "remote.turn_this_machine_off") }}
          </button>
          <button role="menuitem" data-action="remote.edit" :data-target="host.name" :disabled="!!busy" @click="pick(onEdit)">
            {{ t("common.edit") }}
          </button>
          <button
            v-if="host.status === 'connected'"
            role="menuitem"
            data-action="remote.probe"
            :data-target="host.name"
            :disabled="probing || !!busy"
            @click="pick(onProbe)"
          >
            {{ t(probing ? "remote.asking" : "common.test_it") }}
          </button>
          <button role="menuitem" data-action="remote.hide" :data-target="host.name" @click="pick(onHide)">
            {{ t(hidden ? "remote.show_in_the_rail" : "remote.hide_from_the_rail") }}
          </button>
          <button
            v-if="position > 0"
            role="menuitem"
            data-action="remote-host.move"
            :data-target="host.name"
            data-value="-1"
            :disabled="!!busy"
            @click="pick(() => onMove?.(-1))"
          >
            {{ t("common.move_name_up", { name: host.name }) }}
          </button>
          <button
            v-if="position < total - 1"
            role="menuitem"
            data-action="remote-host.move"
            :data-target="host.name"
            data-value="1"
            :disabled="!!busy"
            @click="pick(() => onMove?.(1))"
          >
            {{ t("common.move_name_down", { name: host.name }) }}
          </button>
          <button role="menuitem" data-danger data-action="remote.remove-ask" :data-target="host.name" :disabled="!!busy" @click="pick(() => { confirm = true; })">
            {{ t("common.remove") }}
          </button>
        </span>
      </span>
    </div>

    <!-- What one machine answered. A connect stops at the first missing piece,
         so the value here is seeing all of them together, and each closed route
         is worded by the code a real failure would have carried. -->
    <div v-if="probe" class="rmtprobe" :data-ok="probe.ready ? '' : undefined">
      <div class="rmtprobe-r">
        <span class="k">{{ t("remote.machine") }}</span>
        <span class="v">{{ probe.os }}/{{ probe.arch }} · {{ probe.home }}</span>
      </div>
      <div class="rmtprobe-r">
        <span class="k">reasonix</span>
        <span class="v">{{ probe.kernel ? `${probe.kernel} ${probe.version ?? ""}`.trim() : probe.outdated ? t("remote.it_has_v_there_too", { v: probe.outdated }) : t("remote.none_yet") }}</span>
      </div>
      <div class="rmtprobe-r">
        <span class="k">npm</span>
        <span class="v">{{ probe.npm || t("remote.will_not_run") }}</span>
      </div>
      <p v-for="route in closed" :key="route.name" class="rmtprobe-why">
        {{ say({ code: route.code, params: { host: host.name } }, t("remote.the_name_route_is_closed", { name: route.name })) }}
      </p>
      <div class="rmtprobe-end">
        {{ probe.ready ? t("remote.ready_there_is_a_kernel") : t("remote.not_ready_clearing_any_one") }}
      </div>
    </div>

    <div v-if="detail" class="project-connection" role="status">
      <StudioIcon :name="working ? 'refresh' : 'warning'" />
      <span>{{ detail }}</span>
    </div>

    <div v-if="confirm" class="rmtconfirm" role="alertdialog">
      <span>{{ t("remote.remove_name_from_the_list", { name: host.name }) }}</span>
      <button type="button" data-action="remote.remove-cancel" @click="confirm = false">{{ t("common.cancel") }}</button>
      <button
        ref="removeButton"
        type="button"
        data-danger
        data-action="remotes.remove"
        :data-target="host.name"
        @click="confirm = false; onRemove()"
      >
        {{ t("common.remove") }}
      </button>
    </div>
  </div>
</template>
