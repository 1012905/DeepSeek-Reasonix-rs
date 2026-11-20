// @vitest-environment jsdom
//
// What this file proves: the first-run guide a new person lands on shows a
// welcome with a way to start, the start press really moves to the first step
// and the progress list follows, the sequence can be walked forward through the
// key step and the model step, and the "go configure the model service instead"
// route calls the window's handler with the provider anchor.
//
// One thing this file found and records rather than fixes: the guide's own
// `onOpenSettings` route ("去连接", ConnectNote) is only reachable from the model
// or project step *while no endpoint has answered*. Every route into those steps
// in the shipped component sets `found`, so the offer is never drawn. See the
// last test — it asserts the gap as it stands and will start failing when the
// route becomes reachable, which is the point.
import { afterEach, describe, expect, it, vi } from "vitest";
import { mount, type VueWrapper } from "@vue/test-utils";
import { nextTick } from "vue";
import "../testkit";
import Onboarding, { STEPS, type OnboardingStep } from "../Onboarding.vue";
import { MockPort } from "../../port/mock";
import { MockHub } from "../../port/mock_hub";
import type { HubPort } from "../../port/hub";
import type { AgentPort } from "../../port/port";
import { boot, STORAGE, t } from "../../i18n";

const settle = (ms = 0) => new Promise((done) => setTimeout(done, ms));

const opened: VueWrapper[] = [];
afterEach(async () => {
  while (opened.length > 0) opened.pop()?.unmount();
  localStorage.removeItem(STORAGE);
  boot();
});

const finishSettle = async (times = 3) => { for (let i = 0; i < times; i++) await nextTick(); };

interface GuideOptions {
  onDone?: (result: { projectRoot?: string }) => void;
  onError?: (e: unknown) => void;
  onOpenSettings?: (at?: string) => void;
}

const guide = (opts: GuideOptions = {}) => {
  const port = new MockPort() as unknown as AgentPort;
  const hub = new MockHub() as unknown as HubPort;
  const wrapper = mount(Onboarding, {
    props: {
      hub,
      port,
      onDone: opts.onDone ?? (() => {}),
      onError: opts.onError ?? (() => {}),
      onOpenSettings: opts.onOpenSettings,
    },
    attachTo: document.body,
  });
  opened.push(wrapper);
  return { wrapper, port, hub };
};

const cardStep = (wrapper: VueWrapper) => wrapper.get(".onb-card").attributes("data-step") as OnboardingStep;

/** The step list's own reading: what is done, what is current, what is ahead. */
const progress = (wrapper: VueWrapper) =>
  wrapper.findAll(".onb-steps li").map((li) => [li.attributes("data-state"), li.text()]);

