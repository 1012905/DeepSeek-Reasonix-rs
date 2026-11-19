// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/vue";
import "../../testkit";
import { boot, t } from "../../../i18n";
import { MockPort } from "../../../port/mock";
import type { AgentPort } from "../../../port/port";
import type { SettingsProps } from "../../SettingsProps";
import Ext from "./ext.vue";

beforeEach(() => boot());
afterEach(() => { cleanup(); vi.restoreAllMocks(); });

const props = (port: AgentPort, over: Partial<SettingsProps> = {}): SettingsProps => ({
  hub: {} as never, onError: () => {}, port, status: null, theme: "auto",
  reloadThemes: () => {}, onTheme: () => {}, contrast: "", weight: "", onWeight: () => {},
  look: {} as never, onLook: () => {}, onContrast: () => {}, onClose: () => {},
  onChanged: () => {}, ...over,
});

const draw = (port: AgentPort, over: Partial<SettingsProps> = {}) =>
  render(Ext, { props: { settingsProps: props(port, over) } });

describe("the extensions page's runtime reload", () => {
  it("offers the reload action in the footer", async () => {
    const { container } = draw(new MockPort() as unknown as AgentPort);
    const button = await waitFor(() => {
      const b = container.querySelector('[data-action="extensions.reload"]');
      expect(b).not.toBeNull();
      return b!;
    });
    expect(button.textContent).toContain(t("app.reload_runtime"));
  });

  it("reports the kernel's success and refreshes the page once it lands", async () => {
    const port = new MockPort() as unknown as AgentPort;
    const reload = vi.spyOn(port, "reloadExtensions").mockResolvedValue(undefined);
    const onChanged = vi.fn();
    const { container } = draw(port, { onChanged });
    const button = await waitFor(() => container.querySelector('[data-action="extensions.reload"]')!);
    await fireEvent.click(button);
    await waitFor(() => expect(container.querySelector('.rnote[data-s="ok"]')?.textContent).toBe(t("app.live_the_next_turn_runs")));
    expect(reload).toHaveBeenCalled();
    expect(onChanged).toHaveBeenCalled();
    expect(button.getAttribute("data-s")).toBe("ok");
  });

  it("says the kernel's own reason when the reload is refused", async () => {
    const port = new MockPort() as unknown as AgentPort;
    vi.spyOn(port, "reloadExtensions").mockRejectedValue(new Error("a turn is in flight"));
    const { container } = draw(port);
    const button = await waitFor(() => container.querySelector('[data-action="extensions.reload"]')!);
    await fireEvent.click(button);
    await waitFor(() => expect(container.querySelector('.rnote[data-s="bad"]')?.textContent).toContain("a turn is in flight"));
    expect(button.getAttribute("data-s")).toBe("bad");
  });
});