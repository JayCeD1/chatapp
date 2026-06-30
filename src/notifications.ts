import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";

let cached: boolean | null = null;

// Ask once (cached) and return whether we may post desktop notifications.
export async function ensureNotificationPermission(): Promise<boolean> {
  if (cached !== null) return cached;
  try {
    let ok = await isPermissionGranted();
    if (!ok) {
      ok = (await requestPermission()) === "granted";
    }
    cached = ok;
    return ok;
  } catch {
    cached = false;
    return false;
  }
}

export async function notify(title: string, body: string): Promise<void> {
  try {
    if (await ensureNotificationPermission()) {
      sendNotification({ title, body });
    }
  } catch {
    // Not in a Tauri context, or permission denied — ignore.
  }
}
