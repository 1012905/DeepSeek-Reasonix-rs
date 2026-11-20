// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/vue";
import "../testkit";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { boot, t } from "../../i18n";
import type { DocumentViewState } from "./viewState";

const pdf = vi.hoisted(() => ({ load: vi.fn(), destroy: vi.fn() }));
vi.mock("pdfjs-dist", () => ({
  GlobalWorkerOptions: {}, TextLayer: class { render() { return Promise.resolve(); } cancel() {} }, getDocument: pdf.load,
}));
import PdfPreview from "./PdfPreview.vue";

beforeEach(() => boot());
afterEach(() => { cleanup(); vi.clearAllMocks(); });

it("keeps XFA off and releases the loading task when the file changes or closes", async () => {
  pdf.load.mockImplementation(() => ({ promise: new Promise(() => {}), destroy: pdf.destroy }));
  const data = new ArrayBuffer(16);
  const { rerender, unmount } = render(PdfPreview, { props: { data, view: {} } });
  expect(pdf.load).toHaveBeenCalledWith(expect.objectContaining({ enableXfa: false }));
  expect(pdf.load.mock.calls[0][0].data).not.toBe(data);
  await rerender({ data: new ArrayBuffer(32), view: {} });
  expect(pdf.destroy).toHaveBeenCalledTimes(1);
  expect(pdf.load).toHaveBeenLastCalledWith(expect.objectContaining({ enableXfa: false }));
  unmount();
  expect(pdf.destroy).toHaveBeenCalledTimes(2);
});

describe("the PDF toolbar", () => {
  const page = {
    getViewport: ({ scale }: { scale: number }) => ({ width: 600 * scale, height: 800 * scale }),
    render: () => ({ promise: Promise.resolve(), cancel() {} }),
    streamTextContent: () => ({}),
  };
  function open(numPages = 12, onOpenExternal = vi.fn()) {
    pdf.load.mockImplementation(() => ({ promise: Promise.resolve({ numPages, getPage: async () => page }), destroy: pdf.destroy }));
    const view: DocumentViewState = {};
    render(PdfPreview, { props: { data: new ArrayBuffer(8), view, onOpenExternal } });
    return { view, onOpenExternal };
  }

  it("moves page by page and takes a typed page number, held to the document", async () => {
    const { view } = open();
    const box = await screen.findByRole("textbox", { name: t("chat.page_number") }) as HTMLInputElement;
    await waitFor(() => expect(screen.getByText("/ 12")).toBeTruthy());
    await fireEvent.click(screen.getByRole("button", { name: t("common.next_page") }));
    expect(box.value).toBe("2");
    await fireEvent.update(box, "40");
    await fireEvent.keyDown(box, { key: "Enter" });
    expect(box.value).toBe("12");
    expect(screen.getByRole("button", { name: t("common.next_page") }).hasAttribute("disabled")).toBe(true);
    await fireEvent.update(box, "abc");
    await fireEvent.blur(box);
    expect(box.value).toBe("12");
    await waitFor(() => expect(view.page).toBe(12));
  });

  it("zooms in steps from wherever fit-width left it, and goes back to fitting the width", async () => {
    const { view } = open();
    const fit = await screen.findByRole("button", { name: t("chat.fit_width") });
    expect(fit.getAttribute("aria-pressed")).toBe("true");
    await fireEvent.click(screen.getByRole("button", { name: t("chat.zoom_in") }));
    await waitFor(() => expect(view.zoom).toBeGreaterThan(0));
    expect(fit.getAttribute("aria-pressed")).toBe("false");
    await fireEvent.click(fit);
    await waitFor(() => expect(view.zoom).toBe(0));
  });

  it("offers the system application when the file cannot be shown", async () => {
    pdf.load.mockImplementation(() => ({ promise: Promise.reject(new Error("bad")), destroy: pdf.destroy }));
    const onOpenExternal = vi.fn();
    render(PdfPreview, { props: { data: new ArrayBuffer(8), view: {}, onOpenExternal } });
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain(t("chat.cannot_display_this_pdf_it"));
    await fireEvent.click(screen.getByRole("button", { name: t("chat.open_in_system_application") }));
    expect(onOpenExternal).toHaveBeenCalled();
  });
});