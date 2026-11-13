// #250 item 13: the release bundle must not carry the fixture bridge.
import { mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { build } from "vite";
import { expect, it } from "vitest";

it("a production build has no fixture bridge in it", async () => {
  const out = mkdtempSync(join(tmpdir(), "sr-panel-"));
  // vitest sets NODE_ENV=test, which makes vite's `import.meta.env.DEV` true; a release build is not.
  const env = process.env.NODE_ENV;
  process.env.NODE_ENV = "production";
  try {
    await build({
      root: join(__dirname, ".."),
      logLevel: "silent",
      mode: "production",
      build: { outDir: out, emptyOutDir: true, write: true },
    });
    const js = readdirSync(out)
      .filter((name) => name.endsWith(".js"))
      .map((name) => readFileSync(join(out, name), "utf8"))
      .join("\n");
    expect(js.length).toBeGreaterThan(1000);
    expect(js).not.toContain("has no fixture");
  } finally {
    process.env.NODE_ENV = env;
    rmSync(out, { recursive: true, force: true });
  }
}, 120_000);
