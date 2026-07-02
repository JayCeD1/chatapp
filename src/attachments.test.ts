import { describe, it, expect } from "vitest";
import {
  formatBytes,
  mimeFromFilename,
  isPreviewableImage,
  shouldAutoFetch,
  AUTO_FETCH_IMAGE_MAX,
} from "./attachments";

describe("formatBytes", () => {
  it("scales bytes → KB → MB", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1024)).toBe("1.0 KB");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(1024 * 1024)).toBe("1.0 MB");
    expect(formatBytes(25 * 1024 * 1024)).toBe("25.0 MB");
  });
});

describe("mimeFromFilename", () => {
  it("maps known extensions case-insensitively", () => {
    expect(mimeFromFilename("photo.PNG")).toBe("image/png");
    expect(mimeFromFilename("clip.jpeg")).toBe("image/jpeg");
    expect(mimeFromFilename("report.pdf")).toBe("application/pdf");
    expect(mimeFromFilename("archive.zip")).toBe("application/zip");
  });

  it("falls back to octet-stream for unknown or missing extensions", () => {
    expect(mimeFromFilename("mystery.xyz")).toBe("application/octet-stream");
    expect(mimeFromFilename("noext")).toBe("application/octet-stream");
    expect(mimeFromFilename("")).toBe("application/octet-stream");
  });

  it("uses only the last extension of a multi-dot name", () => {
    expect(mimeFromFilename("archive.tar.gz")).toBe("application/gzip");
  });
});

describe("isPreviewableImage", () => {
  it("accepts common raster + svg image types", () => {
    for (const m of [
      "image/png",
      "image/jpeg",
      "image/gif",
      "image/webp",
      "image/svg+xml",
    ]) {
      expect(isPreviewableImage(m)).toBe(true);
    }
  });

  it("rejects non-image and unknown types", () => {
    expect(isPreviewableImage("application/pdf")).toBe(false);
    expect(isPreviewableImage("text/plain")).toBe(false);
    expect(isPreviewableImage("image/tiff")).toBe(false); // not in the preview set
    expect(isPreviewableImage("application/octet-stream")).toBe(false);
  });
});

describe("shouldAutoFetch (auto-fetch policy §5)", () => {
  it("auto-fetches small images only", () => {
    expect(shouldAutoFetch({ mime: "image/png", size: 1024 })).toBe(true);
    expect(shouldAutoFetch({ mime: "image/png", size: AUTO_FETCH_IMAGE_MAX })).toBe(
      true,
    );
  });

  it("does not auto-fetch large images", () => {
    expect(
      shouldAutoFetch({ mime: "image/jpeg", size: AUTO_FETCH_IMAGE_MAX + 1 }),
    ).toBe(false);
    expect(shouldAutoFetch({ mime: "image/jpeg", size: 5 * 1024 * 1024 })).toBe(
      false,
    );
  });

  it("never auto-fetches non-image files, regardless of size", () => {
    expect(shouldAutoFetch({ mime: "application/pdf", size: 100 })).toBe(false);
    expect(shouldAutoFetch({ mime: "text/plain", size: 10 })).toBe(false);
  });
});
