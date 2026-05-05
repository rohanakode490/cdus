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
          <span class="device-badge">${item.source}</span>
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

  // --- Q2 Discovery & Pairing Real Logic ---
  
  const scanBtn = document.querySelector("#scan-btn");
  const discoverySection = document.querySelector("#discovery-section");
  const discoveryList = document.querySelector("#discovery-list");
  const pairingModal = document.querySelector("#pairing-modal");
  const cancelPairingBtn = document.querySelector("#pairing-cancel-btn");
  const confirmPairingBtn = document.querySelector("#pairing-confirm-btn");
  const pairedList = document.querySelector("#paired-list");
  const devicesEmpty = document.querySelector("#devices-empty");

  // Local state
  let pairedDeviceIds: string[] = [];
  let scanInterval: any = null;

  async function updateDiscoveryList() {
    if (!discoveryList) return;
    
    try {
      const discovered: [string, string, string][] = await invoke("get_discovered_devices");
      
      // Filter out already paired devices
      const availableDevices = discovered.filter(([id]) => !pairedDeviceIds.includes(id));
      
      if (availableDevices.length === 0) {
        return;
      }

      discoveryList.innerHTML = "";
      availableDevices.forEach(([id, name, os]) => {
        const row = document.createElement("div");
        row.className = "device-row";
        row.innerHTML = `
          <div class="device-info">
            <span class="device-name-label">${name}</span>
            <span class="device-type-label">${os}</span>
          </div>
          <button class="primary-btn connect-btn" data-id="${id}">Connect</button>
        `;
        
        row.querySelector(".connect-btn")?.addEventListener("click", () => {
          showPairingModal({ id, name, os });
        });
        
        discoveryList.appendChild(row);
      });
    } catch (err) {
      console.error("Failed to fetch discovered devices:", err);
    }
  }

  let currentPairingDevice: any = null;

  function showPairingModal(device: any) {
    if (!pairingModal) return;
    currentPairingDevice = device;
    pairingModal.classList.remove("hidden");
    
    // Generate random PIN
    const pin = Math.floor(1000 + Math.random() * 9000).toString();
    const digits = document.querySelectorAll(".pin-digit");
    digits.forEach((el, i) => {
      el.textContent = pin[i];
    });
  }

  function renderPairedDevices() {
    if (!pairedList) return;
    pairedList.innerHTML = "";
    
    if (pairedDeviceIds.length === 0) {
      devicesEmpty?.classList.remove("hidden");
      return;
    }

    devicesEmpty?.classList.add("hidden");
    pairedDeviceIds.forEach(id => {
      // In a real app, we'd fetch this from the agent's trusted list
      // For now we just use the ID as a placeholder or cache the name
      const row = document.createElement("div");
      row.className = "device-row";
      row.innerHTML = `
        <div class="device-info">
          <span class="device-name-label">Device ${id.substring(0, 8)}</span>
          <div class="device-status">
            <span class="status-dot online"></span>
            <span class="device-type-label">Online</span>
          </div>
        </div>
        <button class="secondary-btn unpair-btn" data-id="${id}">Unpair</button>
      `;

      row.querySelector(".unpair-btn")?.addEventListener("click", () => {
        unpairDevice(id);
      });

      pairedList.appendChild(row);
    });
  }

  function unpairDevice(id: string) {
    if (confirm("Are you sure you want to unpair this device?")) {
      pairedDeviceIds = pairedDeviceIds.filter(pid => pid !== id);
      renderPairedDevices();
    }
  }

  scanBtn?.addEventListener("click", async () => {
    discoverySection?.classList.remove("hidden");
    document.querySelector("#no-discovery-hint")?.classList.add("hidden");
    
    if (scanBtn.textContent === "Scanning...") {
      await invoke("stop_scan");
      if (scanInterval) clearInterval(scanInterval);
      scanBtn.textContent = "Scan for Devices";
      return;
    }

    scanBtn.textContent = "Scanning...";
    discoveryList!.innerHTML = "<div class=\"scanning-indicator\"><div class=\"spinner\"></div><span>Scanning for nearby devices...</span></div>";
    
    try {
      await invoke("start_scan");
      scanInterval = setInterval(updateDiscoveryList, 2000);
      
      setTimeout(async () => {
        if (scanBtn.textContent === "Scanning...") {
          await invoke("stop_scan");
          if (scanInterval) clearInterval(scanInterval);
          scanBtn.textContent = "Scan for Devices";
          if (discoveryList?.innerHTML.includes("scanning-indicator")) {
            discoveryList.innerHTML = "";
            document.querySelector("#no-discovery-hint")?.classList.remove("hidden");
          }
        }
      }, 30000);
    } catch (err) {
      console.error("Failed to start scan:", err);
      scanBtn.textContent = "Scan for Devices";
    }
  });

  cancelPairingBtn?.addEventListener("click", () => {
    pairingModal?.classList.add("hidden");
    currentPairingDevice = null;
  });

  confirmPairingBtn?.addEventListener("click", () => {
    if (currentPairingDevice) {
      pairedDeviceIds.push(currentPairingDevice.id);
      renderPairedDevices();
      alert(`Device ${currentPairingDevice.name} paired successfully!`);
      pairingModal?.classList.add("hidden");
      currentPairingDevice = null;
      
      discoveryList!.innerHTML = "<div class=\"scanning-indicator\"><div class=\"spinner\"></div><span>Scanning for nearby devices...</span></div>";
      discoverySection?.classList.add("hidden");
    }
  });

  // --- End Q2 Real Logic ---

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
