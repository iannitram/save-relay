import { describe, expect, it } from "vitest";
import { panelPicture } from "./App.tsx";

describe("panelPicture", () => {
  it("builds the served picture URL with the token in the query", () => {
    const picture = panelPicture("T0k_en-43");

    expect(picture("steam-1", "2026-11-01", ".save-relay-picture.png")).toBe(
      "/picture/steam-1/2026-11-01/.save-relay-picture.png?token=T0k_en-43",
    );
  });

  it("percent-encodes each segment and the token", () => {
    const picture = panelPicture("a+b/c");

    expect(picture("g 1", "e/2", "f.png")).toBe("/picture/g%201/e%2F2/f.png?token=a%2Bb%2Fc");
  });

  it("has no token query without a token", () => {
    expect(panelPicture(null)("g", "e", "f.png")).toBe("/picture/g/e/f.png");
    expect(panelPicture(undefined)("g", "e", "f.png")).toBe("/picture/g/e/f.png");
  });
});
