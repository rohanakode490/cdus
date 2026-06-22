import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { listen } from "@tauri-apps/api/event";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import QRCode from "qrcode";
import { Html5Qrcode } from "html5-qrcode";

function formatTimestamp(timestampStr: string): string {
  try {
    const isoStr = timestampStr.trim().replace(" ", "T") + "Z";
    const date = new Date(isoStr);
    if (isNaN(date.getTime())) {
      return timestampStr;
    }
    return date.toLocaleString(undefined, {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
      hour12: false
    });
  } catch (e) {
    return timestampStr;
  }
}


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

let currentOnboardingStep = 1;

interface NotificationPayload {
  key: string;
  package_name: string;
  app_name: string;
  title: string;
  text: string;
  timestamp: number;
}

let activeNotifications: NotificationPayload[] = [];
let notificationsLoaded = false;



async function addAuditLog(type: "sync" | "pairing" | "system", content: string) {
  try {
    await invoke("append_audit_log", { eventType: type, content });
  } catch (err) {
    console.error("Failed to append audit log:", err);
  }
  const auditView = document.querySelector("#view-audit");
  if (auditView?.classList.contains("active")) {
    renderAuditLogs();
  }
}

function sanitizeErrorMessage(err: string): string {
  if (!err) return "";
  let sanitized = err.replace(/https?:\/\/[^\s]+/g, "[URL]");
  sanitized = sanitized.replace(/wss?:\/\/[^\s]+/g, "[URL]");
  sanitized = sanitized.replace(/\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}(:\d+)?\b/g, "[address]");
  sanitized = sanitized.replace(/\b[0-9a-fA-F]{32,}\b/g, "[id]");
  return sanitized;
}

// --- Error Recovery UI Modal ---
function showErrorModal(
  title: string,
  message: string,
  technicalDetails?: string,
  retryAction?: () => void,
  troubleshootingUrl?: string
) {
  const modal = document.querySelector("#error-modal") as HTMLElement;
  const titleEl = document.querySelector("#error-modal-title") as HTMLElement;
  const messageEl = document.querySelector("#error-modal-message") as HTMLElement;
  const detailsContainer = document.querySelector("#error-modal-details-container") as HTMLElement;
  const detailsEl = document.querySelector("#error-modal-details") as HTMLElement;
  const troubleshootEl = document.querySelector("#error-modal-troubleshoot") as HTMLElement;
  const closeBtn = document.querySelector("#error-modal-close") as HTMLElement;
  const retryBtn = document.querySelector("#error-modal-retry") as HTMLElement;

  if (!modal) return;

  titleEl.textContent = title;
  messageEl.textContent = message;

  if (technicalDetails) {
    detailsEl.textContent = sanitizeErrorMessage(technicalDetails);
    detailsContainer.style.display = "block";
  } else {
    detailsContainer.style.display = "none";
  }

  if (troubleshootingUrl) {
    troubleshootEl.setAttribute("href", troubleshootingUrl);
    troubleshootEl.style.display = "inline-flex";
  } else {
    troubleshootEl.style.display = "none";
  }

  if (retryAction) {
    retryBtn.style.display = "inline-block";
    const newRetryBtn = retryBtn.cloneNode(true) as HTMLButtonElement;
    retryBtn.parentNode?.replaceChild(newRetryBtn, retryBtn);
    newRetryBtn.addEventListener("click", () => {
      modal.classList.add("hidden");
      retryAction();
    });
  } else {
    retryBtn.style.display = "none";
  }

  const newCloseBtn = closeBtn.cloneNode(true) as HTMLButtonElement;
  closeBtn.parentNode?.replaceChild(newCloseBtn, closeBtn);
  newCloseBtn.addEventListener("click", () => {
    modal.classList.add("hidden");
  });

  modal.classList.remove("hidden");
}

// Hook up the troubleshoot link click opening externally via plugin-opener
document.addEventListener("DOMContentLoaded", () => {
  document.querySelector("#error-modal-troubleshoot")?.addEventListener("click", (e) => {
    e.preventDefault();
    const url = (e.currentTarget as HTMLElement).getAttribute("href");
    if (url) {
      openUrl(url).catch((err: any) => console.error("Failed to open URL:", err));
    }
  });
});

// --- Device Connection States ---
interface MockDeviceState {
  status: "online" | "offline" | "connecting";
  transport: string | null;
  timerId?: any;
}
const mockDeviceStates = new Map<string, MockDeviceState>();

function triggerDeviceConnection(id: string) {
  let s = mockDeviceStates.get(id);
  if (!s) {
    s = { status: "offline", transport: null };
    mockDeviceStates.set(id, s);
  }
  
  if (s.timerId) {
    clearTimeout(s.timerId);
  }

  s.status = "connecting";
  renderPairedDevices();

  // Reset to offline if connection does not succeed within 15 seconds
  s.timerId = setTimeout(() => {
    const current = mockDeviceStates.get(id);
    if (current && current.status === "connecting") {
      current.status = "offline";
      renderPairedDevices();
    }
  }, 15000);
}


async function renderAuditLogs() {
  const listContainer = document.querySelector("#audit-list");
  const emptyState = document.querySelector("#audit-empty");
  const loadingEl = document.querySelector("#audit-loading");
  const errorEl = document.querySelector("#audit-error");

  if (!listContainer) return;

  loadingEl?.classList.remove("hidden");
  errorEl?.classList.add("hidden");
  listContainer.classList.add("hidden");
  emptyState?.classList.add("hidden");

  try {
    const logs: any[] = await invoke("get_audit_logs", { limit: 100 });
    loadingEl?.classList.add("hidden");
    
    if (logs.length === 0) {
      listContainer.innerHTML = "";
      emptyState?.classList.remove("hidden");
      return;
    }

    emptyState?.classList.add("hidden");
    listContainer.classList.remove("hidden");
    listContainer.innerHTML = "";

    logs.forEach((log) => {
      const itemEl = document.createElement("div");
      itemEl.className = "audit-item";
      
      const timeStr = new Date(Number(log.timestamp)).toLocaleString();
      itemEl.innerHTML = `
        <span class="audit-badge ${log.event_type}">${log.event_type}</span>
        <div class="audit-content">${log.content}</div>
        <span class="audit-time">${timeStr}</span>
      `;
      
      listContainer.appendChild(itemEl);
    });
  } catch (err) {
    console.error("Failed to load audit logs:", err);
    loadingEl?.classList.add("hidden");
    errorEl?.classList.remove("hidden");
    listContainer.classList.add("hidden");
    emptyState?.classList.add("hidden");
  }
}

