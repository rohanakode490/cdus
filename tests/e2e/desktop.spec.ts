/**
 * CDUS Desktop Comprehensive E2E Test Suite
 * Covers: Window initialization, Sidebar navigation across all tabs,
 * QR Pairing Modal flow, and Spotlight Global Search Overlay interaction.
 *
 * Shared WebDriver infrastructure lives in ./driver-utils.ts.
 */
import { existsSync } from 'fs';
import { resolve } from 'path';
import type { ChildProcess } from 'child_process';

import {
  createTauriCapabilities,
  startTauriDriver,
  ensureDevServerIfNeeded,
  WebDriverClient,
} from './driver-utils.js';

// ============================================================================
// Comprehensive E2E Test Suite
// ============================================================================

export async function runComprehensiveE2ETest(port = 4444, binaryPath: string): Promise<boolean> {
  const baseUrl = `http://127.0.0.1:${port}`;
  console.log(`[E2E] Establishing WebDriver session with binary: ${binaryPath}...`);

  const payload = createTauriCapabilities({ applicationPath: binaryPath });
  const sessionRes = await fetch(`${baseUrl}/session`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
    signal: AbortSignal.timeout(30_000), // P3: global watchdog — never hang on dead driver
  });

  if (!sessionRes.ok) {
    console.error(`[E2E] Session creation failed: ${await sessionRes.text()}`);
    return false;
  }

  const sessionData = await sessionRes.json();
  const sessionId = sessionData.value?.sessionId || sessionData.sessionId;
  console.log(`[E2E] Session established: ${sessionId}`);

  const client = new WebDriverClient(baseUrl, sessionId);

  try {
    console.log(`\n--- Test 1: Window and Shell Initialization ---`);
    const title = await client.getTitle();
    console.log(`  ✓ Application Window Initialized (Title: "${title}")`);

    // Wait for DOM readiness and root layout
    for (let i = 0; i < 30; i++) {
      const ready = await client
        .executeScript(`return document.readyState === 'complete' && !!document.querySelector('nav')`)
        .catch(() => false);
      if (ready) break;
      await new Promise((r) => setTimeout(r, 200));
    }

    // Dismiss onboarding modal if it appears on fresh profile
    const onboardingVisible = await client.isVisible('#onboarding-modal');
    if (onboardingVisible) {
      console.log(`  [Notice] Dismissing first-run onboarding screen...`);
      await client.click('#onboarding-finish-btn');
      await new Promise((r) => setTimeout(r, 300));
    }

    console.log(`\n--- Test 2: Sidebar Tab Navigation ---`);
    const tabs = [
      { name: 'Clipboard',      selector: 'button[data-view="clipboard"]',     view: '#view-clipboard' },
      { name: 'Files',          selector: 'button[data-view="files"]',          view: '#view-files' },
      { name: 'Notifications',  selector: 'button[data-view="notifications"]',  view: '#view-notifications' },
      { name: 'Notes',          selector: 'button[data-view="notes"]',          view: '#view-notes' },
      { name: 'Audit Log',      selector: 'button[data-view="audit"]',          view: '#view-audit' },
      { name: 'Settings',       selector: 'button[data-view="settings"]',       view: '#view-settings' },
      { name: 'Devices',        selector: 'button[data-view="devices"]',        view: '#view-devices' },
    ];

    for (const tab of tabs) {
      await client.click(tab.selector);
      await new Promise((r) => setTimeout(r, 150));
      const isActive = await client.hasClass(tab.view, 'active');
      if (!isActive) {
        throw new Error(`Tab navigation failed: ${tab.name} view (${tab.view}) did not become active.`);
      }
      console.log(`  ✓ Navigated to "${tab.name}" tab -> View active`);
    }

    console.log(`\n--- Test 3: Device QR Pairing Dialog Flow ---`);
    await client.click('button[data-view="devices"]');
    await new Promise((r) => setTimeout(r, 100));

    await client.click('#show-qr-btn');
    await new Promise((r) => setTimeout(r, 200));

    const alertMsg = await client.dismissAlertIfOpen();
    if (alertMsg) {
      console.log(`  ✓ Handled expected daemon offline IPC alert: "${alertMsg}"`);
    } else {
      const qrModalOpen = await client.isVisible('#show-qr-modal');
      if (qrModalOpen) {
        console.log(`  ✓ My Pairing QR modal opened successfully.`);
        await client.click('#close-qr-btn');
        console.log(`  ✓ My Pairing QR modal closed cleanly.`);
      }
    }

    console.log(`\n--- Test 4: Global Spotlight Search Overlay ---`);
    await client.click('#sidebar-search-btn');
    await new Promise((r) => setTimeout(r, 200));

    const searchOverlayOpen = await client.isVisible('#search-overlay');
    if (!searchOverlayOpen) {
      throw new Error('Search overlay did not open upon clicking #sidebar-search-btn');
    }
    console.log(`  ✓ Spotlight Search overlay opened.`);

    const testQuery = 'quarterly-report.pdf';
    await client.type('#global-search-input', testQuery);
    await new Promise((r) => setTimeout(r, 250));

    const inputValue = await client.getInputValue('#global-search-input');
    console.log(`  ✓ Search input received typed value: "${inputValue}"`);
    if (inputValue !== testQuery) {
      throw new Error(`Expected input value to be "${testQuery}", got "${inputValue}"`);
    }

    // P3 Fix: dispatch a real Escape keydown event instead of manually toggling
    // classList.add('hidden'). This exercises the actual JS keyboard listener path
    // (document.addEventListener('keydown', ...) that was previously invisible to tests.
    await client.dispatchKeydown('Escape');
    await new Promise((r) => setTimeout(r, 200));
    const searchOverlayClosed = !(await client.isVisible('#search-overlay'));
    if (!searchOverlayClosed) {
      throw new Error('Search overlay did not close after Escape keydown.');
    }
    console.log(`  ✓ Spotlight Search overlay closed cleanly via Escape keydown.`);

    console.log(`\n=======================================================`);
    console.log(` [E2E] ALL SUITE TESTS PASSED (100% SUCCESS)`);
    console.log(`=======================================================`);
    return true;
  } finally {
    console.log(`[E2E] Terminating session ${sessionId}...`);
    await client.closeSession();
    console.log(`[E2E] Session terminated.`);
  }
}

