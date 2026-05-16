import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";

// --- State Management ---
let pairedDeviceIds: string[] = [];
let scanInterval: any = null;
let pairingInterval: any = null;
let currentPairingDevice: any = null;
const transfers = new Map<string, any>();
let currentIncomingFileHash = "";

// --- UI Elements (initialized in DOMContentLoaded) ---
let fileTransferModal: Element | null = null;
let fileAcceptBtn: Element | null = null;
let fileRejectBtn: Element | null = null;
let progressToast: Element | null = null;
let progressBar: HTMLElement | null = null;
let progressPercent: Element | null = null;
let progressLabel: Element | null = null;
let pairedList: Element | null = null;
let devicesEmpty: Element | null = null;
let discoveryList: Element | null = null;
let discoverySection: Element | null = null;
let scanBtn: Element | null = null;
let pairingModal: Element | null = null;
let cancelPairingBtn: Element | null = null;
let confirmPairingBtn: HTMLButtonElement | null = null;

// --- Helper Functions ---

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

function renderFiles() {
  const listContainer = document.querySelector("#files-list");
  const emptyState = document.querySelector("#files-empty");
  if (!listContainer) return;

  const transferList = Array.from(transfers.values()).reverse();
  
  if (transferList.length === 0) {
    listContainer.innerHTML = "";
    emptyState?.classList.remove("hidden");
    return;
  }

  emptyState?.classList.add("hidden");
  listContainer.innerHTML = "";

  transferList.forEach((transfer) => {
    const itemEl = document.createElement("div");
    itemEl.className = `file-transfer-item ${transfer.status}`;
    
    let statusIcon = "↓";
    if (transfer.direction === "outgoing") statusIcon = "↑";
    if (transfer.status === "complete") statusIcon = "✓";
    if (transfer.status === "error") statusIcon = "!";

    itemEl.innerHTML = `
      <div class="transfer-icon">${statusIcon}</div>
      <div class="transfer-details">
        <div class="transfer-row">
          <span class="file-name">${transfer.fileName}</span>
          <span class="transfer-status-text">${transfer.status}</span>
        </div>
        <div class="progress-bar-container small">
          <div class="progress-bar" style="width: ${transfer.progress}%"></div>
        </div>
        <div class="transfer-meta">
          <span>${transfer.direction === "incoming" ? "from" : "to"} ${transfer.nodeId}</span>
          <span>${transfer.progress}%</span>
        </div>
      </div>
    `;
    listContainer.appendChild(itemEl);
  });
}

function showProgressToast(label: string) {
  if (!progressToast) return;
  progressToast.classList.remove("hidden");
  if (progressLabel) progressLabel.textContent = label;
  if (progressBar) progressBar.style.width = "0%";
  if (progressPercent) progressPercent.textContent = "0%";
}

function updateProgress(percent: number) {
  if (progressBar) progressBar.style.width = `${percent}%`;
  if (progressPercent) progressPercent.textContent = `${Math.round(percent)}%`;
}

function hideProgressToast() {
  progressToast?.classList.add("hidden");
}

