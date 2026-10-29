export type Platform = 'windows' | 'macos' | 'linux';
export const platformNames: Record<Platform, string> = { windows: 'Windows', macos: 'macOS', linux: 'Linux' };
export function detectPlatform(userAgent: string, platform = ''): Platform | null {
  const source = `${platform} ${userAgent}`;
  if (/Android|iPhone|iPad|iPod/i.test(source)) return null;
  if (/Windows|Win32|Win64/i.test(source)) return 'windows';
  if (/Macintosh|Mac OS X|macOS/i.test(source)) return 'macos';
  if (/Linux|X11/i.test(source)) return 'linux';
  return null;
}
export function visitorPlatform(): Platform | null {
  const nav = navigator as Navigator & { userAgentData?: { platform?: string } };
  // iPadOS can identify as a Mac; it cannot run the desktop archive.
  if (/Mac/i.test(nav.userAgent) && nav.maxTouchPoints > 1) return null;
  return detectPlatform(nav.userAgent, nav.userAgentData?.platform);
}
