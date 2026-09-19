import { describe, it, expect } from "bun:test";
import { readFileSync } from "fs";
import { resolve } from "path";

describe("Frontend UI Contracts & Template Integrity", () => {
  const rootDir = resolve(__dirname, "../..");
  const indexPath = resolve(rootDir, "index.html");
  const mainTsPath = resolve(rootDir, "src/main.ts");
  const stylesPath = resolve(rootDir, "src/styles.css");

  const indexContent = readFileSync(indexPath, "utf-8");
  const mainTsContent = readFileSync(mainTsPath, "utf-8");
  const stylesContent = readFileSync(stylesPath, "utf-8");

  const emojiRegex = /[\u{10000}-\u{10ffff}\u{2600}-\u{27bf}\u{2300}-\u{23ff}\u{2b50}]/u;

  describe("Emoji Removal Regression Suite", () => {
    it("ensures zero emojis exist anywhere in index.html", () => {
      const match = indexContent.match(emojiRegex);
      expect(match).toBeNull();
    });

    it("ensures zero emojis exist in search result items or icon variables in main.ts", () => {
      // main.ts only contains '✓' character for transfer status, no emoji icons
      const lines = mainTsContent.split("\n");
      const emojiLines = lines.filter((line) => {
        // Exclude checkmark character used in progress indicator if matched
        const clean = line.replace(/✓/g, "");
        return emojiRegex.test(clean);
      });
      expect(emojiLines).toEqual([]);
    });
  });

  describe("Onboarding Modal Structure (index.html)", () => {
    it("contains onboarding overlay container and all 3 onboarding steps", () => {
      expect(indexContent).toContain('id="onboarding-overlay"');
      expect(indexContent).toContain('id="onboarding-step-1"');
      expect(indexContent).toContain('id="onboarding-step-2"');
      expect(indexContent).toContain('id="onboarding-step-3"');
    });

    it("ensures step 3 feature rows use SVGs instead of emojis", () => {
      expect(indexContent).toContain("Clipboard Synchronization");
      expect(indexContent).toContain("P2P File Transfer");
      expect(indexContent).toContain("End-to-End Encryption");

      // Verify that each feature-icon has an inline svg
      const featureIconSvgMatches = indexContent.match(
        /<span class="feature-icon"[^>]*>[\s\S]*?<svg[\s\S]*?<\/svg>[\s\S]*?<\/span>/g
      );
      expect(featureIconSvgMatches).not.toBeNull();
      expect(featureIconSvgMatches?.length).toBe(3);
    });
  });

  describe("Navigation & Modals Vector Icons (index.html)", () => {
    it("ensures sidebar search button uses SVG icon instead of emoji", () => {
      const sidebarMatch = indexContent.match(
        /<button[^>]*id="sidebar-search-btn"[^>]*>[\s\S]*?<svg[\s\S]*?<\/svg>[\s\S]*?Search/
      );
      expect(sidebarMatch).not.toBeNull();
    });

    it("ensures search overlay header uses SVG icon instead of emoji", () => {
      const searchHeaderMatch = indexContent.match(
        /<div class="search-overlay-header">[\s\S]*?<span class="search-icon"[^>]*>[\s\S]*?<svg[\s\S]*?<\/svg>[\s\S]*?<\/span>/
      );
      expect(searchHeaderMatch).not.toBeNull();
    });

    it("ensures search feedback button uses SVG icon instead of emoji", () => {
      const feedbackMatch = indexContent.match(
        /<button[^>]*id="search-feedback-btn"[^>]*>[\s\S]*?<svg[\s\S]*?<\/svg>[\s\S]*?Send Feedback<\/button>/
      );
      expect(feedbackMatch).not.toBeNull();
    });

    it("ensures error recovery modal uses SVG warning icon instead of emoji", () => {
      const errorIconMatch = indexContent.match(
        /<span class="error-icon"[^>]*>[\s\S]*?<svg[\s\S]*?<\/svg>[\s\S]*?<\/span>/
      );
      expect(errorIconMatch).not.toBeNull();
    });
  });

  describe("CSS Rules for SVG Icons (src/styles.css)", () => {
    it("defines .feature-icon container styling with SVG child sizing", () => {
      expect(stylesContent).toContain(".feature-icon {");
      expect(stylesContent).toContain(".feature-icon svg {");
      expect(stylesContent).toContain("width: 20px;");
      expect(stylesContent).toContain("height: 20px;");
    });

    it("defines .result-icon with SVG child sizing", () => {
      expect(stylesContent).toContain(".result-icon {");
      expect(stylesContent).toContain(".result-icon svg {");
      expect(stylesContent).toContain("width: 16px;");
      expect(stylesContent).toContain("height: 16px;");
    });

    it("defines .favicon-fallback with SVG child sizing", () => {
      expect(stylesContent).toContain(".favicon-fallback {");
      expect(stylesContent).toContain(".favicon-fallback svg {");
    });
  });

  describe("Application Lifecycle & Icon Registration (src/main.ts)", () => {
    it("imports icon constants and renderIcons from ./icons", () => {
      expect(mainTsContent).toContain('from "./icons"');
      expect(mainTsContent).toContain("renderIcons");
    });

    it("registers renderIcons on DOMContentLoaded lifecycle", () => {
      expect(mainTsContent).toContain('window.addEventListener("DOMContentLoaded"');
      expect(mainTsContent).toContain("renderIcons();");
    });
  });

  describe("Device Revocation UI Contracts (src/main.ts & src/styles.css)", () => {
    it("includes a Revoke action button with danger styling in paired device rows", () => {
      expect(mainTsContent).toContain('class="danger-btn revoke-btn"');
      expect(mainTsContent).toContain('data-id="${id}"');
    });

    it("wires the revoke-btn click listener to revokeDevice", () => {
      expect(mainTsContent).toContain('.querySelector(".revoke-btn")?.addEventListener("click"');
      expect(mainTsContent).toContain("revokeDevice(id, name);");
    });

    it("prompts the user with a security lockout confirmation and calls revoke_device invoke", () => {
      expect(mainTsContent).toContain("async function revokeDevice");
      expect(mainTsContent).toContain('invoke("revoke_device"');
      expect(mainTsContent).toContain("uuid: id");
      expect(mainTsContent).toContain('addAuditLog("system"');
    });

    it("verifies danger-btn CSS rules for visual affordance", () => {
      expect(stylesContent).toContain(".danger-btn {");
      expect(stylesContent).toContain("cursor: pointer;");
    });
  });
});