async function initOnboarding() {
  const overlay = document.querySelector("#onboarding-overlay");
  if (!overlay) return;

  try {
    const completed = await invoke("get_state", { key: "onboarding_completed" });
    if (completed === "true") {
      overlay.classList.add("hidden");
    } else {
      overlay.classList.remove("hidden");
      setupOnboardingFlow(overlay);
    }
  } catch (err) {
    console.error("Failed to check onboarding state:", err);
    overlay.classList.remove("hidden");
    setupOnboardingFlow(overlay);
  }
}

function setupOnboardingFlow(overlay: Element) {
  currentOnboardingStep = 1;
  showStep(1);

  const nextBtns = overlay.querySelectorAll(".onboarding-next-btn");
  nextBtns.forEach((btn) => {
    btn.addEventListener("click", () => {
      if (currentOnboardingStep < 3) {
        currentOnboardingStep++;
        showStep(currentOnboardingStep);
      }
    });
  });

  const finishBtn = overlay.querySelector("#onboarding-finish-btn");
  finishBtn?.addEventListener("click", async () => {
    try {
      await invoke("set_state", { key: "onboarding_completed", value: "true" });
      overlay.classList.add("hidden");
      addAuditLog("system", "CDUS initialization completed successfully.");
    } catch (err) {
      console.error("Failed to save onboarding completion state:", err);
      overlay.classList.add("hidden");
    }
  });
}

function showStep(stepNum: number) {
  const steps = document.querySelectorAll(".onboarding-step");
  steps.forEach((step, idx) => {
    if (idx + 1 === stepNum) {
      step.classList.add("active");
    } else {
      step.classList.remove("active");
    }
  });

  const dots = document.querySelectorAll(".progress-dot");
  dots.forEach((dot, idx) => {
    if (idx + 1 === stepNum) {
      dot.classList.add("active");
    } else {
      dot.classList.remove("active");
    }
  });
}

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

// --- Search & Feedback Elements ---
let searchOverlay: HTMLElement | null = null;
let globalSearchInput: HTMLInputElement | null = null;
let searchResultsList: HTMLElement | null = null;
let searchLoading: HTMLElement | null = null;
let searchEmpty: HTMLElement | null = null;
let searchError: HTMLElement | null = null;
let searchFeedbackBtn: HTMLElement | null = null;
let sidebarSearchBtn: HTMLElement | null = null;

let feedbackModal: HTMLElement | null = null;
let feedbackCancelBtn: HTMLElement | null = null;
let feedbackSubmitBtn: HTMLElement | null = null;
let feedbackText: HTMLTextAreaElement | null = null;
let feedbackAttachLogs: HTMLInputElement | null = null;
let feedbackStatus: HTMLElement | null = null;

// --- Helper Functions ---

let clipboardHistory: any[] = [];
let clipboardSearchQuery = "";
const visibleSensitiveIds = new Set<number>();

