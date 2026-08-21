/** Settings-window toast bus — host renders outside the scroll body. */

export type SettingsToastPayload = {
  id: number;
  text: string;
};

type Listener = (toast: SettingsToastPayload | null) => void;

const listeners = new Set<Listener>();
let seq = 0;

export function pushSettingsToast(message: string | null): void {
  const toast =
    message == null || message === ""
      ? null
      : { id: ++seq, text: message };
  for (const l of [...listeners]) l(toast);
}

export function subscribeSettingsToast(listener: Listener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