// ============================================================================
// Standalone entry point (bun run tests/e2e/desktop.spec.ts)
// ============================================================================

if (import.meta.main) {
  let binaryPath = process.env.TAURI_BIN_PATH;
  if (!binaryPath) {
    const releasePath = resolve(process.cwd(), 'src-tauri/target/release/tauri-app');
    const debugPath = resolve(process.cwd(), 'src-tauri/target/debug/tauri-app');
    if (existsSync(releasePath)) {
      binaryPath = releasePath;
    } else if (existsSync(debugPath)) {
      binaryPath = debugPath;
    }
  }

  console.log(`=======================================================`);
  console.log(` CDUS Desktop Full E2E Test Runner`);
  console.log(`=======================================================`);
  console.log(`Binary Path: ${binaryPath || '(not found)'}`);

  if (!binaryPath || !existsSync(binaryPath)) {
    console.error(`Error: Desktop binary not found.`);
    process.exit(1);
  }

  let spawnedDriver: ChildProcess | null = null;
  let spawnedVite: ChildProcess | null = null;
  try {
    spawnedVite = await ensureDevServerIfNeeded(binaryPath);
    spawnedDriver = await startTauriDriver(4444);
    const success = await runComprehensiveE2ETest(4444, binaryPath);

    if (!success) {
      console.error(`[E2E] Test suite encountered failures.`);
      process.exit(1);
    }
  } catch (err) {
    console.error(`[E2E] Test run failed:`, err);
    process.exit(1);
  } finally {
    if (spawnedDriver) {
      console.log(`[E2E:Driver] Stopping tauri-driver process...`);
      spawnedDriver.kill('SIGTERM');
    }
    if (spawnedVite) {
      console.log(`[E2E:DevServer] Stopping Vite dev server process...`);
      spawnedVite.kill('SIGTERM');
    }
  }
}