function filterAndRenderClipboard() {
  const listContainer = document.querySelector("#clipboard-list");
  const emptyState = document.querySelector("#clipboard-empty");
  if (!listContainer) return;

  const query = clipboardSearchQuery.toLowerCase().trim();
  const filtered = clipboardHistory.filter(item => 
    item.content.toLowerCase().includes(query) || 
    item.source.toLowerCase().includes(query)
  );

  if (filtered.length === 0) {
    listContainer.innerHTML = "";
    emptyState?.classList.remove("hidden");
    return;
  }

  emptyState?.classList.add("hidden");
  listContainer.innerHTML = "";

  filtered.forEach((item) => {
    const itemEl = document.createElement("div");
    itemEl.className = "clipboard-item";
    
    const isSensitive = item.is_sensitive;
    const isVisible = visibleSensitiveIds.has(item.id);
    
    const eyeIcon = isVisible ? "🙈" : "👁️";
    const sensitiveToggle = isSensitive 
      ? `<button class="action-btn toggle-sensitive-btn" title="${isVisible ? 'Hide password' : 'Show password'}">${eyeIcon}</button>` 
      : "";

    let displayHtml = "";
    let isRich = false;
    
    if (isSensitive && !isVisible) {
      displayHtml = `<div class="clipboard-content sensitive-text">••••••••••••</div>`;
    } else {
      try {
        const parsed = JSON.parse(item.content);
        if (parsed && parsed.type === "image") {
          isRich = true;
          displayHtml = `
            <div class="clipboard-content rich-content image-content">
              <img src="${parsed.data}" class="clipboard-image" alt="Clipboard Image" />
            </div>
          `;
        } else if (parsed && parsed.type === "url") {
          isRich = true;
          const faviconHtml = parsed.favicon 
            ? `<img src="${parsed.favicon}" class="favicon-icon" alt="" />`
            : `<span class="favicon-fallback">🌐</span>`;
          displayHtml = `
            <div class="clipboard-content rich-content url-content">
              <div class="url-metadata-container">
                ${faviconHtml}
                <div class="url-metadata-text">
                  <div class="url-title">${parsed.title}</div>
                  <a href="${parsed.url}" target="_blank" class="url-link" onclick="event.stopPropagation()">${parsed.url}</a>
                </div>
              </div>
            </div>
          `;
        }
      } catch (e) {
        // Not JSON or parsing failed, render as plain text
      }
      
      if (!isRich) {
        displayHtml = `<div class="clipboard-content">${item.content}</div>`;
      }
    }

    const localOnlyToggle = `
      <button class="action-btn toggle-local-btn ${item.local_only ? 'active' : ''}" title="${item.local_only ? 'Shared sync disabled' : 'Keep on this device only'}">
        ${item.local_only ? '🔒' : '🔓'}
      </button>
    `;

    itemEl.innerHTML = `
      ${displayHtml}
      <div class="clipboard-meta">
        <span class="device-badge">${item.source}</span>
        <span class="timestamp">${formatTimestamp(item.timestamp)}</span>
      </div>
      <div class="clipboard-actions">
        ${localOnlyToggle}
        ${sensitiveToggle}
        <button class="action-btn delete-btn" title="Delete from history">🗑️</button>
      </div>
      <div class="copy-feedback">Copied!</div>
    `;

    itemEl.addEventListener("click", (e) => {
      if ((e.target as Element).closest(".action-btn")) {
        return;
      }
      
      invoke("set_clipboard", { content: item.content }).then(() => {
        const feedback = itemEl.querySelector(".copy-feedback");
        feedback?.classList.add("show");
        setTimeout(() => feedback?.classList.remove("show"), 1500);
      }).catch((err) => {
        console.error("Failed to copy to clipboard:", err);
      });
    });

    const toggleLocalBtn = itemEl.querySelector(".toggle-local-btn");
    toggleLocalBtn?.addEventListener("click", async (e) => {
      e.stopPropagation();
      const newLocalOnly = !item.local_only;
      try {
        await invoke("toggle_local_only", { id: item.id, localOnly: newLocalOnly });
        item.local_only = newLocalOnly;
        filterAndRenderClipboard();
      } catch (err) {
        console.error("Failed to toggle local only status:", err);
      }
    });

    if (isSensitive) {
      const toggleBtn = itemEl.querySelector(".toggle-sensitive-btn");
      toggleBtn?.addEventListener("click", (e) => {
        e.stopPropagation();
        if (isVisible) {
          visibleSensitiveIds.delete(item.id);
        } else {
          visibleSensitiveIds.add(item.id);
        }
        filterAndRenderClipboard();
      });
    }

    const deleteBtn = itemEl.querySelector(".delete-btn");
    deleteBtn?.addEventListener("click", async (e) => {
      e.stopPropagation();
      if (confirm("Delete this item from history?")) {
        try {
          await invoke("delete_clipboard_item", { id: item.id });
          clipboardHistory = clipboardHistory.filter(i => i.id !== item.id);
          filterAndRenderClipboard();
        } catch (err) {
          console.error("Failed to delete item:", err);
        }
      }
    });

    listContainer.appendChild(itemEl);
  });
}

async function renderClipboard() {
  const loadingEl = document.querySelector("#clipboard-loading");
  const errorEl = document.querySelector("#clipboard-error");
  const listEl = document.querySelector("#clipboard-list");
  const emptyEl = document.querySelector("#clipboard-empty");

  loadingEl?.classList.remove("hidden");
  errorEl?.classList.add("hidden");
  listEl?.classList.add("hidden");
  emptyEl?.classList.add("hidden");

  try {
    const history: any[] = await invoke("get_clipboard_history", { limit: 50 });
    clipboardHistory = history;
    loadingEl?.classList.add("hidden");
    listEl?.classList.remove("hidden");
    filterAndRenderClipboard();
  } catch (err) {
    console.error("Failed to fetch history:", err);
    loadingEl?.classList.add("hidden");
    errorEl?.classList.remove("hidden");
    listEl?.classList.add("hidden");
    emptyEl?.classList.add("hidden");
  }
}

async function loadSettings() {
  const deviceNameInput = document.querySelector("#device-name") as HTMLInputElement;
  const syncEnabledInput = document.querySelector("#sync-enabled") as HTMLInputElement;
  const limitSlider = document.querySelector("#clipboard-limit") as HTMLInputElement;
  const limitValue = document.querySelector("#limit-value");

  const loadingEl = document.querySelector("#settings-loading");
  const errorEl = document.querySelector("#settings-error");
  const settingsContainer = document.querySelector(".settings-container");
  const saveBtn = document.querySelector("#save-settings-btn");

  loadingEl?.classList.remove("hidden");
  errorEl?.classList.add("hidden");
  settingsContainer?.classList.add("hidden");
  saveBtn?.classList.add("hidden");

  try {
    const deviceName: string | null = await invoke("get_state", { key: "device_name" });
    const syncEnabled: string | null = await invoke("get_state", { key: "sync_enabled" });
    const limit: string | null = await invoke("get_state", { key: "clipboard_limit" });
    const telemetryOptIn: boolean = await invoke("get_telemetry_opt_in");

    if (deviceName && deviceNameInput) deviceNameInput.value = deviceName;
    if (syncEnabled && syncEnabledInput) syncEnabledInput.checked = syncEnabled === "true";
    const telemetryOptInInput = document.querySelector("#telemetry-opt-in") as HTMLInputElement;
    if (telemetryOptInInput) telemetryOptInInput.checked = telemetryOptIn;
    if (limit && limitSlider) {
      limitSlider.value = limit;
      if (limitValue) limitValue.textContent = `${limit} items`;
    }

    loadingEl?.classList.add("hidden");
    settingsContainer?.classList.remove("hidden");
    saveBtn?.classList.remove("hidden");
  } catch (err) {
    console.error("Failed to load settings:", err);
    loadingEl?.classList.add("hidden");
    errorEl?.classList.remove("hidden");
    settingsContainer?.classList.add("hidden");
    saveBtn?.classList.add("hidden");
  }
}


