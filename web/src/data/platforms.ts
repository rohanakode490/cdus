export interface PlatformAction {
  label: string;
  href: string;
  variant: 'primary' | 'secondary';
}

export interface PlatformInfo {
  id: string;
  name: string;
  icon: string;
  osSpec: string;
  summary: string;
  description: string;
  actions: PlatformAction[];
  installSnippet: string;
}

export const platforms: PlatformInfo[] = [
  {
    id: 'linux',
    name: 'Linux',
    icon: 'simple-icons:linux',
    osSpec: 'Ubuntu, Debian, Fedora, Arch',
    summary: '.deb, .AppImage, AUR, systemd service',
    description: 'Native desktop shell bundled with the systemd background sync daemon.',
    actions: [
      { label: 'Download .deb', href: 'https://github.com/rohanakode490/cdus/releases/latest', variant: 'primary' },
      { label: 'Download .AppImage', href: 'https://github.com/rohanakode490/cdus/releases/latest', variant: 'secondary' },
    ],
    installSnippet: 'sudo dpkg -i cdus-desktop.deb',
  },
  {
    id: 'windows',
    name: 'Windows',
    icon: 'simple-icons:windows',
    osSpec: 'Windows 10 / 11 (64-bit)',
    summary: 'Windows 10/11 64-bit, NSIS & MSI',
    description: 'Integrated Windows service with native system tray and Credential Manager.',
    actions: [
      { label: 'Download .exe', href: 'https://github.com/rohanakode490/cdus/releases/latest', variant: 'primary' },
      { label: 'Download .msi', href: 'https://github.com/rohanakode490/cdus/releases/latest', variant: 'secondary' },
    ],
    installSnippet: 'winget install cdus.desktop',
  },
  {
    id: 'macos',
    name: 'macOS',
    icon: 'simple-icons:apple',
    osSpec: 'macOS 12+ (Apple Silicon & Intel)',
    summary: 'Universal binary for Apple Silicon & Intel',
    description: 'Universal macOS app with native menu bar widget, Keychain, and Notifications.',
    actions: [
      { label: 'Download .dmg', href: 'https://github.com/rohanakode490/cdus/releases/latest', variant: 'primary' },
      { label: 'Download .tar.gz', href: 'https://github.com/rohanakode490/cdus/releases/latest', variant: 'secondary' },
    ],
    installSnippet: 'brew install --cask cdus',
  },
  {
    id: 'android',
    name: 'Android',
    icon: 'simple-icons:android',
    osSpec: 'Android 10+ (Phones & Tablets)',
    summary: 'Android 10+, Jetpack Compose, KeyStore',
    description: 'Native Android app with hardware KeyStore backing and QR code pairing.',
    actions: [
      { label: 'Download .apk', href: 'https://github.com/rohanakode490/cdus/releases/latest', variant: 'primary' },
      { label: 'GitHub Releases', href: 'https://github.com/rohanakode490/cdus/releases/latest', variant: 'secondary' },
    ],
    installSnippet: 'adb install -r cdus-android.apk',
  },
];
