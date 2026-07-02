// Typed wrappers for the attachment Tauri commands (docs/architecture/attachments.md §9
// task 4.1). The flow: upload the blob first (uploadAttachment → sha), then send the
// message carrying AttachmentRefs (sendMessageWithAttachments). Recipients render from
// metadata and pull bytes on demand (fetchAttachment → `attachment_ready`/
// `attachment_failed` events keyed by sha → getAttachmentBytes for an object URL).

import { invoke } from "@tauri-apps/api/core";
import { AttachmentRef } from "./types";

/** Result of uploading one file: the store-assigned content address + display metadata. */
export interface UploadedAttachment {
  sha256: string;
  size: number;
  name: string;
}

/** Payload of the `attachment_progress` event (both directions, bytes so far). */
export interface AttachmentProgress {
  sha256: string;
  direction: "upload" | "download";
  transferred: number;
  total: number;
}

/** Payload of the `attachment_ready` / `attachment_failed` events. */
export interface AttachmentReady {
  attachment_id: string;
  sha256: string;
}
export interface AttachmentFailed {
  attachment_id?: string | null;
  sha256?: string | null;
  reason: string;
}

/** Upload a file by path (attach dialog / native drag-drop both yield paths). */
export const uploadAttachment = (path: string): Promise<UploadedAttachment> =>
  invoke<UploadedAttachment>("upload_attachment", { path });

/** Send a Chat carrying attachment refs (blobs must be uploaded first). */
export const sendMessageWithAttachments = (
  message: string,
  userId: number,
  attachments: AttachmentRef[],
): Promise<void> =>
  invoke("send_message_with_attachments", {
    message,
    user_id: userId,
    is_emoji: false,
    attachments,
  });

/** Make an attachment's bytes available locally; resolves before the bytes arrive —
 *  completion is signaled by `attachment_ready`/`attachment_failed` (keyed by sha). */
export const fetchAttachment = (
  attachmentId: string,
  sha256: string,
): Promise<void> =>
  invoke("fetch_attachment", { attachment_id: attachmentId, sha256 });

/** Read an available attachment's raw bytes (host store / client cache). */
export const getAttachmentBytes = async (sha256: string): Promise<Blob> => {
  const data = await invoke<ArrayBuffer>("get_attachment_bytes", { sha256 });
  return new Blob([data]);
};

/** Save an attachment to disk via the OS save dialog (the original filename is only the
 *  dialog's suggestion). Resolves false when the user cancels. */
export const saveAttachment = (
  sha256: string,
  suggestedName: string,
): Promise<boolean> =>
  invoke<boolean>("save_attachment", {
    sha256,
    suggested_name: suggestedName,
  });

/** Attachments render as inline image previews only for these types. */
export const isPreviewableImage = (mime: string): boolean =>
  /^image\/(png|jpeg|gif|webp|avif|bmp|svg\+xml)$/.test(mime);

/** Images at or below this size auto-fetch on render; larger ones (and all non-image
 *  files) load on an explicit click, so opening a channel never pulls megabytes eagerly. */
export const AUTO_FETCH_IMAGE_MAX = 512 * 1024;

/** The auto-fetch policy (design §5): small images only. */
export const shouldAutoFetch = (ref: {
  mime: string;
  size: number;
}): boolean => isPreviewableImage(ref.mime) && ref.size <= AUTO_FETCH_IMAGE_MAX;

/** Max attachments per message (mirrors the backend MAX_ATTACHMENTS_PER_MESSAGE). */
export const MAX_ATTACHMENTS_PER_MESSAGE = 5;

/** Max attachment size in bytes (mirrors the backend MAX_ATTACHMENT_BYTES = 25 MiB). */
export const MAX_ATTACHMENT_BYTES = 25 * 1024 * 1024;

const MIME_BY_EXTENSION: Record<string, string> = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
  avif: "image/avif",
  bmp: "image/bmp",
  svg: "image/svg+xml",
  pdf: "application/pdf",
  txt: "text/plain",
  md: "text/markdown",
  csv: "text/csv",
  json: "application/json",
  zip: "application/zip",
  gz: "application/gzip",
  mp3: "audio/mpeg",
  wav: "audio/wav",
  mp4: "video/mp4",
  mov: "video/quicktime",
  webm: "video/webm",
  doc: "application/msword",
  docx: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
  xls: "application/vnd.ms-excel",
  xlsx: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
  ppt: "application/vnd.ms-powerpoint",
  pptx: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
};

/** Best-effort mime from a filename's extension (display metadata only — the host never
 *  trusts or inspects it). */
export const mimeFromFilename = (name: string): string => {
  const ext = name.split(".").pop()?.toLowerCase() ?? "";
  return MIME_BY_EXTENSION[ext] ?? "application/octet-stream";
};

/** Human-readable size for attachment cards. */
export const formatBytes = (bytes: number): string => {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
};