async function loadFileHistory() {
  const loadingEl = document.querySelector("#files-loading");
  const errorEl = document.querySelector("#files-error");
  const listEl = document.querySelector("#files-list-container");

  loadingEl?.classList.remove("hidden");
  errorEl?.classList.add("hidden");
  listEl?.classList.add("hidden");

  try {
    const history: any[] = await invoke("get_file_transfer_history", { limit: 50 });
    transfers.clear();
    history.forEach((record) => {
      let status = record.status;
      if (status === "in_progress" || status === "paused" || status === "awaiting_acceptance") {
        status = "paused";
      } else if (status === "failed") {
        status = "error";
      } else if (status === "declined") {
        status = "rejected";
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
    loadingEl?.classList.add("hidden");
    listEl?.classList.remove("hidden");
    renderFiles();
  } catch (err) {
    console.error("Failed to load file history:", err);
    loadingEl?.classList.add("hidden");
    errorEl?.classList.remove("hidden");
    listEl?.classList.add("hidden");
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
        ${transfer.status === "error" ? `<div class="error-message">${sanitizeErrorMessage(transfer.error || "Unknown error")}</div>` : ""}
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
            ${transfer.status === "complete" ? `<button class="open-location-btn dropdown-option" data-id="${transfer.transferId}">Show in Folder</button>` : ""}
            <button class="dismiss-btn dropdown-option" data-id="${transfer.transferId}">Dismiss</button>
            <button class="delete-file-btn danger-option dropdown-option" data-id="${transfer.transferId}">Delete Permanently</button>
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
      if (transfer.status === "complete") {
        itemEl.querySelector(".open-location-btn")?.addEventListener("click", async (e) => {
          e.stopPropagation();
          dropdown?.classList.add("hidden");
          try {
            await invoke("open_file_location", { transferId: transfer.transferId });
          } catch (err) {
            console.error("Failed to open file location:", err);
            alert(err);
          }
        });
      }

      itemEl.querySelector(".dismiss-btn")?.addEventListener("click", async (e) => {
        e.stopPropagation();
        try {
          await invoke("delete_file_transfer", { transferId: transfer.transferId });
        } catch (err) {
          console.error("Failed to delete file transfer:", err);
        }
        transfers.delete(transfer.transferId);
        renderFiles();
      });

      itemEl.querySelector(".delete-file-btn")?.addEventListener("click", async (e) => {
        e.stopPropagation();
        if (confirm("Are you sure you want to permanently delete this file from your disk?")) {
          try {
            await invoke("delete_file_permanently", { transferId: transfer.transferId });
          } catch (err) {
            console.error("Failed to delete file permanently:", err);
          }
          transfers.delete(transfer.transferId);
          renderFiles();
        }
      });
    } else {
      itemEl.querySelector(".cancel-btn")?.addEventListener("click", async (e) => {
        e.stopPropagation();
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
  
  const loadingEl = document.querySelector("#devices-loading");
  const errorEl = document.querySelector("#devices-error");
  const emptyEl = document.querySelector("#devices-empty");

  loadingEl?.classList.remove("hidden");
  errorEl?.classList.add("hidden");
  pairedList.classList.add("hidden");
  emptyEl?.classList.add("hidden");

  try {
    const devices: [string, string, string | null][] = await invoke("get_paired_devices");
    pairedDeviceIds = devices.map(([id]) => id);
    
    // Update labels map
    deviceLabels.clear();
    devices.forEach(([id, label]) => deviceLabels.set(id, label));
    
    pairedList.innerHTML = "";
    loadingEl?.classList.add("hidden");
    
    if (devices.length === 0) {
      emptyEl?.classList.remove("hidden");
      return;
    }

    pairedList.classList.remove("hidden");
    devices.forEach(([id, name, transport]) => {
      const row = document.createElement("div");
      row.className = "device-row";
      
      // Get or initialize state
      let mockState = mockDeviceStates.get(id);
      if (transport !== null) {
        if (!mockState || mockState.status !== "online" || mockState.transport !== transport) {
          if (mockState?.timerId) {
            clearInterval(mockState.timerId);
            clearTimeout(mockState.timerId);
          }
          mockState = { status: "online", transport };
          mockDeviceStates.set(id, mockState);
        }
      } else {
        if (!mockState || mockState.status === "online") {
          if (mockState?.timerId) {
            clearTimeout(mockState.timerId);
          }
          mockState = { status: "offline", transport: null };
          mockDeviceStates.set(id, mockState);
        }
      }

      let statusClass = "offline";
      let statusText = "Offline";
      let pathHtml = "";
      let actionBtnHtml = "";

      if (mockState.status === "online") {
        statusClass = "online";
        statusText = "Online";
        const tType = mockState.transport || "Lan";
        const badgeClass = tType.toLowerCase();
        pathHtml = `<span class="connection-path ${badgeClass}">${tType}</span>`;
        actionBtnHtml = `
          <button class="primary-btn send-file-btn" data-id="${id}">Send File</button>
          <button class="secondary-btn disconnect-btn" data-id="${id}" style="margin-left: 8px;">Disconnect</button>
        `;
      } else if (mockState.status === "connecting") {
        statusClass = "connecting";
        statusText = "Connecting...";
        actionBtnHtml = `
          <button class="primary-btn reconnect-now-btn" data-id="${id}" disabled>Connecting...</button>
        `;
      } else {
        statusClass = "offline";
        statusText = "Offline";
        actionBtnHtml = `
          <button class="primary-btn reconnect-now-btn" data-id="${id}">Connect</button>
        `;
      }

      row.innerHTML = `
        <div class="device-info">
          <span class="device-name-label">${name}</span>
          <div class="device-status">
            <span class="status-dot ${statusClass}"></span>
            <span class="device-type-label">${statusText}</span>
            ${pathHtml}
          </div>
        </div>
        <div class="device-actions">
          ${actionBtnHtml}
          ${(mockState.status === "online" && isDeveloperMode) ? `<button class="tertiary-btn benchmark-btn" data-id="${id}" style="margin-right: 8px;">Benchmark</button>` : ""}
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

      row.querySelector(".reconnect-now-btn")?.addEventListener("click", async () => {
        triggerDeviceConnection(id);
        try {
          await invoke("pair_with", { nodeId: id });
        } catch (err) {
          console.error("Failed to initiate connection:", err);
          const s = mockDeviceStates.get(id);
          if (s) {
            if (s.timerId) clearTimeout(s.timerId);
            s.status = "offline";
            s.transport = null;
            renderPairedDevices();
          }
        }
      });

      row.querySelector(".disconnect-btn")?.addEventListener("click", async () => {
        const s = mockDeviceStates.get(id);
        if (s) {
          if (s.timerId) clearTimeout(s.timerId);
          s.status = "offline";
          s.transport = null;
          renderPairedDevices();
          addAuditLog("system", `Disconnected from device ${name}`);
          try {
            await invoke("disconnect_device", { nodeId: id });
          } catch (err) {
            console.error("Failed to disconnect peer:", err);
          }
        }
      });

      row.querySelector(".send-file-btn")?.addEventListener("click", () => {
        initiateFileSend(id);
      });

      pairedList?.appendChild(row);
    });
  } catch (err) {
    console.error("Failed to render paired devices:", err);
    loadingEl?.classList.add("hidden");
    errorEl?.classList.remove("hidden");
    pairedList.classList.add("hidden");
    emptyEl?.classList.add("hidden");
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

  // Initialize Search & Feedback references
  searchOverlay = document.querySelector("#search-overlay") as HTMLElement;
  globalSearchInput = document.querySelector("#global-search-input") as HTMLInputElement;
  searchResultsList = document.querySelector("#search-results-list") as HTMLElement;
  searchLoading = document.querySelector("#search-loading") as HTMLElement;
  searchEmpty = document.querySelector("#search-empty") as HTMLElement;
  searchError = document.querySelector("#search-error") as HTMLElement;
  searchFeedbackBtn = document.querySelector("#search-feedback-btn") as HTMLElement;
  sidebarSearchBtn = document.querySelector("#sidebar-search-btn") as HTMLElement;

  feedbackModal = document.querySelector("#feedback-modal") as HTMLElement;
  feedbackCancelBtn = document.querySelector("#feedback-cancel-btn") as HTMLElement;
  feedbackSubmitBtn = document.querySelector("#feedback-submit-btn") as HTMLElement;
  feedbackText = document.querySelector("#feedback-text") as HTMLTextAreaElement;
  feedbackAttachLogs = document.querySelector("#feedback-attach-logs") as HTMLInputElement;
  feedbackStatus = document.querySelector("#feedback-status") as HTMLElement;

  loadSettings();
  loadFileHistory();
  initOnboarding();

  document.querySelector("#clear-audit-btn")?.addEventListener("click", async () => {
    if (confirm("Are you sure you want to clear all audit logs?")) {
      try {
        await invoke("clear_audit_logs");
        renderAuditLogs();
      } catch (err) {
        console.error("Failed to clear audit logs:", err);
      }
    }
  });

  // Wire Retry buttons
  document.querySelector("#retry-devices-btn")?.addEventListener("click", () => renderPairedDevices());
  document.querySelector("#retry-clipboard-btn")?.addEventListener("click", () => renderClipboard());
  document.querySelector("#retry-files-btn")?.addEventListener("click", () => loadFileHistory());
  document.querySelector("#retry-settings-btn")?.addEventListener("click", () => loadSettings());
  document.querySelector("#retry-audit-btn")?.addEventListener("click", () => renderAuditLogs());

  const searchInput = document.querySelector("#clipboard-search") as HTMLInputElement;
  searchInput?.addEventListener("input", (e) => {
    clipboardSearchQuery = (e.target as HTMLInputElement).value;
    filterAndRenderClipboard();
  });

  document.querySelector("#clear-history-btn")?.addEventListener("click", async () => {
    if (confirm("Are you sure you want to clear all clipboard history?")) {
      try {
        await invoke("clear_clipboard_history");
        clipboardHistory = [];
        filterAndRenderClipboard();
      } catch (err) {
        console.error("Failed to clear history:", err);
      }
    }
  });

  document.querySelector("#clear-finished-btn")?.addEventListener("click", async () => {
    try {
      await invoke("clear_finished_transfers");
    } catch (err) {
      console.error("Failed to clear finished transfers:", err);
    }
    transfers.forEach((transfer, hash) => {
      if (transfer.status === "complete" || transfer.status === "error" || transfer.status === "rejected") {
        transfers.delete(hash);
      }
    });
    renderFiles();
  });

  document.querySelector("#clear-file-history-btn")?.addEventListener("click", async () => {
    if (confirm("Are you sure you want to clear all file transfer history?")) {
      try {
        await invoke("clear_finished_transfers");
        transfers.forEach((transfer, hash) => {
          if (transfer.status === "complete" || transfer.status === "error" || transfer.status === "rejected") {
            transfers.delete(hash);
          }
        });
        renderFiles();
      } catch (err) {
        console.error("Failed to clear file transfer history:", err);
      }
    }
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

  const navItems = document.querySelectorAll(".nav-item[data-view]");
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
      } else if (targetViewId === "notifications") {
        renderNotifications();
      } else if (targetViewId === "audit") {
        renderAuditLogs();
      }
    });
  });

  document.querySelector("#clear-notifications-btn")?.addEventListener("click", async () => {
    const keys = activeNotifications.map(n => n.key);
    for (const key of keys) {
      try {
        await invoke("dismiss_notification", { key });
      } catch (err) {
        console.error("Failed to dismiss notification:", key, err);
      }
    }
    activeNotifications = [];
    renderNotifications();
  });

  document.querySelector("#retry-notifications-btn")?.addEventListener("click", () => {
    notificationsLoaded = false;
    renderNotifications();
  });

  const activeView = document.querySelector(".view.active");
  if (activeView?.id === "view-clipboard") {
    renderClipboard();
  } else if (activeView?.id === "view-devices") {
    renderPairedDevices();
  } else if (activeView?.id === "view-files") {
    renderFiles();
  } else if (activeView?.id === "view-notifications") {
    renderNotifications();
  } else if (activeView?.id === "view-audit") {
    renderAuditLogs();
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
    const telemetryOptIn = (document.querySelector("#telemetry-opt-in") as HTMLInputElement).checked;
    const limit = limitSlider.value;
    
    try {
      await invoke("set_state", { key: "device_name", value: deviceName });
      await invoke("set_state", { key: "sync_enabled", value: syncEnabled.toString() });
      await invoke("set_telemetry_opt_in", { optIn: telemetryOptIn });
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
        if (active && pin && !silent) {
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
    
    showErrorModal(
      "File Transfer Failed",
      "We couldn't complete the file transfer. Please make sure the other device is online, has CDUS open, and has enough storage space.",
      error || "Unknown transfer error",
      transferId ? () => {
        invoke("resume_file_transfer", { transferId }).catch(err => {
          console.error("Failed to resume transfer:", err);
        });
      } : undefined,
      "https://github.com/rohanakode490/cdus/blob/main/docs/troubleshooting.md"
    );
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
    const [success, nodeId, label, error] = event.payload;
    if (success) {
      console.log(`Pairing successful with ${label}`);
      renderPairedDevices().then(() => {
        updateDiscoveryList();
      });
      addAuditLog("pairing", `Successfully paired with device '${label}'`);
    } else {
      console.error(`Pairing failed with ${label}: ${error}`);
      let userFriendlyMsg = "The pairing request failed. Please check the following:\n1. Make sure both devices are on the same network.\n2. Ensure the PIN code was entered correctly.\n3. Make sure CDUS is open and not blocked by a firewall.";
      if (error && error.includes("timed out")) {
        userFriendlyMsg = "Pairing timed out. The other device took too long to respond. Please make sure the app is running on both devices and try again.";
      } else if (error && error.includes("rejected")) {
        userFriendlyMsg = "Pairing was rejected by the remote device.";
      }
      
      showErrorModal(
        "Pairing Failed",
        userFriendlyMsg,
        error || "Unknown pairing error",
        nodeId ? () => {
          invoke("pair_with", { nodeId }).catch(err => {
            console.error("Failed to retry pairing:", err);
          });
        } : undefined,
        "https://github.com/rohanakode490/cdus/blob/main/docs/troubleshooting.md"
      );
    }
  });

  listen("relay-status", (event: any) => {
    const [connected, _error] = event.payload;
    const relayIndicator = document.querySelector("#relay-status-indicator");
    if (relayIndicator) {
      if (connected) {
        relayIndicator.classList.remove("offline");
        relayIndicator.classList.add("online");
        relayIndicator.textContent = "Relay Connected";
      } else {
        relayIndicator.classList.remove("online");
        relayIndicator.classList.add("offline");
        relayIndicator.textContent = "Relay Offline";
      }
    }
  });

  listen("peer-connected", (_event: any) => {
    renderPairedDevices();
  });

  listen("notification-mirrored", (event: any) => {
    console.log("UI: Received notification-mirrored", event.payload);
    const payload = event.payload;
    activeNotifications = activeNotifications.filter(n => n.key !== payload.key);
    activeNotifications.push(payload);
    const notificationsView = document.querySelector("#view-notifications");
    if (notificationsView?.classList.contains("active")) {
      renderNotifications();
    }
  });

  listen("notification-dismissed", (event: any) => {
    console.log("UI: Received notification-dismissed", event.payload);
    const key = event.payload;
    activeNotifications = activeNotifications.filter(n => n.key !== key);
    const notificationsView = document.querySelector("#view-notifications");
    if (notificationsView?.classList.contains("active")) {
      renderNotifications();
    }
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


  // Check for updates on startup
  checkForUpdates();

  document.querySelector("#check-update-btn")?.addEventListener("click", async () => {
    const statusEl = document.querySelector("#update-status");
    if (statusEl) statusEl.textContent = "Checking for updates...";
    try {
      const update = await check();
      if (update) {
        if (statusEl) statusEl.textContent = `Update available: version ${update.version}`;
        if (confirm(`A new version ${update.version} is available. Would you like to install it now and restart?`)) {
          if (statusEl) statusEl.textContent = "Downloading and installing...";
          await update.downloadAndInstall();
          await relaunch();
        }
      } else {
        if (statusEl) statusEl.textContent = "Application is up to date.";
      }
    } catch (err) {
      console.error("Failed to check for updates:", err);
      if (statusEl) statusEl.textContent = "Failed to check for updates.";
    }
  });

  // --- Global Spotlight Search ---
  function openSearch() {
    searchOverlay?.classList.remove("hidden");
    globalSearchInput?.focus();
    if (globalSearchInput) {
      globalSearchInput.value = "";
    }
    performSearch("");
  }

  function closeSearch() {
    searchOverlay?.classList.add("hidden");
  }

  sidebarSearchBtn?.addEventListener("click", (e) => {
    e.stopPropagation();
    openSearch();
  });

  // Global Keyboard Shortcuts
  document.addEventListener("keydown", (e) => {
    // Ctrl+S to open search
    if (e.ctrlKey && e.key.toLowerCase() === "s") {
      e.preventDefault();
      if (searchOverlay?.classList.contains("hidden")) {
        openSearch();
      } else {
        closeSearch();
      }
    }

    // Escape to close active search or feedback
    if (e.key === "Escape") {
      if (searchOverlay && !searchOverlay.classList.contains("hidden")) {
        closeSearch();
      }
      if (feedbackModal && !feedbackModal.classList.contains("hidden")) {
        closeFeedback();
      }
    }
  });

  // Close search when clicking backdrop
  searchOverlay?.addEventListener("click", (e) => {
    if (e.target === searchOverlay) {
      closeSearch();
    }
  });

  // Input event to search
  globalSearchInput?.addEventListener("input", (e) => {
    const query = (e.target as HTMLInputElement).value;
    performSearch(query);
  });

  let searchTimeout: any = null;
  let activeSearchResults: any[] = [];

  function renderSearchResults(results: any[]) {
    const mappedItems = results.map(item => ({
      id: item.id,
      type: item.item_type,
      text: item.title,
      meta: item.subtitle,
      hint: item.item_type === "clipboard" ? "Press Enter to Copy" : (item.item_type === "file" ? "Press Enter to Show in Folder" : "Press Enter to Action")
    }));

    activeSearchResults = mappedItems;

    if (mappedItems.length === 0) {
      searchLoading?.classList.add("hidden");
      searchResultsList?.classList.add("hidden");
      searchEmpty?.classList.remove("hidden");
      return;
    }

    searchEmpty?.classList.add("hidden");
    searchResultsList?.classList.remove("hidden");
    searchResultsList!.innerHTML = "";

    const groups: Record<string, typeof mappedItems> = {
      clipboard: [],
      file: [],
      device: []
    };

    mappedItems.forEach(item => {
      if (groups[item.type]) {
        groups[item.type].push(item);
      }
    });

    const typeLabels: Record<string, string> = {
      clipboard: "Clipboard History",
      file: "Files",
      device: "Devices"
    };

    Object.entries(groups).forEach(([type, items]) => {
      if (items.length === 0) return;

      const groupDiv = document.createElement("div");
      groupDiv.className = "results-group";
      
      const titleDiv = document.createElement("div");
      titleDiv.className = "group-title";
      titleDiv.textContent = typeLabels[type];
      groupDiv.appendChild(titleDiv);

      items.forEach(item => {
        const itemDiv = document.createElement("div");
        itemDiv.className = "result-item";
        itemDiv.setAttribute("data-id", item.id);
        itemDiv.setAttribute("data-type", item.type);
        itemDiv.setAttribute("tabindex", "0");

        const icon = item.type === "clipboard" ? "📋" : (item.type === "file" ? (item.text.endsWith(".png") ? "🖼️" : "📄") : "💻");
        
        itemDiv.innerHTML = `
          <div class="result-icon">${icon}</div>
          <div class="result-main">
            <div class="result-text">${escapeHtml(item.text)}</div>
            <div class="result-meta">${item.meta}</div>
          </div>
          <div class="result-action-hint">${item.hint}</div>
        `;

        itemDiv.addEventListener("click", () => {
          triggerResultAction(item);
        });

        groupDiv.appendChild(itemDiv);
      });

      searchResultsList?.appendChild(groupDiv);
    });

    const firstItem = searchResultsList?.querySelector(".result-item");
    if (firstItem) {
      firstItem.classList.add("selected");
    }
  }

  function performSearch(query: string) {
    if (searchTimeout) clearTimeout(searchTimeout);
    
    searchEmpty?.classList.add("hidden");
    searchError?.classList.add("hidden");

    if (query.trim() === "") {
      // Execute instantly for empty query (recent items) to avoid loading flash
      searchLoading?.classList.add("hidden");
      invoke("search", { query: "" })
        .then((results: any) => {
          renderSearchResults(results);
        })
        .catch(err => {
          console.error("Initial search failed:", err);
          searchError?.classList.remove("hidden");
        });
    } else {
      searchLoading?.classList.remove("hidden");
      searchResultsList?.classList.add("hidden");

      searchTimeout = setTimeout(async () => {
        try {
          const results: any[] = await invoke("search", { query });
          searchLoading?.classList.add("hidden");
          renderSearchResults(results);
        } catch (err) {
          console.error("Search failed:", err);
          searchLoading?.classList.add("hidden");
          searchError?.classList.remove("hidden");
        }
      }, 150);
    }
  }

  function escapeHtml(text: string): string {
    const div = document.createElement("div");
    div.textContent = text;
    return div.innerHTML;
  }

  async function triggerResultAction(item: any) {
    if (item.type === "clipboard") {
      try {
        await invoke("set_clipboard", { content: item.text });
        addAuditLog("sync", `Copied text from search overlay to clipboard: ${item.text.substring(0, 30)}...`);
        closeSearch();
      } catch (err) {
        console.error("Failed to copy search item to clipboard:", err);
      }
    } else if (item.type === "file") {
      try {
        await invoke("open_file_location", { transferId: item.id });
        addAuditLog("system", `Opened file location from search overlay: ${item.text}`);
      } catch (err) {
        console.error("Failed to open file location:", err);
        alert(err);
      }
      closeSearch();
    } else if (item.type === "device") {
      const devicesNav = document.querySelector('.nav-item[data-view="devices"]') as HTMLElement;
      devicesNav?.click();
      closeSearch();
    }
  }

  // Keyboard navigation inside search overlay
  document.addEventListener("keydown", (e) => {
    if (searchOverlay?.classList.contains("hidden")) return;

    const items = Array.from(searchResultsList?.querySelectorAll(".result-item") || []) as HTMLElement[];
    if (items.length === 0) return;

    let selectedIndex = items.findIndex(item => item.classList.contains("selected"));

    if (e.key === "ArrowDown") {
      e.preventDefault();
      if (selectedIndex !== -1) {
        items[selectedIndex].classList.remove("selected");
        selectedIndex = (selectedIndex + 1) % items.length;
      } else {
        selectedIndex = 0;
      }
      items[selectedIndex].classList.add("selected");
      items[selectedIndex].scrollIntoView({ block: "nearest" });
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      if (selectedIndex !== -1) {
        items[selectedIndex].classList.remove("selected");
        selectedIndex = (selectedIndex - 1 + items.length) % items.length;
      } else {
        selectedIndex = items.length - 1;
      }
      items[selectedIndex].classList.add("selected");
      items[selectedIndex].scrollIntoView({ block: "nearest" });
    } else if (e.key === "Enter") {
      if (selectedIndex !== -1) {
        e.preventDefault();
        const selectedId = items[selectedIndex].getAttribute("data-id");
        const item = activeSearchResults.find(i => i.id === selectedId);
        if (item) {
          triggerResultAction(item);
        }
      }
    }
  });

  // --- Send Feedback Modal ---
  function openFeedback() {
    closeSearch();
    feedbackModal?.classList.remove("hidden");
    if (feedbackText) {
      feedbackText.value = "";
    }
    if (feedbackStatus) {
      feedbackStatus.classList.add("hidden");
      feedbackStatus.textContent = "";
    }
    feedbackText?.focus();
  }

  function closeFeedback() {
    feedbackModal?.classList.add("hidden");
  }

  searchFeedbackBtn?.addEventListener("click", (e) => {
    e.stopPropagation();
    openFeedback();
  });

  feedbackCancelBtn?.addEventListener("click", () => {
    closeFeedback();
  });

  feedbackModal?.addEventListener("click", (e) => {
    if (e.target === feedbackModal) {
      closeFeedback();
    }
  });

  feedbackSubmitBtn?.addEventListener("click", async () => {
    const text = feedbackText?.value.trim() || "";
    if (text.length < 5) {
      if (feedbackStatus) {
        feedbackStatus.className = "setting-hint feedback-status-error";
        feedbackStatus.textContent = "Please enter at least 5 characters.";
        feedbackStatus.classList.remove("hidden");
      }
      return;
    }

    if (feedbackStatus) {
      feedbackStatus.className = "setting-hint";
      feedbackStatus.textContent = "Submitting feedback...";
      feedbackStatus.classList.remove("hidden");
    }

    setTimeout(async () => {
      try {
        const attachLogs = feedbackAttachLogs?.checked || false;
        await invoke("submit_feedback", { text, attachLogs });
        
        if (feedbackStatus) {
          feedbackStatus.className = "setting-hint feedback-status-success";
          feedbackStatus.textContent = "Thank you! Your feedback has been submitted successfully.";
        }
        
        setTimeout(() => {
          closeFeedback();
        }, 1500);
      } catch (err) {
        console.error("Failed to submit feedback:", err);
        if (feedbackStatus) {
          feedbackStatus.className = "setting-hint feedback-status-error";
          feedbackStatus.textContent = "Failed to submit feedback. Please try again.";
        }
      }
    }, 1000);
  });
});

async function checkForUpdates() {
  try {
    console.log("Checking for updates...");
    const update = await check();
    if (update) {
      console.log(`Update found: version ${update.version} (current: ${update.currentVersion})`);
      if (confirm(`A new version ${update.version} is available. Would you like to install it now and restart?`)) {
        console.log("Downloading and installing update...");
        await update.downloadAndInstall();
        console.log("Update installed, relaunching application...");
        await relaunch();
      }
    } else {
      console.log("No updates found.");
    }
  } catch (err) {
    console.error("Failed to check for updates:", err);
  }
}

async function renderNotifications() {
  const container = document.querySelector("#notifications-list-container");
  const listEl = document.querySelector("#notifications-list");
  const emptyEl = document.querySelector("#notifications-empty");
  const loadingEl = document.querySelector("#notifications-loading");
  const errorEl = document.querySelector("#notifications-error");

  if (!container || !listEl || !emptyEl || !loadingEl || !errorEl) return;

  // Show loading state if not loaded yet
  if (!notificationsLoaded) {
    loadingEl.classList.remove("hidden");
    listEl.innerHTML = "";
    emptyEl.classList.add("hidden");
    errorEl.classList.add("hidden");
    
    try {
      const active: any = await invoke("get_active_notifications");
      activeNotifications = active;
      notificationsLoaded = true;
    } catch (err) {
      console.error("Failed to load active notifications:", err);
      loadingEl.classList.add("hidden");
      errorEl.classList.remove("hidden");
      return;
    }
  }

  loadingEl.classList.add("hidden");
  errorEl.classList.add("hidden");
  listEl.innerHTML = "";

  if (activeNotifications.length === 0) {
    emptyEl.classList.remove("hidden");
    return;
  }

  emptyEl.classList.add("hidden");

  // Sort by timestamp descending
  const sorted = [...activeNotifications].sort((a, b) => b.timestamp - a.timestamp);

  sorted.forEach((notif) => {
    const cardEl = document.createElement("div");
    cardEl.className = "notification-card";
    
    const formattedTime = new Date(notif.timestamp).toLocaleTimeString(undefined, {
      hour: "2-digit",
      minute: "2-digit",
    });

    const headerEl = document.createElement("div");
    headerEl.className = "notification-card-header";
    
    const appEl = document.createElement("span");
    appEl.className = "notification-app-name";
    appEl.textContent = notif.app_name || notif.package_name;
    
    const timeEl = document.createElement("span");
    timeEl.className = "notification-time";
    timeEl.textContent = formattedTime;
    
    headerEl.appendChild(appEl);
    headerEl.appendChild(timeEl);

    const titleEl = document.createElement("div");
    titleEl.className = "notification-title";
    titleEl.textContent = notif.title || "";

    const textEl = document.createElement("div");
    textEl.className = "notification-text";
    textEl.textContent = notif.text || "";

    const actionsEl = document.createElement("div");
    actionsEl.className = "notification-actions";

    const dismissBtn = document.createElement("button");
    dismissBtn.className = "dismiss-btn";
    dismissBtn.textContent = "Dismiss";
    dismissBtn.setAttribute("data-key", notif.key);
    dismissBtn.addEventListener("click", async (e) => {
      e.stopPropagation();
      try {
        await invoke("dismiss_notification", { key: notif.key });
        activeNotifications = activeNotifications.filter(n => n.key !== notif.key);
        renderNotifications();
      } catch (err) {
        console.error("Failed to dismiss notification:", err);
      }
    });

    actionsEl.appendChild(dismissBtn);

    cardEl.appendChild(headerEl);
    cardEl.appendChild(titleEl);
    cardEl.appendChild(textEl);
    cardEl.appendChild(actionsEl);

    listEl.appendChild(cardEl);
  });
}

