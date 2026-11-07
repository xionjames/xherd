// Theme choice: system (prefers-color-scheme), light or dark. Applied as
// `data-theme` on <html> (app.css); remembered per browser.
export type Theme = 'system' | 'light' | 'dark';
const KEY = 'xherd.theme';

function stored(): Theme {
  try {
    const t = localStorage.getItem(KEY);
    return t === 'light' || t === 'dark' ? t : 'system';
  } catch {
    return 'system';
  }
}

export const theme = $state<{ value: Theme }>({ value: stored() });

export function apply(t: Theme) {
  theme.value = t;
  if (t === 'system') delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = t;
  try {
    if (t === 'system') localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, t);
  } catch {
    // Storage blocked: the choice lasts until the page reloads.
  }
}

const NEXT: Record<Theme, Theme> = { system: 'light', light: 'dark', dark: 'system' };
export const cycle = () => apply(NEXT[theme.value]);
