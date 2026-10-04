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

  // ==========================================================================
  // Mission 15: Notification Mirroring & Dismiss Sync
  // ==========================================================================
  describe("Notification Mirroring UI Contracts (Mission 15)", () => {
    it("renders the notifications view container and list root", () => {
      expect(indexContent).toContain('id="view-notifications"');
      expect(indexContent).toContain('id="notifications-list"');
      expect(indexContent).toContain('id="notifications-list-container"');
    });

    it("includes a clear-all button for bulk dismissal", () => {
      expect(indexContent).toContain('id="clear-notifications-btn"');
    });

    it("includes a retry button for error recovery", () => {
      expect(indexContent).toContain('id="retry-notifications-btn"');
    });

    it("renders loading and error state containers", () => {
      expect(indexContent).toContain('id="notifications-loading"');
      expect(indexContent).toContain('id="notifications-error"');
    });

    it("renders an empty-state placeholder mentioning mirrored notifications", () => {
      expect(indexContent).toContain("No active mirrored notifications from your phone.");
    });

    it("invokes get_active_notifications to populate the list", () => {
      expect(mainTsContent).toContain('invoke("get_active_notifications")');
    });

    it("wires dismiss_notification IPC on individual card dismiss buttons", () => {
      expect(mainTsContent).toContain('invoke("dismiss_notification"');
      expect(mainTsContent).toContain("key: notif.key");
    });

    it("listens to the 'notification-mirrored' backend event", () => {
      expect(mainTsContent).toContain('listen("notification-mirrored"');
    });

    it("listens to the 'notification-dismissed' backend event for cross-device sync", () => {
      expect(mainTsContent).toContain('listen("notification-dismissed"');
    });

    it("wires clear-notifications-btn click to bulk dismiss_notification calls", () => {
      expect(mainTsContent).toContain(
        'document.querySelector("#clear-notifications-btn")?.addEventListener("click"'
      );
      expect(mainTsContent).toContain('invoke("dismiss_notification"');
    });
  });

  // ==========================================================================
  // Mission 20: Cross-Device Settings Sync (LWW)
  // ==========================================================================
  describe("Settings LWW Sync UI Contracts (Mission 20)", () => {
    it("renders the settings view container", () => {
      expect(indexContent).toContain('id="view-settings"');
      expect(indexContent).toContain('class="settings-container"');
    });

    it("includes loading and error state containers", () => {
      expect(indexContent).toContain('id="settings-loading"');
      expect(indexContent).toContain('id="settings-error"');
      expect(indexContent).toContain('id="retry-settings-btn"');
    });

    it("renders the save-settings button", () => {
      expect(indexContent).toContain('id="save-settings-btn"');
    });

    it("save-settings-btn handler calls update_setting for sync_enabled key", () => {
      expect(mainTsContent).toContain('invoke("update_setting"');
      expect(mainTsContent).toContain('"sync_enabled"');
    });

    it("save-settings-btn handler calls update_setting for clipboard_limit key", () => {
      expect(mainTsContent).toContain('"clipboard_limit"');
    });

    it("save-settings-btn handler calls set_state for device_name key", () => {
      expect(mainTsContent).toContain('invoke("set_state"');
      expect(mainTsContent).toContain('"device_name"');
    });

    it("wires save-settings-btn to the click listener in main.ts", () => {
      expect(mainTsContent).toContain(
        'document.querySelector("#save-settings-btn")?.addEventListener("click"'
      );
    });

    it("wires retry-settings-btn to reload the settings view", () => {
      expect(mainTsContent).toContain(
        'document.querySelector("#retry-settings-btn")?.addEventListener("click"'
      );
    });
  });

  // ==========================================================================
  // Mission 23: Collaborative Notes & CRDT Sync
  // ==========================================================================
  describe("CRDT Notes Sync UI Contracts (Mission 23)", () => {
    it("renders the notes view container and layout shell", () => {
      expect(indexContent).toContain('id="view-notes"');
      expect(indexContent).toContain('class="notes-layout"');
    });

    it("renders the notes sidebar with list, filter input, and empty state", () => {
      expect(indexContent).toContain('id="notes-list"');
      expect(indexContent).toContain('id="notes-filter-input"');
      expect(indexContent).toContain('id="notes-empty"');
    });

    it("renders the active editor container and no-selection placeholder", () => {
      expect(indexContent).toContain('id="notes-active-editor"');
      expect(indexContent).toContain('id="notes-no-selection"');
    });

    it("renders the title input and content textarea for the active note editor", () => {
      expect(indexContent).toContain('id="note-title-input"');
      expect(indexContent).toContain('id="note-content-input"');
    });

    it("renders the note-sync-status badge that shows live CRDT convergence state", () => {
      expect(indexContent).toContain('id="note-sync-status"');
      // Verify default copy reflects a 'Synced' baseline state
      expect(indexContent).toContain(">Synced<");
    });

    it("renders new-note and delete-note action buttons", () => {
      expect(indexContent).toContain('id="new-note-btn"');
      expect(indexContent).toContain('id="delete-note-btn"');
    });

    it("includes a loading state container for the notes panel", () => {
      expect(indexContent).toContain('id="notes-loading"');
    });

    it("invokes get_notes to load the note list from the Rust core", () => {
      expect(mainTsContent).toContain('invoke("get_notes")');
    });

    it("invokes save_note with docId, title, and content to persist CRDT patches", () => {
      expect(mainTsContent).toContain('invoke("save_note"');
      expect(mainTsContent).toContain("docId");
      expect(mainTsContent).toContain("title");
      expect(mainTsContent).toContain("content");
    });

    it("invokes delete_note to remove a document from the CRDT store", () => {
      expect(mainTsContent).toContain('invoke("delete_note"');
      expect(mainTsContent).toContain("docId");
    });

    it("wires note-title-input and note-content-input to the debounced save scheduler", () => {
      expect(mainTsContent).toContain(
        'document.querySelector("#note-title-input")?.addEventListener("input"'
      );
      expect(mainTsContent).toContain(
        'document.querySelector("#note-content-input")?.addEventListener("input"'
      );
      // Both should point to the same debounce function
      expect(mainTsContent).toContain("scheduleNoteSave");
    });

    it("sets note-sync-status class to 'note-status-badge syncing' while a save is in-flight", () => {
      expect(mainTsContent).toContain('note-status-badge syncing');
    });

    it("resets note-sync-status class to 'note-status-badge' on successful save", () => {
      // The class is reset (without 'syncing' suffix) after the invoke resolves
      expect(mainTsContent).toContain('statusBadge.className = "note-status-badge"');
    });
  });
});

