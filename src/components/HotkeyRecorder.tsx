import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  claimRecording,
  releaseRecording,
  subscribeRecording,
} from "./hotkeyRecordingBus";

type Props = {
  bindingId: string;
  value: string;
  disabled?: boolean;
  onCommit: (chord: string) => void | Promise<void>;
  /** Surface validation / save errors outside the row (toast / footer). */
  onError?: (message: string | null) => void;
};

type RecordKeyPayload = {
  key: string;
  down?: boolean;
  alt?: boolean;
  ctrl?: boolean;
  shift?: boolean;
  win?: boolean;
};

const DOUBLE_TAP_MS = 480;

function modsFromFlags(p: {
  alt?: boolean;
  ctrl?: boolean;
  shift?: boolean;
  win?: boolean;
}): string[] {
  const parts: string[] = [];
  if (p.ctrl) parts.push("Ctrl");
  if (p.alt) parts.push("Alt");
  if (p.shift) parts.push("Shift");
  if (p.win) parts.push("Win");
  return parts;
}

function modsPrefix(ev: KeyboardEvent): string[] {
  return modsFromFlags({
    ctrl: ev.ctrlKey,
    alt: ev.altKey,
    shift: ev.shiftKey,
    win: ev.metaKey,
  });
}

function keyToken(ev: KeyboardEvent): string | null {
  const key = ev.key;
  if (!key || key === "Shift" || key === "Control" || key === "Alt" || key === "Meta") {
    return null;
  }
  if (key === " ") return "Space";
  if (key.length === 1) return key.toUpperCase();
  if (key.startsWith("Arrow")) return key.slice(5);
  if (key === "Escape") return "Esc";
  return key;
}

/**
 * Record a global hotkey. Modifiers pass through normally.
 * Space+mod is owned by Host (system menu / IME) and re-emitted as
 * `hotkey-record-key` so the recorder can capture single/double Space taps.
 */
