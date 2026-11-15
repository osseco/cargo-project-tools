import fs from "node:fs";
import { describe, expect, it } from "vitest";
import { DOC_LINK, ORDER, rewriteDocHref, slugOf, summaryOf } from "./docs";
import { slugify } from "./markdown";

const docsDir = new URL("../../../docs/", import.meta.url);
const files = fs.readdirSync(docsDir).filter((file) => file.endsWith(".md"));
const read = (file: string) => fs.readFileSync(new URL(file, docsDir), "utf8");

describe("ORDER", () => {
  it("lists every doc exactly once", () => {
    expect([...ORDER].sort()).toEqual(files.map(slugOf).sort());
  });
});

describe("docs", () => {
  it.each(files)("%s has a title and a summary", (file) => {
    const text = read(file);
    expect(text).toMatch(/^# \S/);
    expect(summaryOf(text)).not.toBe("");
  });

  it.each(files)("%s links only to docs and headings that exist", (file) => {
    for (const [, href] of read(file).matchAll(/\]\(([^)\s]+)\)/g)) {
      const link = DOC_LINK.exec(href);
      if (!link) continue;
      const [, slug, fragment] = link;
      expect(ORDER).toContain(slug);
      if (fragment) {
        const headings = [...read(`${slug}.md`).matchAll(/^#+ (.*)$/gm)].map(
          ([, text]) => slugify(text),
        );
        expect(headings).toContain(fragment.slice(1));
      }
    }
  });
});

describe("llms.txt", () => {
  it("links every doc", () => {
    const text = fs.readFileSync(
      new URL("../../../llms.txt", import.meta.url),
      "utf8",
    );
    for (const slug of ORDER) expect(text).toContain(`/docs/${slug}.md)`);
  });
});

describe("rewriteDocHref", () => {
  it("routes links between docs", () => {
    expect(rewriteDocHref("commands.md", "/base/")).toBe("/base/docs/commands");
    expect(rewriteDocHref("./configuration.md#sfw", "/")).toBe(
      "/docs/configuration#sfw",
    );
  });

  it("sends other repository paths to GitHub", () => {
    expect(rewriteDocHref("../CONTRIBUTING.md", "/")).toBe(
      "https://github.com/osseco/cargo-project-tools/blob/main/CONTRIBUTING.md",
    );
  });
});
