import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { parse, compileScript, compileTemplate } from "vue/compiler-sfc";
import ts from "typescript";

const root = fileURLToPath(new URL("../../../", import.meta.url));
const names = ["App", "Pane", "Transcript", "Composer"] as const;
const source = (name: string) => readFileSync(`${root}desktop/frontend-next/src/ui/${name}.tsx`, "utf8");
const target = (name: string) => readFileSync(`${root}desktop-tauri/src/ui/${name}.vue`, "utf8");

function tree(text: string) {
  return ts.createSourceFile("component.tsx", text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
}
function visit(node: ts.Node, fn: (node: ts.Node) => void) {
  fn(node);
  node.forEachChild((child) => visit(child, fn));
}
function interfaceProps(text: string, name: string) {
  const props: string[] = [];
  visit(tree(text), (node) => {
    if (ts.isInterfaceDeclaration(node) && node.name.text === name) {
      for (const member of node.members) if (member.name && ts.isIdentifier(member.name)) props.push(member.name.text);
    }
  });
  return props.sort();
}
function script(name: string) {
  const filename = `${name}.vue`;
  const { descriptor, errors } = parse(target(name), { filename });
  expect(errors).toEqual([]);
  const compiled = compileScript(descriptor, { id: name });
  if (descriptor.template) {
    const template = compileTemplate({ id: name, filename, source: descriptor.template.content, compilerOptions: { bindingMetadata: compiled.bindings } });
    expect(template.errors).toEqual([]);
  }
  const checked = ts.transpileModule(compiled.content, { reportDiagnostics: true, compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } });
  expect(checked.diagnostics?.filter((d) => d.category === ts.DiagnosticCategory.Error)).toEqual([]);
  return compiled.content;
}

describe("window and conversation Vue port contracts", () => {
  for (const name of names) {
    it(`${name} passes compiler-sfc and TypeScript syntax validation`, () => { script(name); });
    it(`${name} keeps every original data/ARIA attribute and action identity`, () => {
      const attributes = new Set<string>();
      const actions = new Set<string>();
      visit(tree(source(name)), (node) => {
        if (!ts.isJsxAttribute(node)) return;
        const key = node.name.getText();
        if (/^(data-|aria-)/.test(key)) attributes.add(key);
        if (key.startsWith("data-action") && node.initializer && ts.isStringLiteral(node.initializer)) actions.add(node.initializer.text);
      });
      const strings = new Set<string>();
      visit(tree(script(name)), (node) => { if (ts.isStringLiteral(node)) strings.add(node.text); });
      for (const key of [...attributes, ...actions]) expect(strings, key).toContain(key);
    });
    it(`${name} has no React runtime or ignored watcher cleanup`, () => {
      visit(tree(script(name)), (node) => {
        if (ts.isImportDeclaration(node) && ts.isStringLiteral(node.moduleSpecifier)) expect(node.moduleSpecifier.text).not.toMatch(/^(react|react-dom|lucide-react)(\/|$)/);
        if (!ts.isCallExpression(node) || !ts.isIdentifier(node.expression) || node.expression.text !== "watch") return;
        const callback = node.arguments[1];
        if (!callback || !(ts.isArrowFunction(callback) || ts.isFunctionExpression(callback))) return;
        if (ts.isBlock(callback.body)) for (const statement of callback.body.statements) {
          if (ts.isReturnStatement(statement) && statement.expression) expect(ts.isArrowFunction(statement.expression) || ts.isFunctionExpression(statement.expression)).toBe(false);
        }
      });
    });
  }
  for (const name of ["Composer", "Transcript", "Pane"] as const) {
    it(`${name} retains its original prop names`, () => {
      const original = name === "Pane" ? readFileSync(`${root}desktop/frontend-next/src/ui/panetypes.ts`, "utf8") : source(name);
      const contract = name === "Pane" ? "PaneProps" : "Props";
      expect(interfaceProps(script(name), contract)).toEqual(interfaceProps(original, contract));
    });
  }
  it("Pane keeps the reducer, history, submit, draft and decision owners", () => {
    const pane = script("Pane");
    for (const path of ["../state/session", "./useHistoryPages", "./usesubmit", "./gates", "./queueactions", "./surface-replay", "../port/streambudget", "./useConversationFiles"]) expect(pane).toContain(`"${path}"`);
    expect(pane).toContain("reply: fileReply.value");
    expect(pane).toContain("fileRequest: files.request.value");
    expect(script("Composer")).toContain('"./useDraft"');
  });
  it("App keeps the rail's feedback unread honest from the local machine", () => {
    const app = script("App");
    for (const owner of ["./useFeedbackUnread", "./feedbackentry.vue"]) expect(app).toContain(`"${owner}"`);
    expect(app).toContain("useFeedbackUnread(");
    expect(app).toContain("feedbackEntryTab(feedbackUnread.value)");
    expect(app).toContain("runtimes.value.find((rt) => !rt.host)");
    expect(app).toContain("feedbackUnread: feedbackUnread.value");
    expect(app).toContain("unread: feedbackUnread.value");
  });
  it("App orders concurrent session opens and lets a focused pane cancel one", () => {
    const app = script("App");
    expect(app).toContain('"./sessionswitch"');
    for (const step of ["sw.opening(wanted)", "sw.begin(wanted)", "sw.current(ticket)", "sw.landed(ticket)", "sw.end(ticket)"]) expect(app).toContain(step);
    expect(app).toContain("sw.cancel()");
    expect(app).toContain("startConversation(root)");
  });
});
