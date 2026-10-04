/**
 * Shared WebDriver utilities for CDUS Desktop E2E test suites.
 *
 * Extracted from desktop.spec.ts and cross_device.spec.ts to eliminate ~90 lines
 * of copy-paste. Import from here in all E2E spec files.
 */
import { spawn, type ChildProcess } from 'child_process';

// ============================================================================
// Types
// ============================================================================

export interface WebDriverSessionOptions {
  applicationPath: string;
}

// ============================================================================
// tauri-driver lifecycle helpers
// ============================================================================

/**
 * Builds the W3C capabilities object required to open a tauri-driver session
 * against the CDUS desktop binary.
 */
export function createTauriCapabilities(options: WebDriverSessionOptions) {
  return {
    capabilities: {
      alwaysMatch: {
        'tauri:options': {
          application: options.applicationPath,
        },
      },
    },
  };
}

/**
 * Returns true if a WebDriver server is already accepting connections on
 * the given port.  Times out after 1 s so startup loops don't stall.
 */
export async function checkDriverHealth(port = 4444): Promise<boolean> {
  try {
    const res = await fetch(`http://127.0.0.1:${port}/status`, {
      signal: AbortSignal.timeout(1000),
    });
    return res.ok;
  } catch {
    return false;
  }
}

/**
 * Spawns `tauri-driver` on the requested port if it isn't already running.
 *
 * Polls for readiness with 250 ms intervals up to 5 s.
 * Returns the spawned `ChildProcess` (or `null` if a server was already live)
 * so callers can kill it in their `finally` block.
 */
export async function startTauriDriver(port = 4444): Promise<ChildProcess | null> {
  if (await checkDriverHealth(port)) {
    console.log(`[E2E:Driver] tauri-driver is already active on port ${port}.`);
    return null;
  }

  console.log(`[E2E:Driver] Spawning tauri-driver on port ${port}...`);
  const driverProcess = spawn('tauri-driver', ['--port', port.toString()], {
    stdio: 'pipe',
    env: process.env,
  });

  for (let i = 0; i < 20; i++) {
    await new Promise((r) => setTimeout(r, 250));
    if (await checkDriverHealth(port)) {
      console.log(`[E2E:Driver] tauri-driver is ready.`);
      return driverProcess;
    }
  }

  return driverProcess;
}

/**
 * Starts the Vite dev server on port 1420 if the binary path contains
 * "debug" and no dev server is already running.
 *
 * Release binaries embed the frontend — no server needed.
 */
export async function ensureDevServerIfNeeded(
  binaryPath: string
): Promise<ChildProcess | null> {
  try {
    const res = await fetch('http://localhost:1420', {
      signal: AbortSignal.timeout(500),
    });
    if (res.ok) {
      console.log(`[E2E:DevServer] Dev server is already active on http://localhost:1420.`);
      return null;
    }
  } catch {}

  if (binaryPath.includes('debug')) {
    console.log(`[E2E:DevServer] Debug binary detected. Spawning Vite dev server on port 1420...`);
    const viteProcess = spawn('bun', ['run', 'dev'], {
      stdio: 'pipe',
      env: process.env,
    });

    for (let i = 0; i < 30; i++) {
      await new Promise((r) => setTimeout(r, 250));
      try {
        const res = await fetch('http://localhost:1420', {
          signal: AbortSignal.timeout(500),
        });
        if (res.ok) {
          console.log(`[E2E:DevServer] Vite dev server is ready.`);
          return viteProcess;
        }
      } catch {}
    }
    return viteProcess;
  }
  return null;
}

// ============================================================================
// W3C WebDriver client
// ============================================================================

/**
 * Minimal W3C WebDriver HTTP client for the CDUS desktop (tauri-driver) session.
 *
 * Avoids a heavy WebDriver library dependency. All interactions go through the
 * raw HTTP protocol so tests remain portable across driver implementations.
 */
export class WebDriverClient {
  constructor(
    private readonly baseUrl: string,
    private readonly sessionId: string
  ) {}

