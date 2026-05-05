import { invoke } from "@tauri-apps/api/core";

async function renderClipboard() {
  const listContainer = document.querySelector("#clipboard-list");
  const emptyState = document.querySelector("#clipboard-empty");
  if (!listContainer) return;

  try {
    const history: any[] = await invoke("get_clipboard_history", { limit: 50 });
    
    if (history.length === 0) {
      listContainer.innerHTML = "";
      emptyState?.classList.remove("hidden");
      return;
    }

    emptyState?.classList.add("hidden");
    listContainer.innerHTML = "";
    
    history.forEach((item) => {
      const itemEl = document.createElement("div");
      itemEl.className = "clipboard-item";
      itemEl.innerHTML = `
        <div class="clipboard-content">${item.content}</div>
        <div class="clipboard-meta">
          <span class="device-badge">Local</span>
          <span class="timestamp">${item.timestamp}</span>
        </div>
        <div class="copy-feedback">Copied!</div>
      `;

      itemEl.addEventListener("click", () => {
        navigator.clipboard.writeText(item.content).then(() => {
          const feedback = itemEl.querySelector(".copy-feedback");
          feedback?.classList.add("show");
          setTimeout(() => feedback?.classList.remove("show"), 1500);
        });
      });

      listContainer.appendChild(itemEl);
    });
  } catch (err) {
    console.error("Failed to fetch history:", err);
  }
}

async function loadSettings() {
  const deviceNameInput = document.querySelector("#device-name") as HTMLInputElement;
  const syncEnabledInput = document.querySelector("#sync-enabled") as HTMLInputElement;
  const limitSlider = document.querySelector("#clipboard-limit") as HTMLInputElement;
  const limitValue = document.querySelector("#limit-value");

  try {
    const deviceName: string | null = await invoke("get_state", { key: "device_name" });
    const syncEnabled: string | null = await invoke("get_state", { key: "sync_enabled" });
    const limit: string | null = await invoke("get_state", { key: "clipboard_limit" });

    if (deviceName && deviceNameInput) deviceNameInput.value = deviceName;
    if (syncEnabled && syncEnabledInput) syncEnabledInput.checked = syncEnabled === "true";
    if (limit && limitSlider) {
      limitSlider.value = limit;
      if (limitValue) limitValue.textContent = `${limit} items`;
    }
  } catch (err) {
    console.error("Failed to load settings:", err);
  }
}

window.addEventListener("DOMContentLoaded", () => {
  loadSettings();
  const navItems = document.querySelectorAll(".nav-item");
  const views = document.querySelectorAll(".view");

  navItems.forEach((item) => {
    item.addEventListener("click", () => {
      const targetViewId = item.getAttribute("data-view");

      navItems.forEach((nav) => nav.classList.remove("active"));
      item.classList.add("active");

      views.forEach((view) => {
        view.classList.remove("active");
        if (view.id === `view-${targetViewId}`) {
          view.classList.add("active");
        }
      });

      if (targetViewId === "clipboard") {
        renderClipboard();
      }
    });
  });

  const activeView = document.querySelector(".view.active");
  if (activeView?.id === "view-clipboard") {
    renderClipboard();
  }

  document.querySelector("#add-device-btn")?.addEventListener("click", () => {
    alert("Discovery mode started! Scanning for nearby devices...");
  });

  const limitSlider = document.querySelector("#clipboard-limit") as HTMLInputElement;
  const limitValue = document.querySelector("#limit-value");
  
  limitSlider?.addEventListener("input", (e) => {
    const val = (e.target as HTMLInputElement).value;
    if (limitValue) limitValue.textContent = `${val} items`;
  });

  document.querySelector("#save-settings-btn")?.addEventListener("click", async () => {
    const deviceName = (document.querySelector("#device-name") as HTMLInputElement).value;
    const syncEnabled = (document.querySelector("#sync-enabled") as HTMLInputElement).checked;
    const limit = limitSlider.value;
    
    try {
      await invoke("set_state", { key: "device_name", value: deviceName });
      await invoke("set_state", { key: "sync_enabled", value: syncEnabled.toString() });
      await invoke("set_state", { key: "clipboard_limit", value: limit });
      alert("Settings saved successfully!");
    } catch (err) {
      console.error("Failed to save settings:", err);
      alert("Failed to save settings.");
    }
  });

  document.querySelector("#send-file-btn")?.addEventListener("click", () => {
    alert("File picker opened! (Mock)");
  });

  const pingBtn = document.querySelector("#ping-agent-btn");
  const pingStatus = document.querySelector("#ping-status");

  pingBtn?.addEventListener("click", async () => {
    if (pingStatus) pingStatus.textContent = "Pinging...";
    try {
      const response = await invoke("ping_agent");
      if (pingStatus) pingStatus.textContent = `Response: ${response}`;
    } catch (err) {
      if (pingStatus) pingStatus.textContent = `Error: ${err}`;
    }
  });

  const setCbBtn = document.querySelector("#set-clipboard-btn");
  const cbInput = document.querySelector("#test-clipboard-input") as HTMLInputElement;
  const cbStatus = document.querySelector("#set-clipboard-status");

  setCbBtn?.addEventListener("click", async () => {
    if (!cbInput.value) return;
    if (cbStatus) cbStatus.textContent = "Setting...";
    try {
      const response = await invoke("set_clipboard", { content: cbInput.value });
      if (cbStatus) cbStatus.textContent = `Response: ${response}`;
    } catch (err) {
      if (cbStatus) cbStatus.textContent = `Error: ${err}`;
    }
  });

  // Refresh clipboard history every 5 seconds if visible
  setInterval(() => {
    const clipboardView = document.querySelector("#view-clipboard");
    if (clipboardView?.classList.contains("active")) {
      renderClipboard();
    }
  }, 5000);
});
