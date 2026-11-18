import { ref, type ComputedRef, type Ref } from "vue";
import type { HubPort } from "../port/hub";
import type { CloudShareOffer, PhoneAction, ShareOffer, SharePort, ShareStatus } from "../port/share";

export interface Share {
  st: Ref<ShareStatus | null>;
  ip: Ref<string>;
  offer: Ref<ShareOffer | null>;
  cloudOffer: Ref<CloudShareOffer | null>;
  busy: Ref<boolean>;
  refresh: () => Promise<ShareStatus | null>;
  newCode: () => Promise<void>;
  newCloudCode: () => Promise<void>;
  toggle: () => Promise<void>;
  pick: (ip: string) => void;
  revoke: (id: string) => Promise<void>;
  remember: Ref<boolean | null>;
  setRemember: (on: boolean) => Promise<void>;
  resolve: (id: string, action: PhoneAction, replaceId?: string) => Promise<void>;
}

// The share state a window needs: what the kernel says, plus the last code it
// minted. Ports are asked, never held: a browser tab's answer is null and the
// surfaces that draw from it stay hidden.
export function useShare(hub: HubPort): Share {
  const st = ref<ShareStatus | null>(null);
  const ip = ref("");
  const offer = ref<ShareOffer | null>(null);
  const cloudOffer = ref<CloudShareOffer | null>(null);
  const busy = ref(false);
  const remember = ref<boolean | null>(null);
  const port = hub as unknown as SharePort;

  async function refresh(): Promise<ShareStatus | null> {
    const next = await port.shareStatus();
    st.value = next;
    if (next?.open && next.origin) ip.value = next.origin.replace(/^https?:\/\//, "").replace(/:\d+$/, "");
    return next;
  }

  async function newCode(): Promise<void> {
    busy.value = true;
    try {
      offer.value = await port.offerShare();
    } finally {
      busy.value = false;
    }
  }

  async function newCloudCode(): Promise<void> {
    busy.value = true;
    try {
      cloudOffer.value = await port.offerCloudShare();
    } finally {
      busy.value = false;
    }
  }

  async function toggle(): Promise<void> {
    if (busy.value || !st.value) return;
    busy.value = true;
    try {
      st.value = st.value.open ? await port.closeShare() : await port.openShare(ip.value || "");
      if (!st.value?.open) {
        offer.value = null;
        return;
      }
      // Opening the door is asking for the first code, so one press gets a phone on.
      offer.value = await port.offerShare();
    } finally {
      busy.value = false;
    }
  }

  function pick(next: string): void {
    ip.value = next;
    if (!st.value?.open) return;
    void port.openShare(next).then((now) => { st.value = now; }).catch(() => {});
  }

  async function revoke(id: string): Promise<void> {
    try {
      st.value = await port.revokeDevice(id);
      if (st.value?.open) offer.value = await port.offerShare();
    } catch {
      await refresh().catch(() => {});
    }
  }

  async function setRemember(on: boolean): Promise<void> {
    remember.value = (await port.setShareRemember(on)).remember;
  }

  async function resolve(id: string, action: PhoneAction, replaceId?: string): Promise<void> {
    await port.resolvePhone(id, action, replaceId);
  }

  return { st, ip, offer, cloudOffer, busy, remember, refresh, newCode, newCloudCode, toggle, pick, revoke, setRemember, resolve };
}

export const shareOpen = (st: ComputedRef<ShareStatus | null>): boolean => st.value?.open ?? false;
