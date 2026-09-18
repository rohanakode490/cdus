import { describe, it, expect } from "bun:test";
import {
  ICONS,
  getIcon,
  renderIcons,
  type IconName,
  SVG_CLIPBOARD,
  SVG_FOLDER,
  SVG_SHIELD,
  SVG_SEARCH,
  SVG_LIGHTBULB,
  SVG_WARNING,
  SVG_GLOBE,
  SVG_IMAGE,
  SVG_DOCUMENT,
  SVG_DEVICE,
} from "../../src/icons";

describe("Frontend SVG Icons Library", () => {
  const expectedIconNames: IconName[] = [
    "clipboard",
    "folder",
    "shield",
    "search",
    "lightbulb",
    "warning",
    "globe",
    "image",
    "document",
    "device",
  ];

  it("exports all expected icon names in the ICONS registry", () => {
    for (const name of expectedIconNames) {
      expect(ICONS[name]).toBeDefined();
      expect(typeof ICONS[name]).toBe("string");
      expect(ICONS[name].length).toBeGreaterThan(0);
    }
  });

  it("ensures all SVG icon strings are valid vector icons", () => {
    for (const name of expectedIconNames) {
      const svg = ICONS[name];
      expect(svg.startsWith("<svg")).toBe(true);
      expect(svg.endsWith("</svg>")).toBe(true);
      expect(svg).toContain('viewBox="0 0 24 24"');
      expect(svg).toContain('fill="none"');
      expect(svg).toContain('stroke="currentColor"');
      expect(svg).toContain('aria-hidden="true"');
    }
  });

  it("ensures no emojis exist in any exported icon strings", () => {
    const emojiRegex = /[\u{10000}-\u{10ffff}\u{2600}-\u{27bf}\u{2300}-\u{23ff}\u{2b50}]/u;
    for (const name of expectedIconNames) {
      const svg = ICONS[name];
      expect(emojiRegex.test(svg)).toBe(false);
    }
  });

  it("verifies individual exported icon constants match registry", () => {
    expect(ICONS.clipboard).toBe(SVG_CLIPBOARD);
    expect(ICONS.folder).toBe(SVG_FOLDER);
    expect(ICONS.shield).toBe(SVG_SHIELD);
    expect(ICONS.search).toBe(SVG_SEARCH);
    expect(ICONS.lightbulb).toBe(SVG_LIGHTBULB);
    expect(ICONS.warning).toBe(SVG_WARNING);
    expect(ICONS.globe).toBe(SVG_GLOBE);
    expect(ICONS.image).toBe(SVG_IMAGE);
    expect(ICONS.document).toBe(SVG_DOCUMENT);
    expect(ICONS.device).toBe(SVG_DEVICE);
  });

  describe("getIcon() options and customization", () => {
    it("returns default icon markup when called without options", () => {
      const clipboardSvg = getIcon("clipboard");
      expect(clipboardSvg).toBe(SVG_CLIPBOARD);
    });

    it("customizes width and height when size option is provided", () => {
      const customSvg = getIcon("search", { size: 32 });
      expect(customSvg).toContain('width="32"');
      expect(customSvg).toContain('height="32"');
      expect(customSvg).not.toContain('width="16"');
      expect(customSvg).not.toContain('height="16"');
    });

    it("injects custom class names when className option is provided", () => {
      const customSvg = getIcon("warning", { className: "custom-alert-icon active" });
      expect(customSvg).toContain('<svg class="custom-alert-icon active" ');
    });

    it("customizes strokeWidth when specified", () => {
      const customSvg = getIcon("shield", { strokeWidth: 3 });
      expect(customSvg).toContain('stroke-width="3"');
    });

    it("returns an empty string when an invalid icon name is requested", () => {
      // @ts-expect-error testing invalid runtime argument
      const result = getIcon("non-existent-icon");
      expect(result).toBe("");
    });
  });

  describe("renderIcons() DOM hydration utility", () => {
    it("populates elements with data-icon attribute", () => {
      const mockElements: Array<{
        attrs: Record<string, string>;
        innerHTML: string;
        getAttribute(name: string): string | null;
      }> = [
        {
          attrs: { "data-icon": "clipboard", "data-icon-size": "24" },
          innerHTML: "",
          getAttribute(name: string) {
            return this.attrs[name] ?? null;
          },
        },
        {
          attrs: { "data-icon": "search", "data-icon-class": "search-svg" },
          innerHTML: "",
          getAttribute(name: string) {
            return this.attrs[name] ?? null;
          },
        },
        {
          attrs: { "data-icon": "unknown-name" },
          innerHTML: "initial",
          getAttribute(name: string) {
            return this.attrs[name] ?? null;
          },
        },
      ];

      const mockRoot = {
        querySelectorAll: (selector: string) => {
          if (selector === "[data-icon]") {
            return mockElements;
          }
          return [];
        },
      };

      // @ts-expect-error passing mock ParentNode
      renderIcons(mockRoot);

      expect(mockElements[0].innerHTML).toContain('width="24"');
      expect(mockElements[0].innerHTML).toContain("<svg");
      expect(mockElements[1].innerHTML).toContain('class="search-svg"');
      expect(mockElements[2].innerHTML).toBe("initial");
    });
  });
});
