import { reason } from "../../i18n/kernel";
import { HttpError } from "../../port/port";

// What the first screen offers before anyone has typed anything.
//
// A shortcut fills the address and nothing more: a service missing from this
// row is not unsupported — what decides that is what the endpoint answers. 自建
// is a first-class entry rather than a fallback, because a relay is what many
// people actually have. `console` is where a person goes to get the key this
// screen is asking for, the step nothing in the app can do for them; a vendor
// whose key page we cannot name links to its platform root instead, because
// sending someone to a 404 is worse than sending them to a front door.
//
// `mark` is the vendor slug vendormarks.vue knows; `label` is an i18n key for
// every entry except the two product names that are not translated.
export interface Shortcut {
  label: string;
  hint: string;
  url: string;
  mark?: string;
  console?: string;
  // The address this service listens on per protocol, for the services that
  // answer on more than one. Switching the protocol moves the address with it.
  protocolUrls?: Record<string, string>;
}

export const SHORTCUTS: Shortcut[] = [
  {
    label: "DeepSeek",
    mark: "deepseek",
    hint: "onboarding.deepseek_hint",
    url: "https://api.deepseek.com",
    console: "https://platform.deepseek.com/api_keys",
    protocolUrls: {
      openai: "https://api.deepseek.com",
      responses: "https://api.deepseek.com",
      anthropic: "https://api.deepseek.com/anthropic",
    },
  },
  { label: "onboarding.siliconflow", hint: "onboarding.siliconflow_hint", url: "https://api.siliconflow.cn/v1", console: "https://cloud.siliconflow.cn" },
  { mark: "kimi", label: "onboarding.moonshot", hint: "onboarding.moonshot_hint", url: "https://api.moonshot.cn/v1", console: "https://platform.moonshot.cn" },
  { mark: "zhipu", label: "onboarding.zhipu", hint: "onboarding.zhipu_hint", url: "https://open.bigmodel.cn/api/paas/v4", console: "https://bigmodel.cn/usercenter/proj-mgmt/apikeys" },
  { mark: "mimo", label: "MiMo", hint: "onboarding.mimo_hint", url: "https://api.xiaomimimo.com/v1", console: "https://platform.xiaomimimo.com" },
  { mark: "longcat", label: "LongCat", hint: "onboarding.longcat_hint", url: "https://api.longcat.chat/openai", console: "https://longcat.chat/platform" },
  { label: "onboarding.relay_self_hosted", hint: "onboarding.relay_hint", url: "" },
];

// How each wire is described under its own name. The kernel names the kinds;
// these are the three whose wire the person may have heard of by name.
export const KIND_DETAIL: Record<string, string> = {
  openai: "Chat Completions",
  responses: "Responses API",
  anthropic: "Messages API",
};

// A listed service at its listed address answers in a known way, so only a
// model list longer than this is put behind a picker rather than laid out.
export const MODEL_LIST_MAX = 8;

// The two failures nobody fixes in this window: the key is refused, or the
// account behind it is out of credit. Both are settled on the vendor's site, so
// the refusal carries a way to get there instead of only a sentence.
export const FIX_AT_VENDOR = new Set(["provider.probe.unauthorized", "provider.probe.payment_required"]);

// Failed is a refusal kept whole: the sentence to read, and the code that says
// whether reading it is enough.
export interface Failed {
  text: string;
  code?: string;
}

export function failed(e: unknown): Failed {
  return { text: reason(e), code: e instanceof HttpError ? e.reason?.code : undefined };
}
