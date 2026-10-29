let toastTimer: ReturnType<typeof setTimeout>;
export function toast(message: string) {
  const element = document.querySelector('#toast');
  if (!element) return;
  element.textContent = message;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { element.textContent = ''; }, 4000);
}
export async function copyText(text: string, success = 'Copied') {
  try { await navigator.clipboard.writeText(text); toast(success); }
  catch { toast('Clipboard unavailable. Select and copy the text manually.'); }
}
