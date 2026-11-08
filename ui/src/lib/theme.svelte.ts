// Theme: follow the system (no saved choice), or light/dark chosen by the user.
// Applied as `data-theme` on <html> (app.css) and remembered per browser. The
// first paint is set by a small script in index.html, so there is no flash.
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

const query = typeof matchMedia === 'function' ? matchMedia('(prefers-color-scheme: dark)') : null;

export const theme = $state<{ value: Theme; systemDark: boolean }>({
  value: stored(),
  systemDark: query?.matches ?? false,
});
query?.addEventListener('change', (e) => (theme.systemDark = e.matches));

/** The theme the user sees now. */
export const isDark = () => (theme.value === 'system' ? theme.systemDark : theme.value === 'dark');

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

/** Flip what is visible, so a click always changes something. */
export const toggle = () => apply(isDark() ? 'light' : 'dark');
/** Back to following the system. */
export const followSystem = () => apply('system');
