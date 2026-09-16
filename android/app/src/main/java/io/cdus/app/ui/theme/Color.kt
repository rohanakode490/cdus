package io.cdus.app.ui.theme

import androidx.compose.ui.graphics.Color

// Canonical CDUS Cross-Platform Palette (Synced with Desktop Theme)

// Brand Cyan / Teal (Desktop: #24C8DB)
val CdusCyan = Color(0xFF24C8DB)              // Primary accent & brand signature
val CdusCyanHover = Color(0xFF1EB4C7)         // Pressed / Hover state
val CdusCyanLight = Color(0xFF4DD0E1)         // High-contrast cyan on dark surfaces
val CdusCyanDark = Color(0xFF00838F)          // Deep cyan text / containers on light surfaces
val CdusCyanContainerLight = Color(0xFFE0F7FA)// Light mode cyan container tint
val CdusCyanContainerDark = Color(0xFF00363A) // Dark mode deep cyan container tint

// Dark Theme (Desktop: background #1A1A1A, surface #262626, border #333333)
val CdusDarkBase = Color(0xFF1A1A1A)          // Desktop root dark background
val CdusDarkSurface = Color(0xFF262626)       // Desktop sidebar, card, and modal surface
val CdusDarkSurfaceVariant = Color(0xFF2D2D2D)// Desktop elevated surface / hover
val CdusDarkBorder = Color(0xFF333333)        // Desktop card and divider border
val CdusDarkBorderSubtle = Color(0xFF444444)  // Desktop secondary border

// Light Theme (Desktop: background #FFFFFF, surface #F6F6F6, border #E0E0E0)
val CdusLightBase = Color(0xFFFFFFFF)         // Desktop root light background
val CdusLightSurface = Color(0xFFF6F6F6)      // Desktop sidebar and neutral container
val CdusLightCard = Color(0xFFFFFFFF)         // Desktop white card surface
val CdusLightSurfaceVariant = Color(0xFFEDEDED) // Desktop hover / secondary container
val CdusLightBorder = Color(0xFFE0E0E0)       // Desktop border
val CdusLightBorderSubtle = Color(0xFFEEEEEE) // Desktop subtle divider

// Text Hierarchies
val CdusTextPrimaryDark = Color(0xFFF6F6F6)   // Desktop dark text
val CdusTextSecondaryDark = Color(0xFF999999) // Desktop dark muted text
val CdusTextMutedDark = Color(0xFF888888)     // Desktop dark caption

val CdusTextPrimaryLight = Color(0xFF0F0F0F)  // Desktop light text
val CdusTextSecondaryLight = Color(0xFF666666)// Desktop light muted text
val CdusTextMutedLight = Color(0xFF888888)    // Desktop light caption

// Network & Status Semantics (Desktop status badges, dots, and indicators)
// Direct LAN / Online (Green)
val CdusStatusOnline = Color(0xFF4CAF50)
val CdusLanBgLight = Color(0xFFE8F5E9)
val CdusLanTextLight = Color(0xFF2E7D32)
val CdusLanBgDark = Color(0xFF1B5E20)
val CdusLanTextDark = Color(0xFFA5D6A7)

// Relay Fallback / Warning / PEX (Amber / Orange)
val CdusStatusRelay = Color(0xFFEF6C00)
val CdusRelayBgLight = Color(0xFFFFF3E0)
val CdusRelayTextLight = Color(0xFFEF6C00)
val CdusRelayBgDark = Color(0xFFE65100)
val CdusRelayTextDark = Color(0xFFFFCC80)

// Offline / Error / Destructive (Red / Crimson)
val CdusStatusOffline = Color(0xFFEF4444)
val CdusErrorText = Color(0xFFC62828)
val CdusErrorBgLight = Color(0xFFFFEBEE)
val CdusErrorBgDark = Color(0xFFB71C1C)
val CdusErrorTextDark = Color(0xFFEF9A9A)

// Connecting / Reconnecting
val CdusConnectingBlue = Color(0xFF2196F3)
val CdusConnectingGray = Color(0xFF9E9E9E)