import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import QRCode from "qrcode";
import { Html5Qrcode } from "html5-qrcode";

// --- State Management ---
let pairedDeviceIds: string[] = [];
const deviceLabels = new Map<string, string>();
let scanInterval: any = null;
let pairingInterval: any = null;
let currentPairingDevice: any = null;
const transfers = new Map<string, any>();
let isDeveloperMode = false;
let currentIncomingTransferId = "";
let html5QrCode: Html5Qrcode | null = null;

// --- UI Elements (initialized in DOMContentLoaded) ---
let fileTransferModal: Element | null = null;
let fileAcceptBtn: Element | null = null;
let fileRejectBtn: Element | null = null;
let progressToast: Element | null = null;
let progressBar: HTMLElement | null = null;
let progressPercent: Element | null = null;
let progressLabel: Element | null = null;
let progressFilename: Element | null = null;
let closeProgressBtn: Element | null = null;
let pairedList: Element | null = null;
let devicesEmpty: Element | null = null;
let discoveryList: Element | null = null;
let discoverySection: Element | null = null;
let scanBtn: Element | null = null;
let showQrBtn: Element | null = null;
let scanQrBtn: Element | null = null;
let showQrModal: Element | null = null;
let scanQrModal: Element | null = null;
let closeQrBtn: Element | null = null;
let closeScanQrBtn: Element | null = null;
let pairingModal: Element | null = null;
let cancelPairingBtn: Element | null = null;
let confirmPairingBtn: HTMLButtonElement | null = null;

// --- Helper Functions ---