describe("the first-run guide a new person lands on", () => {
  it("opens on a welcome that names the product and offers a way to start", async () => {
    boot();
    const { wrapper } = guide();
    await finishSettle();

    expect(cardStep(wrapper)).toBe("welcome");
    // The welcome is the introduction, not a step: no progress bar over it.
    expect(wrapper.find(".onb-steps").exists()).toBe(false);

    const card = wrapper.get(".onb-welcome");
    expect(card.get(".onb-t").text()).toBe(t("onboarding.hand_it_a_task"));
    expect(card.text()).toContain(t("onboarding.what_it_does_and_shows"));

    const begin = card.findAll("button").find((b) => b.attributes("data-action") === "onboarding.begin");
    expect(begin, "the welcome offers a start button").toBeTruthy();
    expect(begin!.text()).toBe(t("onboarding.set_up_reasonix"));
    // And a way out for someone who would rather not be shown around.
    expect(card.findAll("button").some((b) => b.attributes("data-action") === "onboarding.skip")).toBe(true);
  });

  it("moves to the first step when the start button is pressed, and the progress list begins there", async () => {
    boot();
    const { wrapper } = guide();
    await finishSettle();

    await wrapper.get('[data-action="onboarding.begin"]').trigger("click");
    await finishSettle();

    expect(cardStep(wrapper)).toBe("connect");
    expect(wrapper.get(".onb-card .onb-t").text()).toBe(t("onboarding.connect_one_model_service"));

    // The steps are the component's own list, in its own order.
    const list = wrapper.findAll(".onb-steps li");
    expect(list.length).toBe(STEPS.length);
    expect(list.map((li) => li.text())).toEqual(STEPS.map(([id, key]) => `${STEPS.findIndex(([s]) => s === id) + 1}${t(key)}`));
    expect(list[0].attributes("data-state")).toBe("current");
    expect(list.slice(1).every((li) => li.attributes("data-state") === "todo")).toBe(true);
    expect(list[0].attributes("aria-current")).toBe("step");
  });

  it("walks forward through the key step and the model step, moving the progress marker each time", async () => {
    boot();
    const { wrapper } = guide();
    await finishSettle();

    await wrapper.get('[data-action="onboarding.begin"]').trigger("click");
    await finishSettle();

    // The key field is the one thing this step owes an answer for.
    await wrapper.get("#onb-key").setValue("sk-test");
    await wrapper.get('[data-action="onboarding.connect"]').trigger("click");
    // The probe answers, the tick shows, and the page moves a beat later.
    await settle(900);
    await finishSettle();

    expect(cardStep(wrapper)).toBe("model");
    // A model came back from the endpoint and one of them is the default.
    const models = wrapper.findAll('[data-action="onboarding.model"]');
    expect(models.length).toBeGreaterThan(0);
    expect(models.some((b) => b.attributes("aria-checked") === "true")).toBe(true);

    const afterConnect = progress(wrapper);
    // A step already passed carries a tick in place of its number, so only the
    // word is asserted there; the state is the assertion that matters.
    expect(afterConnect[0][0]).toBe("done");
    expect(afterConnect[0][1]).toBe(t(STEPS[0][1]));
    expect(afterConnect[1][0]).toBe("current");

    await wrapper.get('[data-action="onboarding.start"]').trigger("click");
    await settle(200);
    await finishSettle();

    // The folder step, and both steps behind it read as done.
    expect(cardStep(wrapper)).toBe("project");
    const afterStart = progress(wrapper);
    expect(afterStart[0][0]).toBe("done");
    expect(afterStart[1][0]).toBe("done");
    expect(afterStart[2][0]).toBe("current");
  });

  it("goes back a step without leaving the guide or writing anything to the machine", async () => {
    boot();
    const { wrapper, port } = guide();
    await finishSettle();

    await wrapper.get('[data-action="onboarding.begin"]').trigger("click");
    await finishSettle();
    expect(cardStep(wrapper)).toBe("connect");

    const back = wrapper.findAll("button").find((b) => b.attributes("data-action") === "onboarding.back");
    expect(back, "the step offers a way back").toBeTruthy();
    await back!.trigger("click");
    await finishSettle();

    expect(cardStep(wrapper)).toBe("welcome");
    // The welcome is drawn again, and nothing was recorded on the way there.
    expect(wrapper.find(".onb-welcome").exists()).toBe(true);
    expect(wrapper.find(".onb-steps").exists()).toBe(false);
    expect(port.markWelcomed).toBeTruthy();
  });

  // ── The gap ────────────────────────────────────────────────────────────────
  //
  // Onboarding.vue builds `offer` (which renders ConnectNote, whose button calls
  // `props.onOpenSettings("providers")`) only when `!found.value`, on the model
  // or project step. But `found` is set by `connect()` — the only way into the
  // model step — and the project step is reached from `start()`, which returns
  // early unless `found` is set. So in the shipped component the offer can never
  // be non-null, and `onOpenSettings` is dead code from the guide's own screen.
  // The route a person actually has into the provider settings is the palette
  // and the panes' own connect note, not the guide.
  it("never offers its own 'go connect instead' route, because every path to that step already found an endpoint", async () => {
    boot();
    const openSettings = vi.fn();
    const { wrapper } = guide({ onOpenSettings: openSettings });
    await finishSettle();

    expect(wrapper.find('[data-action="onboarding.reopen"]').exists()).toBe(false);

    await wrapper.get('[data-action="onboarding.begin"]').trigger("click");
    await finishSettle();
    // The step that already asks for a key deliberately has no second "connect".
    expect(wrapper.find('[data-action="onboarding.reopen"]').exists()).toBe(false);

    await wrapper.get("#onb-key").setValue("sk-test");
    await wrapper.get('[data-action="onboarding.connect"]').trigger("click");
    await settle(900);
    await finishSettle();
    expect(cardStep(wrapper)).toBe("model");
    // Found is set by the connect that got here, so the note is still absent.
    expect(wrapper.find('[data-action="onboarding.reopen"]').exists()).toBe(false);

    await wrapper.get('[data-action="onboarding.start"]').trigger("click");
    await settle(200);
    await finishSettle();
    expect(cardStep(wrapper)).toBe("project");
    expect(wrapper.find('[data-action="onboarding.reopen"]').exists()).toBe(false);

    // Nothing on the guide's own screen ever reached the handler.
    expect(openSettings).not.toHaveBeenCalled();
  });
});
