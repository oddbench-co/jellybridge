// Every page of the site, with the text that proves the right page rendered.
export const PAGES = [
  { path: "/", name: "home", text: "The native desktop client for Jellyfin." },
  { path: "/features", name: "features", text: "Built for a better Jellyfin experience." },
  { path: "/screenshots", name: "screenshots", text: "The familiar interface." },
  { path: "/download", name: "download", text: "Get JellyBridge." },
  { path: "/docs", name: "docs", text: "Learn and get started." },
  { path: "/docs/installation", name: "docs-installation", text: "Installation" },
  { path: "/docs/keyboard-shortcuts", name: "docs-keyboard-shortcuts", text: "Keyboard shortcuts" },
  { path: "/faq", name: "faq", text: "Frequently asked questions." },
  { path: "/privacy", name: "privacy", text: "Your data stays with you." },
  { path: "/about", name: "about", text: "Built by people who love open media." },
  { path: "/this-page-does-not-exist", name: "not-found", text: "This page does not exist." },
] as const;

export const DOCS = [
  "installation", "first-launch", "connecting", "basic-usage", "playback", "subtitles-and-audio",
  "keyboard-shortcuts", "troubleshooting", "settings", "command-line", "file-locations",
  "building-from-source", "contributing", "architecture",
];