async function renderClipboard() {
  const listContainer = document.querySelector("#clipboard-list");
  const emptyState = document.querySelector("#clipboard-empty");
  if (!listContainer) return;

  // Proactively check system clipboard and broadcast if new
  await checkSystemClipboard();

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

let lastLocalClipboard = "";
async function checkSystemClipboard() {
  try {
    const syncEnabled: string | null = await invoke("get_state", { key: "sync_enabled" });
    if (syncEnabled !== "true") return;

    const content = await navigator.clipboard.readText();
    if (content && content !== lastLocalClipboard) {
      lastLocalClipboard = content;
      console.log("New system clipboard detected on Desktop, broadcasting");
      await invoke("broadcast_clipboard", { content });
    }
  } catch (err) {
    // Might fail if window not focused, ignore
  }
}

async function loadFileHistory() {
  try {
    const history: any[] = await invoke("get_file_transfer_history", { limit: 50 });
    history.forEach((record) => {
      let status = record.status;
      if (status === "in_progress" || status === "paused" || status === "awaiting_acceptance") {
        status = "paused";
      }

      transfers.set(record.transfer_id, {
        transferId: record.transfer_id,
        fileName: record.file_name,
        nodeId: record.peer_node_id,
        progress: record.total_bytes > 0 ? Math.round((Number(record.bytes_confirmed) / Number(record.total_bytes)) * 100) : 0,
        status: status,
        direction: record.direction,
        error: record.error_message,
        totalBytes: Number(record.total_bytes)
      });
    });
    renderFiles();
  } catch (err) {
    console.error("Failed to load file history:", err);
  }
}

let currentSortOrder = "newest";

function renderFiles() {
  const listContainer = document.querySelector("#files-list");
  const emptyState = document.querySelector("#files-empty");
  if (!listContainer) return;

  let transferList = Array.from(transfers.values());
  if (currentSortOrder === "newest") {
    transferList.reverse();
  } else if (currentSortOrder === "name-asc") {
    transferList.sort((a, b) => a.fileName.localeCompare(b.fileName, undefined, { sensitivity: "base" }));
  } else if (currentSortOrder === "name-desc") {
    transferList.sort((a, b) => b.fileName.localeCompare(a.fileName, undefined, { sensitivity: "base" }));
  } else if (currentSortOrder === "size-desc") {
    transferList.sort((a, b) => (b.totalBytes || 0) - (a.totalBytes || 0));
  } else if (currentSortOrder === "size-asc") {
    transferList.sort((a, b) => (a.totalBytes || 0) - (b.totalBytes || 0));
  } // "oldest" maintains insertion order (no-op)
  
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

    const isFinished = transfer.status === "error" || transfer.status === "complete" || transfer.status === "rejected";
    
    itemEl.innerHTML = `
      <div class="transfer-icon">${statusIcon}</div>
      <div class="transfer-details">
        <div class="transfer-row">
          <span class="file-name">${transfer.fileName}</span>
          <span class="transfer-status-text ${transfer.status === "error" ? "error-text" : ""}">${transfer.status}</span>
        </div>
        ${transfer.status === "error" ? `<div class="error-message">${transfer.error || "Unknown error"}</div>` : ""}
        <div class="progress-bar-container small">
          <div class="progress-bar" style="width: ${transfer.progress}%"></div>
        </div>
        <div class="transfer-meta">
          <span>${transfer.direction === "incoming" ? "from" : "to"} ${getDeviceLabel(transfer.nodeId)}</span>
          <span style="color: var(--tertiary-color);">${transfer.speedMbps ? transfer.speedMbps.toFixed(1) + " Mbps" : ""}</span>
          <span>${transfer.progress}%</span>
        </div>
      </div>
      <div class="transfer-actions-menu">
        <button class="options-trigger-btn" data-id="${transfer.transferId}">⋮</button>
        <div class="options-dropdown hidden" id="dropdown-${transfer.transferId}">
          ${isFinished ? `
            <button class="dismiss-btn dropdown-option" data-id="${transfer.transferId}">Dismiss</button>
          ` : `
            <button class="cancel-btn danger-option dropdown-option" data-id="${transfer.transferId}">Cancel</button>
          `}
        </div>
      </div>
    `;

    const triggerBtn = itemEl.querySelector(".options-trigger-btn");
    const dropdown = itemEl.querySelector(".options-dropdown");
    
    triggerBtn?.addEventListener("click", (e) => {
      e.stopPropagation();
      document.querySelectorAll(".options-dropdown").forEach((el) => {
        if (el !== dropdown) {
          el.classList.add("hidden");
        }
      });
      dropdown?.classList.toggle("hidden");
    });

    if (isFinished) {
      itemEl.querySelector(".dismiss-btn")?.addEventListener("click", () => {
        transfers.delete(transfer.transferId);
        renderFiles();
      });
    } else {
      itemEl.querySelector(".cancel-btn")?.addEventListener("click", async () => {
        const id = transfer.transferId;
        try {
          await invoke("cancel_file_transfer", { transferId: id });
          const t = transfers.get(id);
          if (t) {
            t.status = "error";
            t.error = "Cancelled";
            renderFiles();
          }
        } catch (err) {
          console.error("Failed to cancel transfer:", err);
        }
      });
    }
    listContainer.appendChild(itemEl);
  });
}

function getDeviceLabel(nodeId: string): string {
  return deviceLabels.get(nodeId) || nodeId.substring(0, 8);
}

