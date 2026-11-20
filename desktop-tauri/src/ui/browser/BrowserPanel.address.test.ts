import { describe, expect, it } from "vitest";
import { hostOf, normalise, START } from "./BrowserPanel.vue";

// What someone types is not an address until this says it is: everything below
// is a shape that reached a real window and had to be read one way or another.
describe("reading what was typed into the address bar", () => {
  it("keeps an http or https address as it stands", () => {
    expect(normalise("https://example.com/a?b=1")).toBe("https://example.com/a?b=1");
    expect(normalise("http://example.com")).toBe("http://example.com/");
  });

  it("takes a bare host as https and a bare intranet name as http", () => {
    expect(normalise("example.com")).toBe("https://example.com/");
    expect(normalise("wiki")).toBe("http://wiki/");
    expect(normalise("localhost:5173")).toBe("http://localhost:5173/");
    expect(normalise("10.1.2.3:8080/x")).toBe("http://10.1.2.3:8080/x");
  });

  it("refuses a path on disk instead of reading its drive letter as a host", () => {
    expect(normalise("C:\\Users\\me\\page.html")).toBeNull();
    expect(normalise("/home/me/page.html")).toBeNull();
    expect(normalise("\\\\server\\share\\page.html")).toBeNull();
  });

  it("refuses a scheme no web page can be", () => {
    expect(normalise("file:///etc/passwd")).toBeNull();
    expect(normalise("javascript:alert(1)")).toBeNull();
    expect(normalise("ftp://example.com")).toBeNull();
  });

  it("answers the panel's own start page for an empty line", () => {
    expect(normalise("")).toBe(START);
    expect(normalise("   ")).toBe(START);
    expect(normalise("reasonix://start")).toBe(START);
  });

  it("names a page by its host, and the start page by nothing", () => {
    expect(hostOf("https://wiki.example.com/Path")).toBe("wiki.example.com");
    expect(hostOf(START)).toBe("");
    expect(hostOf("")).toBe("");
    expect(hostOf("not an address")).toBe("");
  });
});
