/** Bounded, cancellable batches. A late icon must not be lost to a throttle. */
export function createGlyphQueue(
  fetch: (ids: string[]) => Promise<Record<string, string>>,
  publish: (glyphs: Record<string, string>) => void,
  delay = 100,
) {
  let wanted: string[] = [];
  const attempts = new Map<string, number>();
  const loaded = new Set<string>();
  let active = false;
  let disposed = false;
  let timer: ReturnType<typeof setTimeout> | undefined;

  function schedule() {
    if (disposed || active || timer !== undefined) return;
    const eligible = wanted.filter(id => !loaded.has(id) && (attempts.get(id) ?? 0) < 3);
    if (!eligible.length) return;
    // New icons precede retries, so one unavailable glyph cannot starve the rest.
    eligible.sort((a, b) => (attempts.get(a) ?? 0) - (attempts.get(b) ?? 0));
    timer = setTimeout(() => {
      timer = undefined;
      void run();
    }, eligible.some(id => !attempts.has(id)) ? delay : Math.max(delay, 1500));
  }

  async function run() {
    const ids = wanted.filter(id => !loaded.has(id) && (attempts.get(id) ?? 0) < 3)
      .sort((a, b) => (attempts.get(a) ?? 0) - (attempts.get(b) ?? 0)).slice(0, 6);
    if (disposed || !ids.length) return;
    active = true;
    ids.forEach(id => attempts.set(id, (attempts.get(id) ?? 0) + 1));
    try {
      const result = await fetch(ids);
      if (!disposed) {
        const current: Record<string, string> = {};
        for (const id of ids) {
          if (result[id] && wanted.includes(id)) {
            loaded.add(id);
            current[id] = result[id];
          }
        }
        if (Object.keys(current).length) publish(current);
      }
    } catch { /* retry in a later batch, never stall other icons */ }
    finally { active = false; schedule(); }
  }

  return {
    update(ids: string[]) {
      wanted = [...new Set(ids)];
      clearTimeout(timer);
      timer = undefined;
      for (const id of attempts.keys()) if (!wanted.includes(id)) attempts.delete(id);
      for (const id of loaded) if (!wanted.includes(id)) loaded.delete(id);
      schedule();
    },
    dispose() { disposed = true; clearTimeout(timer); },
  };
}
