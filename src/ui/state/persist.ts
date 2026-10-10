/** Tiny localStorage helpers (wrapped: storage can be unavailable or full). */
export function loadJson<T>(key: string, fallback: T): T {
  try {
    const s = localStorage.getItem(key);
    if (!s) return fallback;
    const v: unknown = JSON.parse(s);
    // Objects are merged over the defaults (new settings keys); arrays are taken as stored.
    if (Array.isArray(fallback)) return (Array.isArray(v) ? v : fallback) as T;
    return { ...fallback, ...(v as object) };
  } catch {
    return fallback;
  }
}

export function saveJson(key: string, value: unknown): void {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* ignore */
  }
}
