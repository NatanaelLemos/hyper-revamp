export function isSafeExternalUrl(url: string): boolean {
  if (typeof url !== 'string' || !url) {
    return false;
  }

  try {
    const parsedUrl = new URL(url);
    return parsedUrl.protocol === 'http:' || parsedUrl.protocol === 'https:';
  } catch {
    return false;
  }
}
