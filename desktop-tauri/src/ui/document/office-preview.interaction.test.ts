// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/vue";
import ExcelJS from "exceljs";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import "../testkit";
import { MockPort } from "../../port/mock";
import type { AgentPort } from "../../port/port";
import { boot, t } from "../../i18n";
import { setPreviewsOffice } from "../../state/prefs";
import DocumentPreview from "./DocumentPreview.vue";
import { previewKind } from "./registry";

beforeEach(() => { localStorage.clear(); boot(); });
afterEach(() => { cleanup(); vi.restoreAllMocks(); });

async function sheetBytes(): Promise<ArrayBuffer> {
  const book = new ExcelJS.Workbook();
  book.addWorksheet("Q3").addRow(["区域", 1100]);
  return new Uint8Array((await book.xlsx.writeBuffer()) as ArrayBuffer).slice().buffer;
}

function officePort(data: ArrayBuffer) {
  const port = new MockPort() as unknown as AgentPort;
  vi.spyOn(port, "workspacePreviewInfo").mockResolvedValue({ ref: { path: "budget.xlsx" }, kind: "office", version: "v1", size: data.byteLength, limit: 32 << 20, readOnly: false });
  const content = vi.spyOn(port, "workspacePreviewContent").mockResolvedValue({ data, version: "v1" });
  return { port, content };
}

const preview = (port: AgentPort, path: string, extra: Record<string, unknown> = {}) =>
  render(DocumentPreview, { props: { port, path, shown: true, refresh: "", view: {}, ...extra } });

it("classes the formats it can read as office and leaves the rest unsupported", () => {
  expect(previewKind("报告.docx")).toBe("office");
  expect(previewKind("budget.XLSX")).toBe("office");
  for (const name of ["old.doc", "old.xls", "deck.pptx", "old.ppt"]) expect(previewKind(name)).toBe("unsupported");
});

it("renders a workbook in a sandboxed frame", async () => {
  const { port } = officePort(await sheetBytes());
  preview(port, "budget.xlsx");
  const frame = await screen.findByTitle(t("common.office_document_preview"), undefined, { timeout: 5000 });
  expect(frame.getAttribute("sandbox")).toBe("");
  expect(frame.getAttribute("srcdoc")).toContain("1100");
  expect(frame.getAttribute("srcdoc")).toContain("Content-Security-Policy");
});

it("with the setting off reads no content, and turning it on loads the preview", async () => {
  setPreviewsOffice(false);
  const { port, content } = officePort(await sheetBytes());
  preview(port, "budget.xlsx");
  await screen.findByText(t("chat.office_preview_is_off"), { exact: false });
  expect(content).not.toHaveBeenCalled();
  setPreviewsOffice(true);
  await waitFor(() => expect(content).toHaveBeenCalled());
  expect(await screen.findByTitle(t("common.office_document_preview"), undefined, { timeout: 5000 })).toBeTruthy();
});

it("says so instead of breaking when the file is not a real workbook", async () => {
  const { port } = officePort(new TextEncoder().encode("not a zip").buffer);
  preview(port, "budget.xlsx");
  expect(await screen.findByText(t("chat.this_file_could_not_be"), undefined, { timeout: 5000 })).toBeTruthy();
});

async function twoSheets(): Promise<ArrayBuffer> {
  const book = new ExcelJS.Workbook();
  book.addWorksheet("用量").addRow(["月份", 9]);
  book.addWorksheet("明细").addRow(["请求", 12480]);
  return new Uint8Array((await book.xlsx.writeBuffer()) as ArrayBuffer).slice().buffer;
}

it("keeps the read-only note at the top and switches sheets from the bottom", async () => {
  const { port } = officePort(await twoSheets());
  const { container } = preview(port, "budget.xlsx");
  const frame = await screen.findByTitle(t("common.office_document_preview"), undefined, { timeout: 5000 });
  const head = container.querySelector(".office-head")!;
  expect(within(head as HTMLElement).getByText(t("common.read_only"))).toBeTruthy();
  expect(head.textContent).toContain(t("chat.read_only_preview_macros_scripts"));
  const tabs = screen.getByRole("tablist", { name: t("chat.sheets") });
  expect(frame.compareDocumentPosition(tabs) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  await fireEvent.click(within(tabs).getByRole("tab", { name: "明细" }));
  await waitFor(() => expect(screen.getByTitle(t("common.office_document_preview")).getAttribute("srcdoc")).toContain("12480"));
});

it("with the setting off names the file and offers both ways out", async () => {
  setPreviewsOffice(false);
  const { port } = officePort(await sheetBytes());
  const onOpenExternal = vi.fn();
  const onSettings = vi.fn();
  const { container } = preview(port, "docs/report.docx", { onOpenExternal, onSettings });
  await waitFor(() => expect(container.querySelector(".office-off")).toBeTruthy());
  const card = container.querySelector(".office-off")!;
  expect(card.textContent).toContain("report.docx");
  await fireEvent.click(within(card as HTMLElement).getByRole("button", { name: t("chat.open_in_system_application") }));
  expect(onOpenExternal).toHaveBeenCalled();
  await fireEvent.click(within(card as HTMLElement).getByRole("button", { name: t("chat.turn_preview_on_in_settings") }));
  expect(onSettings).toHaveBeenCalledWith("appearance");
});