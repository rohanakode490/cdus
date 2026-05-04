import { invoke } from "@tauri-apps/api/core";

interface CdusClipboardItem {
  id: number;
  content: string;
  device: string;
  timestamp: string;
}

const MOCK_CLIPBOARD_ITEMS: CdusClipboardItem[] = [
  { id: 1, content: "https://github.com/tauri-apps/tauri", device: "Rahul's Laptop", timestamp: "2 mins ago" },
  { id: 2, content: "npm install @tauri-apps/api", device: "Rahul's Laptop", timestamp: "15 mins ago" },
  { id: 3, content: "Meeting password: sync-is-cool-2026", device: "Pixel 8", timestamp: "1 hour ago" },
  { id: 4, content: "Dinner recipe: https://tasty.co/recipe/classic-lasagna", device: "Rahul's Laptop", timestamp: "3 hours ago" },
  { id: 5, content: "Pick up milk and eggs", device: "Pixel 8", timestamp: "5 hours ago" },
  { id: 6, content: "cd ~/projects/cdus && bun run dev", device: "Rahul's Laptop", timestamp: "Yesterday" },
  { id: 7, content: "Design specs for clipboard sync", device: "iPad Air", timestamp: "Yesterday" },
  { id: 8, content: "0x71C444e249E067AD74A02A4f1458C96538466eE6", device: "Rahul's Laptop", timestamp: "2 days ago" },
  { id: 9, content: "Remember to call mom", device: "Pixel 8", timestamp: "3 days ago" },
  { id: 10, content: "Family trip photo.jpg", device: "iPad Air", timestamp: "4 days ago" },
];

function renderClipboard() {
  const listContainer = document.querySelector("#clipboard-list");
  if (!listContainer) return;

  listContainer.innerHTML = "";
  
  MOCK_CLIPBOARD_ITEMS.forEach((item) => {
    const itemEl = document.createElement("div");
    itemEl.className = "clipboard-item";
    itemEl.innerHTML = `
      <div class="clipboard-content">${item.content}</div>
      <div class="clipboard-meta">
        <span class="device-badge">${item.device}</span>
        <span class="timestamp">${item.timestamp}</span>
      </div>
      <div class="copy-feedback">Copied!</div>
    `;

    itemEl.addEventListener("click", () => {
      // Simulate copy
      navigator.clipboard.writeText(item.content).then(() => {
        const feedback = itemEl.querySelector(".copy-feedback");
        feedback?.classList.add("show");
        setTimeout(() => feedback?.classList.remove("show"), 1500);
      });
    });

    listContainer.appendChild(itemEl);
  });
}

window.addEventListener("DOMContentLoaded", () => {
  const navItems = document.querySelectorAll(".nav-item");
  const views = document.querySelectorAll(".view");

  navItems.forEach((item) => {
    item.addEventListener("click", () => {
      const targetViewId = item.getAttribute("data-view");

      // Update active nav item
      navItems.forEach((nav) => nav.classList.remove("active"));
      item.classList.add("active");

      // Update active view
      views.forEach((view) => {
        view.classList.remove("active");
        if (view.id === `view-${targetViewId}`) {
          view.classList.add("active");
        }
      });

      // Special handling for clipboard view
      if (targetViewId === "clipboard") {
        renderClipboard();
      }
    });
  });

  // Initial render if default view is clipboard (it's devices by default)
  const activeView = document.querySelector(".view.active");
  if (activeView?.id === "view-clipboard") {
    renderClipboard();
  }

  // Add Device button handler
  document.querySelector("#add-device-btn")?.addEventListener("click", () => {
    alert("Discovery mode started! Scanning for nearby devices...");
  });

  // Settings: Clipboard limit slider logic
  const limitSlider = document.querySelector("#clipboard-limit") as HTMLInputElement;
  const limitValue = document.querySelector("#limit-value");
  
  limitSlider?.addEventListener("input", (e) => {
    const val = (e.target as HTMLInputElement).value;
    if (limitValue) limitValue.textContent = `${val} items`;
  });

  // Settings: Save button logic
  document.querySelector("#save-settings-btn")?.addEventListener("click", () => {
    const deviceName = (document.querySelector("#device-name") as HTMLInputElement).value;
    const syncEnabled = (document.querySelector("#sync-enabled") as HTMLInputElement).checked;
    const limit = limitSlider.value;
    
    console.log("Saving settings:", { deviceName, syncEnabled, limit });
    alert("Settings saved locally!");
  });

  // Files: Send File button handler
  document.querySelector("#send-file-btn")?.addEventListener("click", () => {
    alert("File picker opened! (Mock)");
  });

  // Diagnostics: Ping Agent button handler
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
});