async function renderPairedDevices() {
  if (!pairedList) return;
  
  try {
    const devices: [string, string, string | null][] = await invoke("get_paired_devices");
    pairedDeviceIds = devices.map(([id]) => id);
    
    pairedList.innerHTML = "";
    
    if (devices.length === 0) {
      devicesEmpty?.classList.remove("hidden");
      return;
    }

    devicesEmpty?.classList.add("hidden");
    devices.forEach(([id, name, transport]) => {
      const row = document.createElement("div");
      row.className = "device-row";
      
      const isOnline = transport !== null;
      const statusClass = isOnline ? "online" : "offline";
      const statusText = isOnline ? "Online" : "Offline";
      const transportText = transport || "";
      const shortId = id.substring(0, 8);

      row.innerHTML = `
        <div class="device-info">
          <span class="device-name-label">${name} <span class="device-id-tag">#${shortId}</span></span>
          <div class="device-status">
            <span class="status-dot ${statusClass}"></span>
            <span class="device-type-label">${statusText}</span>
            ${isOnline ? `<span class="connection-path">${transportText}</span>` : ""}
          </div>
        </div>
        <div class="device-actions">
          ${isOnline ? `<button class="primary-btn send-file-btn" data-id="${id}">Send File</button>` : ""}
          <button class="secondary-btn unpair-btn" data-id="${id}">Unpair</button>
        </div>
      `;

      row.querySelector(".unpair-btn")?.addEventListener("click", () => {
        unpairDevice(id);
      });

      row.querySelector(".send-file-btn")?.addEventListener("click", () => {
        initiateFileSend(id);
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

async function initiateFileSend(nodeId: string) {
  if (nodeId === "unknown") {
    alert("Error: This device was paired using an older version of CDUS and is missing a valid Node ID.\n\nPlease click 'Unpair' for this device, then re-pair it to enable file transfers.");
    return;
  }

  try {
    const selected = await open({
      multiple: false,
      directory: false,
    });

    if (selected) {
      const path = selected as string;
      const fileName = path.split(/[/\\]/).pop() || "file";
      
      const tempId = `outgoing-${Date.now()}`;
      transfers.set(tempId, {
          fileHash: tempId,
          fileName: fileName,
          nodeId: nodeId,
          progress: 0,
          status: "preparing",
          direction: "outgoing"
      });
      renderFiles();

      await invoke("send_file", { nodeId, path });
      showProgressToast(`Sending file to ${nodeId}...`);
    }
  } catch (err) {
    console.error("Failed to initiate file send:", err);
    alert("Failed to open file picker or initiate transfer.");
  }
}

async function updateDiscoveryList() {
  if (!discoveryList) return;
  
  try {
    const discovered: [string, string, string, string, number][] = await invoke("get_discovered_devices");
    const selfNodeId: string | null = await invoke("get_state", { key: "node_id" });

    const availableDevices = discovered.filter(([id]) => !pairedDeviceIds.includes(id) && id !== selfNodeId);
    
    if (availableDevices.length === 0) {
      return;
    }

    discoveryList.innerHTML = "";
    availableDevices.forEach(([id, name, os, ip, port]) => {
      const row = document.createElement("div");
      row.className = "device-row";
      const shortId = id.substring(0, 8);
      row.innerHTML = `
        <div class="device-info">
          <span class="device-name-label">${name} <span class="device-id-tag">#${shortId}</span></span>
          <span class="device-type-label">${os} • ${ip}:${port}</span>
        </div>
        <button class="primary-btn connect-btn" data-id="${id}">Connect</button>
      `;
      
      row.querySelector(".connect-btn")?.addEventListener("click", async () => {
        try {
          await invoke("pair_with", { nodeId: id });
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
        renderPairedDevices();
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
  const confirmBtn = document.querySelector("#pairing-confirm-btn") as HTMLButtonElement;

  if (confirmBtn) {
    confirmBtn.style.setProperty("display", "block", "important");
    confirmBtn.disabled = false;
    confirmBtn.textContent = "Confirm PIN Matches";
  }

  if (isInitiator) {
    if (modalTitle) modalTitle.textContent = "Confirm Pairing";
    if (modalDesc) modalDesc.textContent = `Please verify that the PIN below matches on ${device.name}. Click Confirm once you have verified it.`;
  } else {
    if (modalTitle) modalTitle.textContent = "Incoming Pairing Request";
    if (modalDesc) modalDesc.textContent = `Device ${device.name} wants to pair. Please verify that the PIN below matches on their screen.`;
  }
  
  const digits = document.querySelectorAll(".pin-digit");
  digits.forEach((el) => {
    el.textContent = "?";
  });
}

// --- Main Application Lifecycle ---

window.addEventListener("DOMContentLoaded", () => {
  // Initialize UI Element references
  fileTransferModal = document.querySelector("#file-transfer-modal");
  fileAcceptBtn = document.querySelector("#file-accept-btn");
  fileRejectBtn = document.querySelector("#file-reject-btn");
  progressToast = document.querySelector("#transfer-progress-toast");
  progressBar = document.querySelector("#transfer-progress-bar") as HTMLElement;
  progressPercent = document.querySelector("#transfer-percent");
  progressLabel = document.querySelector("#transfer-label");
  pairedList = document.querySelector("#paired-list");
  devicesEmpty = document.querySelector("#devices-empty");
  discoveryList = document.querySelector("#discovery-list");
  discoverySection = document.querySelector("#discovery-section");
  scanBtn = document.querySelector("#scan-btn");
  pairingModal = document.querySelector("#pairing-modal");
  cancelPairingBtn = document.querySelector("#pairing-cancel-btn");
  confirmPairingBtn = document.querySelector("#pairing-confirm-btn") as HTMLButtonElement;

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
      } else if (targetViewId === "devices") {
        renderPairedDevices();
      } else if (targetViewId === "files") {
        renderFiles();
      }
    });
  });

  const activeView = document.querySelector(".view.active");
  if (activeView?.id === "view-clipboard") {
    renderClipboard();
  } else if (activeView?.id === "view-devices") {
    renderPairedDevices();
  } else if (activeView?.id === "view-files") {
    renderFiles();
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

  // --- Scan Button Logic ---
  scanBtn?.addEventListener("click", async () => {
    discoverySection?.classList.remove("hidden");
    document.querySelector("#no-discovery-hint")?.classList.add("hidden");
    
    if (scanBtn?.classList.contains("scanning")) {
      await invoke("stop_scan");
      if (scanInterval) clearInterval(scanInterval);
      scanBtn.classList.remove("scanning");
      scanBtn.innerHTML = "Scan for Devices";
      return;
    }

    scanBtn?.classList.add("scanning");
    scanBtn!.innerHTML = "<div class=\"spinner\" style=\"width: 14px; height: 14px; border-width: 2px; margin-right: 8px; display: inline-block; vertical-align: middle;\"></div><span>Stop Scan</span>";
    discoveryList!.innerHTML = "<div class=\"scanning-indicator\"><div class=\"spinner\"></div><span>Scanning for nearby devices...</span></div>";
    
    try {
      await invoke("start_scan");
      scanInterval = setInterval(updateDiscoveryList, 2000);
      
      setTimeout(async () => {
        if (scanBtn?.classList.contains("scanning")) {
          await invoke("stop_scan");
          if (scanInterval) clearInterval(scanInterval);
          scanBtn.classList.remove("scanning");
          scanBtn.innerHTML = "Scan for Devices";
          if (discoveryList?.innerHTML.includes("scanning-indicator")) {
            discoveryList.innerHTML = "";
            document.querySelector("#no-discovery-hint")?.classList.remove("hidden");
          }
        }
      }, 30000);
    } catch (err) {
      console.error("Failed to start scan:", err);
      scanBtn?.classList.remove("scanning");
      scanBtn!.innerHTML = "Scan for Devices";
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
        confirmPairingBtn!.disabled = true;
        confirmPairingBtn!.textContent = "Waiting for other device...";
        await invoke("confirm_pairing", { accepted: true });
      } catch (err) {
        console.error("Failed to confirm pairing on agent:", err);
        alert("Failed to confirm pairing.");
        confirmPairingBtn!.disabled = false;
        confirmPairingBtn!.textContent = "Confirm PIN Matches";
      }
    }
  });

  document.querySelector("#manual-connect-btn")?.addEventListener("click", async () => {
    const ipInput = document.querySelector("#manual-ip") as HTMLInputElement;
    const portInput = document.querySelector("#manual-port") as HTMLInputElement;
    const ip = ipInput.value;
    const port = parseInt(portInput.value);
    
    try {
      await invoke("manual_pair", { ip, port });
    } catch (err) {
      console.error("Manual pairing failed:", err);
      alert("Failed to connect to IP.");
    }
  });

  // --- File Transfer Listeners ---

  fileAcceptBtn?.addEventListener("click", async () => {
    if (currentIncomingFileHash) {
      await invoke("accept_file_transfer", { fileHash: currentIncomingFileHash });
      fileTransferModal?.classList.add("hidden");
      
      const transfer = transfers.get(currentIncomingFileHash);
      if (transfer) {
        transfer.status = "downloading";
        renderFiles();
      }
      
      showProgressToast("Downloading file...");
    }
  });

  fileRejectBtn?.addEventListener("click", async () => {
    if (currentIncomingFileHash) {
      await invoke("reject_file_transfer", { fileHash: currentIncomingFileHash });
      fileTransferModal?.classList.add("hidden");
      
      const transfer = transfers.get(currentIncomingFileHash);
      if (transfer) {
        transfer.status = "rejected";
        renderFiles();
      }
    }
  });

  // --- Background Polling & Events ---

  // Pairing Poll
  setInterval(async () => {
    if (pairingModal?.classList.contains("hidden")) {
      try {
        const status: [string | null, boolean, boolean, string] = await invoke("get_pairing_status");
        const [pin, active, isInitiator, remoteLabel] = status;
        if (active && pin) {
          showPairingModal({ id: "remote", name: remoteLabel, os: "Unknown" }, isInitiator);
          const digits = document.querySelectorAll(".pin-digit");
          digits.forEach((el, i) => {
            el.textContent = pin[i];
          });
          startPairingPoll();
        }
      } catch (err) { }
    }
  }, 1000);

  // Agent Event Stream
  listen("incoming-file-request", (event: any) => {
    console.log("UI: Received incoming-file-request", event.payload);
    const [nodeId, manifest] = event.payload;
    currentIncomingFileHash = manifest.file_hash;
    
    transfers.set(manifest.file_hash, {
      fileHash: manifest.file_hash,
      fileName: manifest.file_name,
      nodeId: nodeId,
      progress: 0,
      status: "pending",
      direction: "incoming"
    });
    renderFiles();

    if (fileTransferModal) {
      const nameEl = document.querySelector("#incoming-file-name");
      const sizeEl = document.querySelector("#incoming-file-size");
      const sourceEl = document.querySelector("#incoming-file-source");
      
      if (nameEl) nameEl.textContent = manifest.file_name;
      if (sizeEl) sizeEl.textContent = `${(manifest.total_size / 1024 / 1024).toFixed(2)} MB`;
      if (sourceEl) sourceEl.textContent = `from ${nodeId}`;
      
      fileTransferModal.classList.remove("hidden");
    }
  });

  listen("file-transfer-progress", (event: any) => {
    const [fileHash, progress] = event.payload;
    updateProgress(progress);
    
    const transfer = transfers.get(fileHash);
    if (transfer) {
      transfer.progress = Math.round(progress);
      if (transfer.status === "pending" || transfer.status === "preparing") transfer.status = "active";
      renderFiles();
    }
  });

  listen("file-transfer-complete", (event: any) => {
    const fileHash = event.payload;
    if (progressLabel) progressLabel.textContent = "Transfer complete!";
    updateProgress(100);
    setTimeout(hideProgressToast, 3000);
    
    const transfer = transfers.get(fileHash);
    if (transfer) {
      transfer.progress = 100;
      transfer.status = "complete";
      renderFiles();
    }
  });

  listen("file-transfer-error", (event: any) => {
    const [fileHash, error] = event.payload;
    alert(`Transfer failed: ${error}`);
    hideProgressToast();
    
    const transfer = transfers.get(fileHash);
    if (transfer) {
      transfer.status = "error";
      transfer.error = error;
      renderFiles();
    }
  });

  listen("clipboard-updated", (_event: any) => {
    renderClipboard();
  });

  // Initial load
  renderPairedDevices();

  // Periodic Refresh
  setInterval(() => {
    const devicesView = document.querySelector("#view-devices");
    if (devicesView?.classList.contains("active")) {
      renderPairedDevices();
    }
    
    const clipboardView = document.querySelector("#view-clipboard");
    if (clipboardView?.classList.contains("active")) {
      renderClipboard();
    }
  }, 5000);
});
