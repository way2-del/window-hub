import { useEffect, useRef, useState } from "react";

type Props = {
  /** 面板打开时开启摄像头，关闭时释放 */
  active: boolean;
};

export default function MirrorPreview({ active }: Props) {
  const videoRef = useRef<HTMLVideoElement>(null);
  const streamRef = useRef<MediaStream | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [ready, setReady] = useState(false);

  useEffect(() => {
    let cancelled = false;

    async function start() {
      setError(null);
      setReady(false);
      try {
        const stream = await navigator.mediaDevices.getUserMedia({
          audio: false,
          video: {
            facingMode: "user",
            width: { ideal: 1280 },
            height: { ideal: 720 },
          },
        });
        if (cancelled) {
          stream.getTracks().forEach((t) => t.stop());
          return;
        }
        streamRef.current = stream;
        const el = videoRef.current;
        if (el) {
          el.srcObject = stream;
          await el.play().catch(() => undefined);
        }
        if (!cancelled) setReady(true);
      } catch (e) {
        if (!cancelled) {
          setError(e instanceof Error ? e.message : "无法打开摄像头");
          setReady(false);
        }
      }
    }

    function stop() {
      const el = videoRef.current;
      if (el) {
        el.srcObject = null;
      }
      streamRef.current?.getTracks().forEach((t) => t.stop());
      streamRef.current = null;
      setReady(false);
    }

    if (active) void start();
    else stop();

    return () => {
      cancelled = true;
      stop();
    };
  }, [active]);

  return (
    <div className={`mirror-panel${ready ? " is-ready" : ""}`}>
      <div className="mirror-frame">
        <video
          ref={videoRef}
          className="mirror-video"
          playsInline
          muted
          autoPlay
          aria-label="镜子预览"
        />
        {!active ? (
          <div className="mirror-status">展开后加载摄像头</div>
        ) : !ready && !error ? (
          <div className="mirror-status">正在打开摄像头…</div>
        ) : null}
        {error ? (
          <div className="mirror-status mirror-error">
            摄像头不可用
            <span>{error}</span>
          </div>
        ) : null}
      </div>
    </div>
  );
}