  async executeScript<T = unknown>(script: string, args: unknown[] = []): Promise<T> {
    const res = await fetch(`${this.baseUrl}/session/${this.sessionId}/execute/sync`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ script, args }),
    });
    if (!res.ok) {
      throw new Error(`executeScript failed: ${await res.text()}`);
    }
    const data = await res.json();
    return data.value as T;
  }

  async executeAsyncScript<T = unknown>(script: string, args: unknown[] = []): Promise<T> {
    const res = await fetch(`${this.baseUrl}/session/${this.sessionId}/execute/async`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ script, args }),
    });
    if (!res.ok) {
      throw new Error(`executeAsyncScript failed: ${await res.text()}`);
    }
    const data = await res.json();
    return data.value as T;
  }

  /**
   * Polls for an element matching the CSS `selector` up to `timeoutMs`.
   * Returns the W3C element ID string, or throws on timeout.
   */
  async getElement(selector: string, timeoutMs = 4000): Promise<string> {
    const startTime = Date.now();
    while (Date.now() - startTime < timeoutMs) {
      const res = await fetch(`${this.baseUrl}/session/${this.sessionId}/element`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ using: 'css selector', value: selector }),
      });
      if (res.ok) {
        const data = await res.json();
        const elemId =
          data.value?.['element-6066-11e4-a52e-4f735466cecf'] || data.value?.ELEMENT;
        if (elemId) return elemId as string;
      }
      await new Promise((r) => setTimeout(r, 150));
    }
    const preview = await this.executeScript<string>(
      `return document.body ? document.body.innerHTML.slice(0, 300) : 'no-body'`
    ).catch(() => 'error');
    throw new Error(
      `Element not found: ${selector} (timeout ${timeoutMs}ms). Body preview: ${preview}`
    );
  }

  async click(selector: string): Promise<void> {
    const elemId = await this.getElement(selector);
    const res = await fetch(
      `${this.baseUrl}/session/${this.sessionId}/element/${elemId}/click`,
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({}),
      }
    );
    if (!res.ok) {
      // Fallback via script if the element is intercepted by a CSS transition
      await this.executeScript(`document.querySelector(arguments[0])?.click();`, [selector]);
    }
  }

  async type(selector: string, text: string): Promise<void> {
    const elemId = await this.getElement(selector);
    const res = await fetch(
      `${this.baseUrl}/session/${this.sessionId}/element/${elemId}/value`,
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ text, value: text.split('') }),
      }
    );
    if (!res.ok) {
      await this.executeScript(
        `const el = document.querySelector(arguments[0]);
         if (el) { el.value = arguments[1]; el.dispatchEvent(new Event('input', { bubbles: true })); }`,
        [selector, text]
      );
    }
  }

  async dismissAlertIfOpen(): Promise<string | null> {
    try {
      const textRes = await fetch(`${this.baseUrl}/session/${this.sessionId}/alert/text`);
      if (textRes.ok) {
        const textData = await textRes.json();
        const alertMsg = textData.value as string;
        await fetch(`${this.baseUrl}/session/${this.sessionId}/alert/accept`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: '{}',
        });
        return alertMsg;
      }
    } catch {}
    return null;
  }

  async getTitle(): Promise<string> {
    const res = await fetch(`${this.baseUrl}/session/${this.sessionId}/title`);
    const data = await res.json();
    return data.value as string;
  }

  async isVisible(selector: string): Promise<boolean> {
    return this.executeScript<boolean>(
      `const el = document.querySelector(arguments[0]);
       if (!el) return false;
       const style = window.getComputedStyle(el);
       return !el.classList.contains('hidden') && style.display !== 'none' && style.visibility !== 'hidden';`,
      [selector]
    );
  }

  async hasClass(selector: string, className: string): Promise<boolean> {
    return this.executeScript<boolean>(
      `const el = document.querySelector(arguments[0]);
       return el ? el.classList.contains(arguments[1]) : false;`,
      [selector, className]
    );
  }

  async getInputValue(selector: string): Promise<string> {
    return this.executeScript<string>(
      `const el = document.querySelector(arguments[0]); return el ? el.value : '';`,
      [selector]
    );
  }

  /**
   * Dispatches a keyboard event on `document` — the proper way to test
   * keyboard-triggered UI behaviour (e.g., Escape to close an overlay).
   * Uses dispatchEvent rather than direct DOM manipulation so JS listeners fire.
   */
  async dispatchKeydown(key: string, opts: { bubbles?: boolean } = {}): Promise<void> {
    await this.executeScript(
      `document.dispatchEvent(new KeyboardEvent('keydown', {
         key: arguments[0], bubbles: arguments[1] !== false
       }));`,
      [key, opts.bubbles ?? true]
    );
  }

  /**
   * Invokes a Tauri command via `window.__TAURI_INTERNALS__.invoke`.
   * Returns the resolved value, or throws if the command rejects.
   */
  async invokeTauri<T = unknown>(
    cmd: string,
    args: Record<string, unknown> = {}
  ): Promise<T> {
    const result = await this.executeAsyncScript<{ value?: T; error?: string }>(
      `const done = arguments[arguments.length - 1];
       if (window.__TAURI_INTERNALS__?.invoke) {
         window.__TAURI_INTERNALS__.invoke(arguments[0], arguments[1])
           .then(res => done({ value: res }))
           .catch(err => done({ error: String(err) }));
       } else {
         done({ error: 'Tauri internals not available' });
       }`,
      [cmd, args]
    );
    if (result?.error) {
      throw new Error(`Tauri invoke '${cmd}' failed: ${result.error}`);
    }
    return result?.value as T;
  }

  async closeSession(): Promise<void> {
    try {
      await fetch(`${this.baseUrl}/session/${this.sessionId}`, { method: 'DELETE' });
    } catch {}
  }
}
