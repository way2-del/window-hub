/** Island notification bus — host-owned banner queue for tray + plugins. */

export type NotifyUrgency = "passive" | "active" | "critical";

export type IslandNotifyRequest = {
  /** Origin: tray system or plugin id */
  source: "tray" | "plugin";
  pluginId?: string;
  title: string;
  body?: string;
  /** PNG base64 without data: prefix */
  iconPng?: string;
  urgency?: NotifyUrgency;
  /** Auto-dismiss ms; 0 = sticky until user dismisses */
  ttlMs?: number;
  /** Tray invoke payload (optional) */
  tray?: {
    /** Tray icon id — preferred coalesce key */
    iconId?: string;
    hwnd: number;
    uid: number;
    callbackMsg: number;
    version: number;
  };
  actions?: { id: string; label: string }[];
};

export type IslandNotifyBanner = IslandNotifyRequest & {
  id: string;
  createdAt: number;
};

type Listener = (banner: IslandNotifyBanner | null) => void;

const URGENCY_RANK: Record<NotifyUrgency, number> = {
  passive: 0,
  active: 1,
  critical: 2,
};

/** Same tray icon / plugin → one banner; later events refresh instead of stacking. */
function coalesceKey(req: IslandNotifyRequest): string | null {
  if (req.source === "tray" && req.tray) {
    if (req.tray.iconId) return `tray:${req.tray.iconId}`;
    return `tray:${req.tray.hwnd}:${req.tray.uid}`;
  }
  if (req.source === "plugin" && req.pluginId) {
    return `plugin:${req.pluginId}`;
  }
  return null;
}

class IslandNotifyBus {
  private current: IslandNotifyBanner | null = null;
  private queue: IslandNotifyBanner[] = [];
  private listeners = new Set<Listener>();
  private ttlTimer: ReturnType<typeof setTimeout> | null = null;
  private rate = new Map<string, number[]>();

  subscribe(fn: Listener): () => void {
    this.listeners.add(fn);
    fn(this.current);
    return () => this.listeners.delete(fn);
  }

  getCurrent(): IslandNotifyBanner | null {
    return this.current;
  }

  /** Rate-limit plugin notifies (default 6/min). */
  private allowRate(pluginId: string, maxPerMinute = 6): boolean {
    const now = Date.now();
    const windowMs = 60_000;
    const prev = (this.rate.get(pluginId) ?? []).filter((t) => now - t < windowMs);
    if (prev.length >= maxPerMinute) {
      this.rate.set(pluginId, prev);
      return false;
    }
    prev.push(now);
    this.rate.set(pluginId, prev);
    return true;
  }

  push(req: IslandNotifyRequest, maxPerMinute?: number): IslandNotifyBanner | null {
    if (req.source === "plugin" && req.pluginId) {
      if (!this.allowRate(req.pluginId, maxPerMinute ?? 6)) return null;
    }

    const urgency = req.urgency ?? "active";
    const key = coalesceKey(req);

    // Already showing same source → refresh payload, keep id (no remount / no re-queue)
    if (this.current && key && coalesceKey(this.current) === key) {
      this.current = {
        ...this.current,
        ...req,
        urgency,
        id: this.current.id,
        createdAt: this.current.createdAt,
      };
      this.show(this.current);
      return this.current;
    }

    // Same source already queued → replace that slot
    if (key) {
      const qi = this.queue.findIndex((b) => coalesceKey(b) === key);
      if (qi >= 0) {
        const prev = this.queue[qi];
        const banner: IslandNotifyBanner = {
          ...req,
          urgency,
          id: prev.id,
          createdAt: Date.now(),
        };
        this.queue[qi] = banner;
        return banner;
      }
    }

    const banner: IslandNotifyBanner = {
      ...req,
      urgency,
      id: `n-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
      createdAt: Date.now(),
    };

    if (!this.current) {
      this.show(banner);
      return banner;
    }

    const curRank = URGENCY_RANK[this.current.urgency ?? "active"];
    const nextRank = URGENCY_RANK[banner.urgency ?? "active"];
    if (nextRank > curRank || banner.urgency === "critical") {
      this.queue.unshift(this.current);
      // After demoting current, drop any queue dup of the incoming key (safety)
      if (key) {
        this.queue = this.queue.filter((b) => coalesceKey(b) !== key);
      }
      this.show(banner);
    } else {
      this.queue.push(banner);
    }
    return banner;
  }

  dismiss(id?: string) {
    if (id && this.current && this.current.id !== id) {
      this.queue = this.queue.filter((b) => b.id !== id);
      return;
    }
    this.clearTtl();
    this.current = null;
    this.emit();
    this.dequeue();
  }

  private show(banner: IslandNotifyBanner) {
    this.clearTtl();
    this.current = banner;
    this.emit();
    const ttl = banner.ttlMs ?? 0;
    if (ttl > 0) {
      this.ttlTimer = setTimeout(() => {
        this.ttlTimer = null;
        if (this.current?.id === banner.id) this.dismiss(banner.id);
      }, ttl);
    }
  }

  private dequeue() {
    const next = this.queue.shift();
    if (next) this.show(next);
  }

  private clearTtl() {
    if (this.ttlTimer) {
      clearTimeout(this.ttlTimer);
      this.ttlTimer = null;
    }
  }

  private emit() {
    for (const l of this.listeners) l(this.current);
  }
}

export const islandNotifyBus = new IslandNotifyBus();
