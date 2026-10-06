/**
 * App colour themes (shared with Gravitation). Every id maps 1:1 to a
 * complete [data-theme] block in src/index.css.
 */
export type Theme =
  | "dark"
  | "light"
  | "slate"
  | "amoled"
  | "vibe"
  | "one-dark-pro"
  | "dracula"
  | "github-dark"
  | "github-light"
  | "tokyo-night"
  | "monokai-pro"
  | "solarized-dark"
  | "solarized-light"
  | "nord"
  | "gruvbox"
  | "catppuccin";

/** Metadata for the theme dropdown: display label, kind and preview swatches. */
export interface ThemeMeta {
  id: Theme;
  label: string;
  kind: "dark" | "light";
  /** [background, surface, accent] — rendered as dots in the picker. */
  swatch: [string, string, string];
}

export const THEME_LIST: ThemeMeta[] = [
  { id: "dark", label: "Dark", kind: "dark", swatch: ["#101010", "#1c1c1c", "#388BFD"] },
  { id: "light", label: "Light", kind: "light", swatch: ["#F8F9FA", "#FFFFFF", "#0969DA"] },
  { id: "slate", label: "Slate", kind: "dark", swatch: ["#0F172A", "#1E293B", "#0EA5E9"] },
  { id: "amoled", label: "AMOLED Black", kind: "dark", swatch: ["#000000", "#111111", "#3B82F6"] },
  { id: "one-dark-pro", label: "One Dark Pro", kind: "dark", swatch: ["#21252B", "#282C34", "#61AFEF"] },
  { id: "dracula", label: "Dracula", kind: "dark", swatch: ["#21222C", "#282A36", "#BD93F9"] },
  { id: "github-dark", label: "GitHub Dark", kind: "dark", swatch: ["#010409", "#0D1117", "#2F81F7"] },
  { id: "github-light", label: "GitHub Light", kind: "light", swatch: ["#F6F8FA", "#FFFFFF", "#0969DA"] },
  { id: "tokyo-night", label: "Tokyo Night", kind: "dark", swatch: ["#16161E", "#1A1B26", "#7AA2F7"] },
  { id: "monokai-pro", label: "Monokai Pro", kind: "dark", swatch: ["#19181A", "#2D2A2E", "#FFD866"] },
  { id: "solarized-dark", label: "Solarized Dark", kind: "dark", swatch: ["#00212B", "#002B36", "#268BD2"] },
  { id: "solarized-light", label: "Solarized Light", kind: "light", swatch: ["#EAE3CB", "#FDF6E3", "#268BD2"] },
  { id: "nord", label: "Nord", kind: "dark", swatch: ["#242933", "#2E3440", "#88C0D0"] },
  { id: "gruvbox", label: "Gruvbox Dark", kind: "dark", swatch: ["#1D2021", "#282828", "#FABD2F"] },
  { id: "catppuccin", label: "Catppuccin Mocha", kind: "dark", swatch: ["#11111B", "#1E1E2E", "#89B4FA"] },
  { id: "vibe", label: "Synthwave", kind: "dark", swatch: ["#12081F", "#221039", "#FF5AC5"] },
];

export const isTheme = (v: unknown): v is Theme => THEME_LIST.some((t) => t.id === v);

/** Applies a theme to the page (and the native window frame colour scheme). */
export function applyTheme(theme: Theme) {
  document.documentElement.setAttribute("data-theme", theme);
  document.documentElement.style.colorScheme = THEME_LIST.find((t) => t.id === theme)?.kind ?? "dark";
}
