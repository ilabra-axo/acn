export function displayName(c: { name: string | null; npub: string }): string {
  return c.name?.trim() || shortNpub(c.npub);
}

export function shortNpub(npub: string): string {
  return npub.length > 20 ? `${npub.slice(0, 10)}…${npub.slice(-6)}` : npub;
}

export function time(ts: number | null): string {
  if (!ts) return "";
  const d = new Date(ts * 1000);
  const now = new Date();
  if (d.toDateString() === now.toDateString()) {
    return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }
  const diffDays = (now.getTime() - d.getTime()) / 86_400_000;
  if (diffDays < 6) return d.toLocaleDateString([], { weekday: "short" });
  return d.toLocaleDateString([], { month: "short", day: "numeric" });
}

/** Stable pleasant hue from a hex pubkey. */
export function hue(pubkey: string): number {
  let h = 0;
  for (let i = 0; i < 12; i++) h = (h * 31 + pubkey.charCodeAt(i)) % 360;
  return h;
}

export function initials(name: string): string {
  const parts = name.replace(/^npub1/, "").split(/\s+/).filter(Boolean);
  if (parts.length >= 2) return (parts[0][0] + parts[1][0]).toUpperCase();
  return (parts[0] ?? "?").slice(0, 2).toUpperCase();
}

export const HOSTING_LABEL: Record<string, string> = {
  self_hosted: "Self-hosted",
  operator: "Run by this nmesh operator",
  third_party: "Third-party",
};