function showProgressToast(label: string, filename: string = "") {
  if (!progressToast) return;
  progressToast.classList.remove("hidden");
  if (progressLabel) progressLabel.textContent = label;
  if (progressFilename) progressFilename.textContent = filename || "File";
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
    
    // Update labels map
    deviceLabels.clear();
    devices.forEach(([id, label]) => deviceLabels.set(id, label));
    
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
          ${isOnline ? `<button class="primary-btn send-file-btn" data-id="${id}">Send File</button>` : `<button class="primary-btn connect-btn" data-id="${id}">Connect</button>`}
          ${(isOnline && isDeveloperMode) ? `<button class="tertiary-btn benchmark-btn" data-id="${id}" style="margin-right: 8px;">Benchmark</button>` : ""}
          <button class="secondary-btn unpair-btn" data-id="${id}">Unpair</button>
        </div>
      `;

      row.querySelector(".benchmark-btn")?.addEventListener("click", async () => {
        try {
          await invoke("start_benchmark", { nodeId: id });
          alert("1GB Network Benchmark started. Watch progress in the Files tab.");
        } catch (err) {
          console.error("Failed to start benchmark:", err);
        }
      });

      row.querySelector(".unpair-btn")?.addEventListener("click", () => {
        unpairDevice(id);
      });

      row.querySelector(".connect-btn")?.addEventListener("click", async () => {
        try {
          await invoke("pair_with", { nodeId: id });
          alert("Connection attempt started...");
        } catch (err) {
          console.error("Failed to initiate connection:", err);
          alert("Failed to initiate connection.");
        }
      });

      row.querySelector(".send-file-btn")?.addEventListener("click", () => {
        initiateFileSend(id);
      });

      pairedList?.appendChild(row);
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
      
      transfers.set(path, {
          transferId: path,
          fileName: fileName,
          nodeId: nodeId,
          progress: 0,
          status: "preparing",
          direction: "outgoing"
      });
      renderFiles();

      await invoke("send_file", { nodeId, path });
      showProgressToast(`Sending file to ${getDeviceLabel(nodeId)}...`, fileName);
    }
  } catch (err) {
    console.error("Failed to initiate file send:", err);
    alert("Failed to open file picker or initiate transfer.");
  }
}

async function updateDiscoveryList() {
  if (!discoveryList) return;
  
  try {
    const discovered: [string, string, string, string[], number][] = await invoke("get_discovered_devices");
    const selfNodeId: string | null = await invoke("get_state", { key: "node_id" });

    const availableDevices = discovered.filter(([id]) => !pairedDeviceIds.includes(id) && id !== selfNodeId);
    
    discoveryList.innerHTML = "";
    if (availableDevices.length === 0) {
      return;
    }
    availableDevices.forEach(([id, name, os]) => {
      const row = document.createElement("div");
      row.className = "device-row";
      const shortId = id.substring(0, 8);
      row.innerHTML = `
        <div class="device-info">
          <span class="device-name-label">${name} <span class="device-id-tag">#${shortId}</span></span>
          <span class="device-type-label">${os}</span>
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
      
      discoveryList?.appendChild(row);
    });
  } catch (err) {
    console.error("Failed to fetch discovered devices:", err);
  }
}

