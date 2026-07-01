import { useState, useEffect, useRef, useCallback } from "react";
import { loadPreferences, savePreferences, Preferences } from "../preferences";

/// Persisted app preferences (notification level, send-on-Enter). Split out of the connection
/// hook: `preferencesRef` lets the once-registered message ingest read the live notification
/// level without re-subscribing; the state updater stays pure and persistence happens in an
/// effect (mirrors useTheme).
export function usePreferences() {
  const [preferences, setPreferencesState] = useState<Preferences>(() =>
    loadPreferences(),
  );
  const preferencesRef = useRef(preferences);
  useEffect(() => {
    preferencesRef.current = preferences;
    savePreferences(preferences);
  }, [preferences]);
  const setPreferences = useCallback((patch: Partial<Preferences>) => {
    setPreferencesState((prev) => ({ ...prev, ...patch }));
  }, []);
  return { preferences, setPreferences, preferencesRef };
}
