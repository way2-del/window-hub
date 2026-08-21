/** Exclusive hotkey-recording bus — only one recorder green at a time. */

type Listener = (activeId: string | null) => void;

let activeId: string | null = null;
const listeners = new Set<Listener>();

export function getActiveRecordingId(): string | null {
  return activeId;
}

/** Start recording for `id`; notifies others to cancel (no resume). */
export function claimRecording(id: string): void {
  if (activeId === id) return;
  activeId = id;
  for (const l of [...listeners]) l(activeId);
}

/** End recording for `id` if it still owns the claim. */
export function releaseRecording(id: string): void {
  if (activeId !== id) return;
  activeId = null;
  for (const l of [...listeners]) l(null);
}

export function subscribeRecording(listener: Listener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