async function startPairingPoll() {
  if (pairingInterval) clearInterval(pairingInterval);
  
  pairingInterval = setInterval(async () => {
    try {
      const [pin, active, _isInitiator, _remoteLabel, _silent]: [string | null, boolean, boolean, string, boolean] = await invoke("get_pairing_status");
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

function showPairingModal(device: any, isInitiator: boolean, silent: boolean = false) {
  if (!pairingModal) return;
  currentPairingDevice = device;
  pairingModal.classList.remove("hidden");
  
  const modalTitle = pairingModal.querySelector("h3");
  const modalDesc = pairingModal.querySelector("p");
  const confirmBtn = document.querySelector("#pairing-confirm-btn") as HTMLButtonElement;

  if (confirmBtn) {
    if (silent) {
      confirmBtn.style.setProperty("display", "none", "important");
    } else {
      confirmBtn.style.setProperty("display", "block", "important");
      confirmBtn.disabled = false;
      confirmBtn.textContent = "Confirm PIN Matches";
    }
  }

  if (silent) {
    if (modalTitle) modalTitle.textContent = "Auto-Pairing...";
    if (modalDesc) modalDesc.textContent = `Pairing with ${device.name} using QR code. Please wait...`;
  } else if (isInitiator) {
    if (modalTitle) modalTitle.textContent = "Confirm Pairing";
    if (modalDesc) modalDesc.textContent = `Please verify that the PIN below matches on ${device.name}. Click Confirm once you have verified it.`;
  } else {
    if (modalTitle) modalTitle.textContent = "Incoming Pairing Request";
    if (modalDesc) modalDesc.textContent = `Device ${device.name} wants to pair. Please verify that the PIN below matches on their screen.`;
  }
  
  const digits = document.querySelectorAll(".pin-digit");
  digits.forEach((el) => {
    el.textContent = silent ? "." : "?";
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
  progressFilename = document.querySelector("#transfer-filename");
  closeProgressBtn = document.querySelector("#close-progress-toast");
  pairedList = document.querySelector("#paired-list");

  closeProgressBtn?.addEventListener("click", hideProgressToast);
  devicesEmpty = document.querySelector("#devices-empty");
  discoveryList = document.querySelector("#discovery-list");
  discoverySection = document.querySelector("#discovery-section");
  scanBtn = document.querySelector("#scan-btn");
  showQrBtn = document.querySelector("#show-qr-btn");
  scanQrBtn = document.querySelector("#scan-qr-btn");
  showQrModal = document.querySelector("#show-qr-modal");
  scanQrModal = document.querySelector("#scan-qr-modal");
  closeQrBtn = document.querySelector("#close-qr-btn");
  closeScanQrBtn = document.querySelector("#close-scan-qr-btn");
  pairingModal = document.querySelector("#pairing-modal");
  cancelPairingBtn = document.querySelector("#pairing-cancel-btn");
  confirmPairingBtn = document.querySelector("#pairing-confirm-btn") as HTMLButtonElement;

  loadSettings();
  loadFileHistory();
  document.querySelector("#clear-finished-btn")?.addEventListener("click", () => {
    transfers.forEach((transfer, hash) => {
      if (transfer.status === "complete" || transfer.status === "error") {
        transfers.delete(hash);
      }
    });
    renderFiles();
  });

  const sortTrigger = document.querySelector("#files-sort-trigger");
  const sortDropdown = document.querySelector("#files-sort-dropdown");
  
  sortTrigger?.addEventListener("click", (e) => {
    e.stopPropagation();
    document.querySelectorAll(".options-dropdown").forEach(el => el.classList.add("hidden"));
    sortDropdown?.classList.toggle("hidden");
  });

  document.querySelectorAll(".custom-select-option").forEach((opt) => {
    opt.addEventListener("click", () => {
      const value = opt.getAttribute("data-value") || "newest";
      currentSortOrder = value;
      
      document.querySelectorAll(".custom-select-option").forEach((o) => o.classList.remove("active"));
      opt.classList.add("active");
      
      const labelEl = document.querySelector("#files-sort-current");
      if (labelEl) labelEl.textContent = opt.textContent;
      
      sortDropdown?.classList.add("hidden");
      renderFiles();
    });
  });

  document.addEventListener("click", () => {
    document.querySelectorAll(".options-dropdown").forEach((el) => {
      el.classList.add("hidden");
    });
    sortDropdown?.classList.add("hidden");
  });

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
  
  let tapCount = 0;
  document.querySelector("#version-text")?.addEventListener("click", () => {
    tapCount++;
    if (tapCount >= 7) {
      isDeveloperMode = true;
      alert("Developer Mode enabled!");
      tapCount = 0;
      renderPairedDevices();
    }
  });

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

  showQrBtn?.addEventListener("click", async () => {
    try {
      const payload: string = await invoke("get_qr_pairing_payload");
      const container = document.querySelector("#qr-container");
      if (container) {
        container.innerHTML = "";
        const canvas = document.createElement("canvas");
        container.appendChild(canvas);
        await QRCode.toCanvas(canvas, payload, { width: 250, margin: 2 });
      }
      showQrModal?.classList.remove("hidden");
    } catch (err) {
      console.error("Failed to generate QR:", err);
      alert("Failed to generate pairing QR.");
    }
  });

  closeQrBtn?.addEventListener("click", () => {
    showQrModal?.classList.add("hidden");
  });

  scanQrBtn?.addEventListener("click", async () => {
    scanQrModal?.classList.remove("hidden");
    if (!html5QrCode) {
      html5QrCode = new Html5Qrcode("reader");
    }

    try {
      await html5QrCode.start(
        { facingMode: "user" },
        { fps: 10, qrbox: { width: 250, height: 250 } },
        async (decodedText) => {
          console.log("QR Decoded:", decodedText);
          stopScanQr();

          // Show connecting feedback
          if (pairingModal) {
            showPairingModal({ id: "remote", name: "Device", os: "Unknown" }, true, true);
          }

          try {
            await invoke("pair_with_qr", { payload: decodedText });
            // Start polling to wait for PIN derivation or completion
            startPairingPoll();
          } catch (err: any) {
            console.error("Failed to pair with QR:", err);
            pairingModal?.classList.add("hidden");
            if (err.toString().includes("Already paired")) {
              alert("This device is already paired!");
            } else {
              alert("Pairing failed: " + err);
            }
          }
        },
        (_errorMessage) => {
          // ignore scan errors
        }
      );
    } catch (err) {
      console.error("Failed to start camera:", err);
      alert("Could not access camera.");
      scanQrModal?.classList.add("hidden");
    }
  });

  async function stopScanQr() {
    if (html5QrCode && html5QrCode.isScanning) {
      try {
        await html5QrCode.stop();
      } catch (err) {
        console.error("Failed to stop scanner:", err);
      }
    }
    scanQrModal?.classList.add("hidden");
  }

  closeScanQrBtn?.addEventListener("click", stopScanQr);

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
    if (currentIncomingTransferId) {
      await invoke("accept_file_transfer", { transferId: currentIncomingTransferId });
      fileTransferModal?.classList.add("hidden");
      
      const transfer = transfers.get(currentIncomingTransferId);
      if (transfer) {
        transfer.status = "downloading";
        renderFiles();
      }
      
      showProgressToast("Downloading file...");
    }
  });

  fileRejectBtn?.addEventListener("click", async () => {
    if (currentIncomingTransferId) {
      await invoke("reject_file_transfer", { transferId: currentIncomingTransferId });
      fileTransferModal?.classList.add("hidden");
      
      const transfer = transfers.get(currentIncomingTransferId);
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
        const status: [string | null, boolean, boolean, string, boolean] = await invoke("get_pairing_status");
        const [pin, active, isInitiator, remoteLabel, silent] = status;
        if (active && (pin || silent)) {
          showPairingModal({ id: "remote", name: remoteLabel, os: "Unknown" }, isInitiator, silent);
          if (pin) {
            const digits = document.querySelectorAll(".pin-digit");
            digits.forEach((el, i) => {
              el.textContent = pin[i];
            });
          }
          startPairingPoll();
        }
      } catch (err) { }
    }
  }, 1000);

  listen("incoming-file-offer", (event: any) => {
    console.log("UI: Received incoming-file-offer", event.payload);
    const [nodeId, offer] = event.payload;
    currentIncomingTransferId = offer.file_hash;
    
    transfers.set(offer.file_hash, {
      transferId: offer.file_hash,
      fileName: offer.file_name,
      nodeId: nodeId,
      progress: 0,
      status: "offered",
      direction: "incoming",
      totalBytes: Number(offer.total_size)
    });
    renderFiles();

    if (fileTransferModal) {
      const nameEl = document.querySelector("#incoming-file-name");
      const sizeEl = document.querySelector("#incoming-file-size");
      const sourceEl = document.querySelector("#incoming-file-source");
      
      if (nameEl) nameEl.textContent = offer.file_name;
      if (sizeEl) sizeEl.textContent = `${(offer.total_size / 1024 / 1024).toFixed(2)} MB`;
      if (sourceEl) sourceEl.textContent = `from ${getDeviceLabel(nodeId)}`;

      fileTransferModal.classList.remove("hidden");
    }
  });

  // Agent Event Stream
  listen("incoming-file-request", (event: any) => {
    console.log("UI: Received incoming-file-request", event.payload);
    const [nodeId, manifest] = event.payload;
    currentIncomingTransferId = manifest.file_hash;
    
    transfers.set(manifest.file_hash, {
      transferId: manifest.file_hash,
      fileName: manifest.file_name,
      nodeId: nodeId,
      progress: 0,
      status: "pending",
      direction: "incoming",
      totalBytes: Number(manifest.total_size)
    });
    renderFiles();

    if (fileTransferModal) {
      const nameEl = document.querySelector("#incoming-file-name");
      const sizeEl = document.querySelector("#incoming-file-size");
      const sourceEl = document.querySelector("#incoming-file-source");
      
      if (nameEl) nameEl.textContent = manifest.file_name;
      if (sizeEl) sizeEl.textContent = `${(manifest.total_size / 1024 / 1024).toFixed(2)} MB`;
      if (sourceEl) sourceEl.textContent = `from ${getDeviceLabel(nodeId)}`;

      fileTransferModal.classList.remove("hidden");
      showProgressToast("Downloading file...", manifest.file_name);
    }
  });

  listen("manifest-progress", (event: any) => {
    const { path, progress } = event.payload;
    const fileName = path.split(/[/\\]/).pop() || "file";
    
    const transfer = transfers.get(path);
    if (transfer) {
      transfer.progress = Math.round(progress);
      transfer.status = "hashing";
      updateProgress(progress);
      renderFiles();
    } else {
      // If we don't have it yet (e.g. from Android), add it
      transfers.set(path, {
        transferId: path,
        fileName: fileName,
        nodeId: "Remote",
        progress: Math.round(progress),
        status: "hashing",
        direction: "outgoing"
      });
      renderFiles();
    }
  });

  listen("file-transfer-progress", (event: any) => {
    const [transferId, progress] = event.payload;
    const now = Date.now();
    
    // Check if we have it by ID
    let transfer = transfers.get(transferId);
    
    if (!transfer) {
        // Check if we have a "hashing" transfer that should now be this ID
        const pending = Array.from(transfers.values()).find(t => t.status === "hashing" || t.status === "preparing" || t.status === "pending");
        if (pending) {
            transfers.delete(pending.transferId);
            pending.transferId = transferId;
            transfers.set(transferId, pending);
            transfer = pending;
        }
    }

    if (transfer) {
      const isBenchmark = transferId === "ffffffff-ffff-ffff-ffff-ffffffffffff";
      const totalSize = isBenchmark ? 1024 * 1024 * 1024 : 10 * 1024 * 1024;
      if (!transfer.totalBytes) {
        transfer.totalBytes = totalSize;
      }
      if (transfer.lastUpdate) {
        const timeDiff = (now - transfer.lastUpdate) / 1000;
        const progressDiff = progress - transfer.progress;
        if (progressDiff > 0 && timeDiff > 0) {
          const bytesDiff = (progressDiff / 100) * totalSize;
          transfer.speedMbps = (bytesDiff * 8 / (1024 * 1024)) / timeDiff;
        }
      }
      transfer.lastUpdate = now;
      
      updateProgress(progress);
      if (progressFilename) progressFilename.textContent = transfer.fileName;
      
      transfer.progress = Math.round(progress);
      if (transfer.status === "pending" || transfer.status === "preparing" || transfer.status === "hashing") transfer.status = "active";
      renderFiles();
    }
  });

  listen("file-transfer-complete", (event: any) => {
    const transferId = event.payload;
    if (progressLabel) progressLabel.textContent = "Transfer complete!";
    updateProgress(100);
    setTimeout(hideProgressToast, 3000);
    
    const transfer = transfers.get(transferId);
    if (transfer) {
      transfer.progress = 100;
      transfer.status = "complete";
      renderFiles();
    }
  });

  listen("file-transfer-error", (event: any) => {
    const [transferId, error] = event.payload;
    alert(`Transfer failed: ${error}`);
    hideProgressToast();
    
    const transfer = transfers.get(transferId);
    if (transfer) {
      transfer.status = "error";
      transfer.error = error;
      renderFiles();
    }
  });

  listen("clipboard-updated", (_event: any) => {
    renderClipboard();
  });

  listen("peer-disconnected", (_event: any) => {
    renderPairedDevices();
  });

  listen("pairing-result", (event: any) => {
    const [success, _nodeId, label] = event.payload;
    if (success) {
      console.log(`Pairing successful with ${label}`);
      renderPairedDevices().then(() => {
        updateDiscoveryList();
      });
    }
  });

  listen("peer-connected", (_event: any) => {
    renderPairedDevices();
  });

  window.addEventListener("focus", () => {
    checkSystemClipboard();
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
    } else {
      // Check system clipboard even if view not active
      checkSystemClipboard();
    }
  }, 5000);
});
