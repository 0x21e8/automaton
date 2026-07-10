import { uiTokens } from "@ic-automaton/ui";

const lab = uiTokens.themes.lab;

export const themeTokens = {
  colors: {
    background: lab.background,
    panel: lab.panel,
    panelStrong: lab.panelStrong,
    ink: lab.ink,
    line: lab.line,
    accent: lab.accent,
    accentSoft: lab.accentSoft,
    muted: lab.muted,
    mutedStrong: lab.mutedStrong,
    inverse: lab.inverse,
    drawerBackground: lab.inverse,
    drawerText: "#cccccc",
    drawerMuted: "#888888",
    drawerSubtle: "#555555",
    tierNormal: uiTokens.status.success,
    tierLow: uiTokens.status.warning,
    tierCritical: uiTokens.status.critical,
    gridNormal: lab.ink,
    gridLow: uiTokens.status.info,
    gridCritical: uiTokens.status.critical,
    gridDot: "rgba(0, 0, 0, 0.018)"
  },
  typography: { display: uiTokens.typography.display, body: uiTokens.typography.body },
  spacing: uiTokens.spacing,
  borderWidths: uiTokens.borderWidths,
  motion: { fast: uiTokens.motion.fast, base: uiTokens.motion.base }
} as const;