export default function HotkeyRecorder({
  bindingId,
  value,
  disabled,
  onCommit,
  onError,
}: Props) {
  const [recording, setRecording] = useState(false);
  const [hint, setHint] = useState<string | null>(null);
  const pendingRef = useRef<{
    mods: string;
    token: string;
    at: number;
  } | null>(null);
  const timerRef = useRef<number | null>(null);
  const recordingRef = useRef(false);
  /** Dedupe Host Space emit vs rare browser / dual-shield duplicate. */
  const lastSpaceAtRef = useRef(0);
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;

  const reportError = useCallback((message: string | null) => {
    onErrorRef.current?.(message);
  }, []);

  const clearTimer = useCallback(() => {
    if (timerRef.current != null) {
      window.clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  }, []);

  const stopLocal = useCallback(
    (opts?: { resume?: boolean; release?: boolean }) => {
      const resume = opts?.resume !== false;
      const release = opts?.release !== false;
      clearTimer();
      pendingRef.current = null;
      setHint(null);
      recordingRef.current = false;
      setRecording(false);
      if (release) releaseRecording(bindingId);
      if (resume) {
        void invoke("resume_hotkeys_after_recording").catch(() => undefined);
      }
    },
    [bindingId, clearTimer],
  );

  const commitChord = useCallback(
    async (chord: string) => {
      try {
        const normalized = await invoke<string>("validate_hotkey_chord", {
          id: bindingId,
          chord,
        });
        reportError(null);
        stopLocal({ resume: true, release: true });
        await onCommit(normalized);
      } catch (err) {
        const msg =
          typeof err === "string"
            ? err
            : err instanceof Error
              ? err.message
              : String(err);
        reportError(msg);
        stopLocal({ resume: true, release: true });
      }
    },
    [bindingId, onCommit, stopLocal, reportError],
  );

  const handleCapture = useCallback(
    (token: string, mods: string[]) => {
      if (token === "Esc") {
        stopLocal({ resume: true, release: true });
        reportError(null);
        return;
      }
      if (token === "Backspace" || token === "Delete") {
        stopLocal({ resume: true, release: true });
        reportError(null);
        void onCommit("");
        return;
      }
      if (mods.length === 0) {
        reportError("需要至少一个修饰键（Ctrl / Alt / Shift / Win）");
        return;
      }
      const modsKey = mods.join("+");
      const now = Date.now();
      const pending = pendingRef.current;

      if (
        pending &&
        pending.mods === modsKey &&
        pending.token === token &&
        now - pending.at <= DOUBLE_TAP_MS
      ) {
        clearTimer();
        pendingRef.current = null;
        setHint(null);
        void commitChord(`${modsKey}+${token}+${token}`);
        return;
      }

      pendingRef.current = { mods: modsKey, token, at: now };
      setHint(`${modsKey}+${token}… 再按一次 ${token} 可录制连按`);
      clearTimer();
      timerRef.current = window.setTimeout(() => {
        const p = pendingRef.current;
        pendingRef.current = null;
        setHint(null);
        if (p) {
          void commitChord(`${p.mods}+${p.token}`);
        }
      }, DOUBLE_TAP_MS);
    },
    [clearTimer, commitChord, onCommit, stopLocal, reportError],
  );

  useEffect(() => {
    return subscribeRecording((activeId) => {
      if (!recordingRef.current) return;
      if (activeId !== bindingId) {
        stopLocal({ resume: false, release: false });
      }
    });
  }, [bindingId, stopLocal]);

  useEffect(() => {
    if (!recording) return;
    void invoke("suspend_hotkeys_for_recording").catch(() => undefined);
    return () => {
      clearTimer();
      if (recordingRef.current) {
        recordingRef.current = false;
        releaseRecording(bindingId);
        void invoke("resume_hotkeys_after_recording").catch(() => undefined);
      }
    };
  }, [recording, bindingId, clearTimer]);

  // Host: Space+mod stolen from system menu / IME → forward here.
  useEffect(() => {
    if (!recording) return;
    let un: (() => void) | undefined;
    void listen<RecordKeyPayload>("hotkey-record-key", (ev) => {
      const p = ev.payload;
      if (!p?.down || p.key !== "Space") return;
      const mods = modsFromFlags(p);
      if (mods.length === 0) return;
      const now = Date.now();
      if (now - lastSpaceAtRef.current < 40) return;
      lastSpaceAtRef.current = now;
      handleCapture("Space", mods);
    }).then((fn) => {
      un = fn;
    });
    return () => un?.();
  }, [recording, handleCapture]);

  // Normal capture in the webview (Ctrl/Shift/Alt+letter, Esc, …).
  useEffect(() => {
    if (!recording) return;
    const onKey = (ev: KeyboardEvent) => {
      ev.preventDefault();
      ev.stopPropagation();
      if (ev.repeat) return;
      // Alt/Ctrl+Space are stolen by Host RegisterHotKey — ignore browser copy.
      // Shift/Win+Space: keep browser as fallback if Host shield misses.
      if (
        (ev.altKey || ev.ctrlKey) &&
        (ev.key === " " || ev.code === "Space")
      ) {
        return;
      }
      const token = keyToken(ev);
      if (!token) return;
      if (token === "Space") {
        const now = Date.now();
        if (now - lastSpaceAtRef.current < 40) return;
        lastSpaceAtRef.current = now;
      }
      handleCapture(token, modsPrefix(ev));
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [recording, handleCapture]);

  return (
    <div className="hotkey-recorder">
      <button
        type="button"
        className={`hotkey-recorder-btn${recording ? " is-recording" : ""}`}
        disabled={disabled}
        onClick={() => {
          // Keep prior error visible until a new result replaces it.
          setHint(null);
          claimRecording(bindingId);
          recordingRef.current = true;
          setRecording(true);
        }}
      >
        {recording
          ? hint || "按下组合键…（Esc 取消）"
          : value?.trim()
            ? value
            : "未设置"}
      </button>
      {value?.trim() ? (
        <button
          type="button"
          className="hotkey-recorder-clear"
          disabled={disabled || recording}
          title="清除"
          onClick={() => {
            reportError(null);
            void onCommit("");
          }}
        >
          ✕
        </button>
      ) : null}
    </div>
  );
}
