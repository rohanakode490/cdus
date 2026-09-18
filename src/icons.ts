/**
 * CDUS Reusable SVG Icons Library
 *
 * Vector icons adhering strictly to the Canonical Desktop Theme and Frontend House Style.
 * Each icon is stroke-based (2px stroke), viewBox 0 0 24 24, with accessible defaults.
 */

export type IconName =
  | "clipboard"
  | "folder"
  | "shield"
  | "search"
  | "lightbulb"
  | "warning"
  | "globe"
  | "image"
  | "document"
  | "device";

export interface IconOptions {
  size?: number;
  className?: string;
  strokeWidth?: number;
  color?: string;
}

export const SVG_CLIPBOARD = `<svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"></path><rect x="8" y="2" width="8" height="4" rx="1" ry="1"></rect><path d="M9 12h6"></path><path d="M9 16h6"></path></svg>`;

export const SVG_FOLDER = `<svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"></path><path d="M12 11v6"></path><path d="m9 14 3 3 3-3"></path></svg>`;

export const SVG_SHIELD = `<svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"></path><circle cx="12" cy="11" r="1.5"></circle><path d="M12 12.5V15"></path></svg>`;

export const SVG_SEARCH = `<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="11" cy="11" r="8"></circle><line x1="21" y1="21" x2="16.65" y2="16.65"></line></svg>`;

export const SVG_LIGHTBULB = `<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 18h6"></path><path d="M10 22h4"></path><path d="M15.09 14c.18-.98.65-1.74 1.41-2.5A4.65 4.65 0 0 0 18 8 6 6 0 0 0 6 8c0 1 .23 2.23 1.5 3.5A4.61 4.61 0 0 1 8.91 14"></path></svg>`;

export const SVG_WARNING = `<svg viewBox="0 0 24 24" width="28" height="28" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3Z"></path><line x1="12" y1="9" x2="12" y2="13"></line><line x1="12" y1="17" x2="12.01" y2="17"></line></svg>`;

export const SVG_GLOBE = `<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="10"></circle><line x1="2" y1="12" x2="22" y2="12"></line><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"></path></svg>`;

export const SVG_IMAGE = `<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="3" width="18" height="18" rx="2" ry="2"></rect><circle cx="8.5" cy="8.5" r="1.5"></circle><polyline points="21 15 16 10 5 21"></polyline></svg>`;

export const SVG_DOCUMENT = `<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"></path><polyline points="14 2 14 8 20 8"></polyline><line x1="16" y1="13" x2="8" y2="13"></line><line x1="16" y1="17" x2="8" y2="17"></line></svg>`;

export const SVG_DEVICE = `<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="2" y="3" width="20" height="14" rx="2" ry="2"></rect><line x1="8" y1="21" x2="16" y2="21"></line><line x1="12" y1="17" x2="12" y2="21"></line></svg>`;

export const ICONS: Record<IconName, string> = {
  clipboard: SVG_CLIPBOARD,
  folder: SVG_FOLDER,
  shield: SVG_SHIELD,
  search: SVG_SEARCH,
  lightbulb: SVG_LIGHTBULB,
  warning: SVG_WARNING,
  globe: SVG_GLOBE,
  image: SVG_IMAGE,
  document: SVG_DOCUMENT,
  device: SVG_DEVICE,
};

/**
 * Get an SVG string for a given icon name, with optional size, class, and stroke adjustments.
 */
export function getIcon(name: IconName, options?: IconOptions): string {
  let svg = ICONS[name];
  if (!svg) return "";

  if (options?.size !== undefined) {
    svg = svg
      .replace(/width="\d+"/, `width="${options.size}"`)
      .replace(/height="\d+"/, `height="${options.size}"`);
  }

  if (options?.strokeWidth !== undefined) {
    svg = svg.replace(/stroke-width="\d+"/, `stroke-width="${options.strokeWidth}"`);
  }

  if (options?.className) {
    svg = svg.replace("<svg ", `<svg class="${options.className}" `);
  }

  return svg;
}

/**
 * Scan a DOM container and populate elements with `data-icon="<name>"` automatically.
 */
export function renderIcons(root: ParentNode = document): void {
  const elements = root.querySelectorAll<HTMLElement>("[data-icon]");
  elements.forEach((el) => {
    const iconName = el.getAttribute("data-icon") as IconName;
    if (iconName && ICONS[iconName]) {
      const sizeAttr = el.getAttribute("data-icon-size");
      const size = sizeAttr ? parseInt(sizeAttr, 10) : undefined;
      const classAttr = el.getAttribute("data-icon-class") || undefined;
      el.innerHTML = getIcon(iconName, { size, className: classAttr });
    }
  });
}
