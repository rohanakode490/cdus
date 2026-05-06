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
      if (targetViewId === "devices") {
        renderPairedDevices();
      }
    });
  });

  const activeView = document.querySelector(".view.active");
  if (activeView?.id === "view-clipboard") {
    renderClipboard();
  }
  if (activeView?.id === "view-devices") {
    renderPairedDevices();
  }

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
  let pairingInterval: any = null;
  let currentPairingDevice: any = null;

  // Background check for incoming pairing requests
  setInterval(async () => {
    // Only poll if modal is NOT already open
    if (pairingModal?.classList.contains("hidden")) {
      try {
        const [pin, active, isInitiator, remoteLabel]: [string | null, boolean, boolean, string] = await invoke("get_pairing_status");
        if (active && pin) {
          console.log("Detected active pairing state. isInitiator:", isInitiator);
          showPairingModal({ id: "remote", name: remoteLabel, os: "Unknown" }, isInitiator);
          const digits = document.querySelectorAll(".pin-digit");
          digits.forEach((el, i) => {
            el.textContent = pin[i];
          });
          startPairingPoll();
        }
      } catch (err) {
        // Silently ignore background poll errors
      }
    }
  }, 1000);

  async function updateDiscoveryList() {
    if (!discoveryList) return;
    
    try {
      const discovered: [string, string, string, string, number][] = await invoke("get_discovered_devices");
      const selfNodeId: string | null = await invoke("get_state", { key: "node_id" });

      // Filter out already paired devices AND self
      const availableDevices = discovered.filter(([id]) => !pairedDeviceIds.includes(id) && id !== selfNodeId);
      
      if (availableDevices.length === 0) {
        return;
      }

      discoveryList.innerHTML = "";
      availableDevices.forEach(([id, name, os, _ip, _port]) => {
        const row = document.createElement("div");
        row.className = "device-row";
        row.innerHTML = `
          <div class="device-info">
            <span class="device-name-label">${name}</span>
            <span class="device-type-label">${os}</span>
          </div>
          <button class="primary-btn connect-btn" data-id="${id}">Connect</button>
        `;
        
        row.querySelector(".connect-btn")?.addEventListener("click", async () => {
          try {
            await invoke("pair_with", { nodeId: id });
            // Note: Modal will be shown by background poll once agent updates state
          } catch (err) {
            console.error("Failed to initiate pairing:", err);
            alert("Failed to connect to device.");
          }
        });
        
        discoveryList.appendChild(row);
      });
    } catch (err) {
      console.error("Failed to fetch discovered devices:", err);
    }
  }

  async function startPairingPoll() {
    if (pairingInterval) clearInterval(pairingInterval);
    
    pairingInterval = setInterval(async () => {
      try {
        const [pin, active, _isInitiator, _remoteLabel]: [string | null, boolean, boolean, string] = await invoke("get_pairing_status");
        if (pin) {
          const digits = document.querySelectorAll(".pin-digit");
          digits.forEach((el, i) => {
            el.textContent = pin[i];
          });
        }
        if (!active && pairingInterval) {
          clearInterval(pairingInterval);
          pairingModal?.classList.add("hidden");
          renderPairedDevices(); // Refresh list to show success
        }
      } catch (err) {
        console.error("Error polling pairing status:", err);
      }
    }, 1000);
  }

  function showPairingModal(device: any, isInitiator: boolean) {
    if (!pairingModal) return;
    currentPairingDevice = device;
    pairingModal.classList.remove("hidden");
    
    const modalTitle = pairingModal.querySelector("h3");
    const modalDesc = pairingModal.querySelector("p");
    const confirmBtn = document.querySelector("#pairing-confirm-btn") as HTMLElement;

    if (isInitiator) {
      console.log("UI: Acting as Initiator. Hiding confirm button.");
      if (modalTitle) modalTitle.textContent = "Waiting for Confirmation";
      if (modalDesc) modalDesc.textContent = `Please verify that the PIN below matches on ${device.name}. Waiting for them to confirm...`;
      if (confirmBtn) {
        confirmBtn.style.setProperty("display", "none", "important");
      }
    } else {
      console.log("UI: Acting as Responder. Showing confirm button.");
      if (modalTitle) modalTitle.textContent = "Pair Device";
      if (modalDesc) modalDesc.textContent = `Incoming pairing request from ${device.name}. Enter the 4-digit PIN shown on the other device:`;
      if (confirmBtn) {
        confirmBtn.style.setProperty("display", "block", "important");
      }
    }
    
    // Placeholder while waiting for real PIN from agent
    const digits = document.querySelectorAll(".pin-digit");
    digits.forEach((el) => {
      el.textContent = "?";
    });
  }

  async function renderPairedDevices() {
    if (!pairedList) return;
    
    try {
      const devices: [string, string][] = await invoke("get_paired_devices");
      pairedDeviceIds = devices.map(([id]) => id);
      
      pairedList.innerHTML = "";
      
      if (devices.length === 0) {
        devicesEmpty?.classList.remove("hidden");
        return;
      }

      devicesEmpty?.classList.add("hidden");
      devices.forEach(([id, name]) => {
        const row = document.createElement("div");
        row.className = "device-row";
        row.innerHTML = `
          <div class="device-info">
            <span class="device-name-label">${name}</span>
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
    } catch (err) {
      console.error("Failed to render paired devices:", err);
    }
  }

  async function unpairDevice(id: string) {
    if (confirm("Are you sure you want to unpair this device?")) {
      try {
        await invoke("unpair_device", { nodeId: id });
        renderPairedDevices();
      } catch (err) {
        console.error("Failed to unpair device:", err);
      }
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

  cancelPairingBtn?.addEventListener("click", async () => {
    try {
      await invoke("confirm_pairing", { accepted: false });
    } catch (err) {
      console.error("Failed to cancel pairing on agent:", err);
    }
    pairingModal?.classList.add("hidden");
    if (pairingInterval) clearInterval(pairingInterval);
    currentPairingDevice = null;
  });

  confirmPairingBtn?.addEventListener("click", async () => {
    if (currentPairingDevice) {
      try {
        await invoke("confirm_pairing", { accepted: true });
        // UI will be closed by startPairingPoll when active becomes false
      } catch (err) {
        console.error("Failed to confirm pairing on agent:", err);
        alert("Failed to confirm pairing.");
      }
    }
  });

  document.querySelector("#manual-connect-btn")?.addEventListener("click", async () => {
    const ip = (document.querySelector("#manual-ip") as HTMLInputElement).value;
    const port = parseInt((document.querySelector("#manual-port") as HTMLInputElement).value);
    
    try {
      await invoke("manual_pair", { ip, port });
      // Modal will be shown by background poll
    } catch (err) {
      console.error("Manual pairing failed:", err);
      alert("Failed to connect to IP.");
    }
  });

  // --- End Q2 Real Logic ---

  document.querySelector("#send-file-btn")?.addEventListener("click", () => {
    alert("File picker opened! (Mock)");
  });

  // Refresh clipboard history every 5 seconds if visible
  setInterval(() => {
    const clipboardView = document.querySelector("#view-clipboard");
    if (clipboardView?.classList.contains("active")) {
      renderClipboard();
    }
  }, 5000);

  // Initial load of paired devices
  renderPairedDevices();
});
