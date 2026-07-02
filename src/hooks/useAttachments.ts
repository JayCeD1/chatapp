import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { AttachmentRef } from "../types";
import {
  fetchAttachment,
  getAttachmentBytes,
  saveAttachment,
  AttachmentReady,
  AttachmentFailed,
  AttachmentProgress,
} from "../attachments";

// Per-attachment download/preview state, keyed by content address (sha256) so messages
// that share content share one entry. The composer's pre-send upload state is separate
// (local to the composer); this hook owns the read/preview side of already-sent messages.
export type AttachmentStatus = "idle" | "loading" | "ready" | "failed";

export interface AttachmentView {
  status: AttachmentStatus;
  url?: string; // object URL for images, once bytes are local
  progress?: number; // 0..1 while transferring
  error?: string;
}

const IDLE: AttachmentView = { status: "idle" };

/**
 * Owns the client-side download/preview lifecycle for attachments on rendered messages:
 * fetch orchestration, object-URL creation + revocation, and progress/failure tracking.
 * Listeners are registered once; state is keyed by sha256 (content-addressed, so a repost
 * of the same file reuses one download). Mounted at a stable level (Workspace) so an
 * in-flight fetch's completion event is never missed while a channel is open.
 */
export const useAttachments = () => {
  const [views, setViews] = useState<Record<string, AttachmentView>>({});

  // Object URLs to revoke on unmount (sha → url).
  const urlsRef = useRef<Record<string, string>>({});
  // shas we want an object URL built for on ready (image previews); a plain download
  // doesn't build one (avoids holding a 25 MiB blob URL for a file we only save to disk).
  const wantUrlRef = useRef<Set<string>>(new Set());
  // Pending `download()` awaiters, resolved/rejected by the ready/failed events.
  const waitersRef = useRef<
    Record<string, { resolve: () => void; reject: (e: Error) => void }[]>
  >({});

  const setView = useCallback((sha: string, patch: Partial<AttachmentView>) => {
    setViews((prev) => ({
      ...prev,
      [sha]: { ...(prev[sha] ?? IDLE), ...patch },
    }));
  }, []);

  const settleWaiters = useCallback(
    (sha: string, err?: string) => {
      const list = waitersRef.current[sha];
      if (!list) return;
      delete waitersRef.current[sha];
      for (const w of list) {
        if (err) w.reject(new Error(err));
        else w.resolve();
      }
    },
    [],
  );

  useEffect(() => {
    const unlisten: Array<Promise<() => void>> = [];

    unlisten.push(
      listen<AttachmentProgress>("attachment_progress", (e) => {
        // Only track download progress here; upload progress belongs to the composer.
        if (e.payload.direction !== "download") return;
        const { sha256, transferred, total } = e.payload;
        setView(sha256, {
          status: "loading",
          progress: total > 0 ? transferred / total : 0,
        });
      }),
    );

    unlisten.push(
      listen<AttachmentReady>("attachment_ready", async (e) => {
        const { sha256 } = e.payload;
        settleWaiters(sha256);
        if (!wantUrlRef.current.has(sha256)) {
          setView(sha256, { status: "ready", progress: 1 });
          return;
        }
        try {
          const blob = await getAttachmentBytes(sha256);
          // Guard against a double-ready building two URLs for one sha.
          if (urlsRef.current[sha256]) {
            setView(sha256, { status: "ready", progress: 1 });
            return;
          }
          const url = URL.createObjectURL(blob);
          urlsRef.current[sha256] = url;
          setView(sha256, { status: "ready", url, progress: 1 });
        } catch (err) {
          setView(sha256, { status: "failed", error: String(err) });
        }
      }),
    );

    unlisten.push(
      listen<AttachmentFailed>("attachment_failed", (e) => {
        const sha = e.payload.sha256 || "";
        if (sha) {
          wantUrlRef.current.delete(sha);
          setView(sha, { status: "failed", error: e.payload.reason });
          settleWaiters(sha, e.payload.reason);
        }
      }),
    );

    // On a dropped connection the backend clears in-flight download assemblies silently
    // (§9a trap 12), so fail any still-loading fetch here rather than leave a card spinning.
    // Already-ready previews keep their bytes (content-addressed, still valid).
    unlisten.push(
      listen("connection_lost", () => {
        wantUrlRef.current.clear();
        Object.keys(waitersRef.current).forEach((sha) =>
          settleWaiters(sha, "Connection lost"),
        );
        setViews((prev) => {
          const next = { ...prev };
          for (const [sha, v] of Object.entries(next)) {
            if (v.status === "loading")
              next[sha] = {
                status: "failed",
                error: "Connection lost — retry",
              };
          }
          return next;
        });
      }),
    );

    return () => {
      unlisten.forEach((p) => p.then((u) => u()));
      Object.values(urlsRef.current).forEach((u) => URL.revokeObjectURL(u));
      urlsRef.current = {};
    };
  }, [setView, settleWaiters]);

  const viewFor = useCallback(
    (sha: string): AttachmentView => views[sha] ?? IDLE,
    [views],
  );

  // Kick off a fetch + build a preview URL when it lands (idempotent; a no-op if the
  // content is already loading or ready).
  const load = useCallback(
    (ref: AttachmentRef) => {
      const cur = views[ref.sha256]?.status;
      if (cur === "loading" || cur === "ready") return;
      wantUrlRef.current.add(ref.sha256);
      setView(ref.sha256, { status: "loading", error: undefined });
      fetchAttachment(ref.id, ref.sha256).catch((err) =>
        setView(ref.sha256, { status: "failed", error: String(err) }),
      );
    },
    [views, setView],
  );

  // Ensure the bytes are local (fetching + awaiting the ready event if needed), then open
  // the OS save dialog. Used by file cards and the "save image" action.
  const download = useCallback(
    async (ref: AttachmentRef) => {
      const ready = views[ref.sha256]?.status === "ready";
      if (!ready) {
        await new Promise<void>((resolve, reject) => {
          (waitersRef.current[ref.sha256] ??= []).push({ resolve, reject });
          setView(ref.sha256, { status: "loading", error: undefined });
          fetchAttachment(ref.id, ref.sha256).catch((err) => {
            setView(ref.sha256, { status: "failed", error: String(err) });
            settleWaiters(ref.sha256, String(err));
          });
        });
      }
      await saveAttachment(ref.sha256, ref.name);
    },
    [views, setView, settleWaiters],
  );

  return { viewFor, load, download };
};

export type UseAttachments = ReturnType<typeof useAttachments>;
