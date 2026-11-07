// One place to show an error dialog with a long, copyable message.
export const errors = $state<{ title: string; message: string } | { title: null; message: null }>({ title: null, message: null });

export function showError(title: string, e: unknown) {
  errors.title = title;
  errors.message = e instanceof Error ? e.message : String(e);
}
